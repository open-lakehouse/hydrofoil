//! ConnectRPC `IngestService` — turn a local file into a Unity Catalog managed
//! Delta table, for web/UI clients (notably the Tauri desktop host).
//!
//! Two methods (see the proto for the wire contract):
//!
//!  - `PreviewFile` (unary): parse a local Parquet file with the engine's Arrow
//!    reader and return its inferred schema + a capped sample, both as Arrow IPC,
//!    so the UI previews rows and lets the user adjust the schema before
//!    committing. Host-local (needs a filesystem path).
//!
//!  - `IngestTable` (client-streaming): create the managed table (if absent) from
//!    the user-confirmed Arrow schema, then append the data. The first frame
//!    carries the target + schema; data is supplied either as Arrow IPC chunks on
//!    later frames (the portable path) or — on desktop — read by the host from a
//!    `source_path`.
//!
//! Both reuse the same engine wiring the query surface does: the principal is
//! parsed from request headers ([`crate::identity::principal_from_http_headers`]),
//! a per-user UC token selects that user's UC factory, and the managed write goes
//! through the shared committer tail
//! ([`crate::server::FlightSqlServiceImpl::append_managed_batches`]) so it faces
//! the identical Cedar authorization a regular `INSERT INTO … Append` would.

use std::sync::Arc;

use arrow::array::RecordBatch;
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::ipc::reader::StreamReader;
use connectrpc::{
    ConnectError, RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream,
    StreamMessage,
};
use datafusion::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use datafusion_open_lineage::context::LineageContext;
use futures::StreamExt;
use uuid::Uuid;

use crate::ingest_proto::v1::{
    IngestTableRequest, IngestTableResponse, PreviewFileRequest, PreviewFileResponse,
};
use crate::ingest_services::v1::IngestService;
use crate::query::encode_ipc_stream;
use crate::server::FlightSqlServiceImpl;

/// Default and maximum rows returned by `PreviewFile` when the request omits /
/// over-asks for a sample size. The preview is for eyeballing shape + types, not
/// for loading the file, so keep it small.
const PREVIEW_DEFAULT_ROWS: u32 = 100;
const PREVIEW_MAX_ROWS: u32 = 1_000;

/// Handler state for the ConnectRPC ingest surface: the shared engine + session
/// store ([`FlightSqlServiceImpl`]). Mirrors [`crate::query_service::QueryAppState`];
/// both surfaces share one `Arc<FlightSqlServiceImpl>`.
#[derive(Clone)]
pub struct IngestAppState {
    pub service: Arc<FlightSqlServiceImpl>,
}

impl IngestAppState {
    /// Register the IngestService on a ConnectRPC router (thin wrapper over the
    /// generated `IngestServiceExt::register`, matching `QueryAppState::register`).
    pub fn register(self, router: connectrpc::Router) -> connectrpc::Router {
        use crate::ingest_services::v1::IngestServiceExt;
        Arc::new(self).register(router)
    }
}

impl IngestService for IngestAppState {
    async fn preview_file(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, PreviewFileRequest>,
    ) -> ServiceResult<PreviewFileResponse> {
        let path = request.path.to_owned();
        let sample_rows = request
            .sample_rows
            .unwrap_or(PREVIEW_DEFAULT_ROWS)
            .min(PREVIEW_MAX_ROWS) as usize;
        if path.trim().is_empty() {
            return Err(ConnectError::invalid_argument("path is required"));
        }

        // Parsing is blocking file I/O + CPU; run it off the async executor.
        let (schema, sample, total_rows) =
            tokio::task::spawn_blocking(move || read_parquet_preview(&path, sample_rows))
                .await
                .map_err(|e| ConnectError::internal(format!("preview task panicked: {e}")))?
                .map_err(ConnectError::invalid_argument)?;

        let schema_ipc = encode_ipc_stream(&schema, &[]).map_err(ConnectError::internal)?;
        let sample_ipc = encode_ipc_stream(&schema, &sample).map_err(ConnectError::internal)?;

        Response::ok(PreviewFileResponse {
            schema_ipc,
            sample_ipc,
            total_rows_estimate: total_rows,
            ..Default::default()
        })
    }

    async fn ingest_table(
        &self,
        ctx: RequestContext,
        mut requests: ServiceStream<StreamMessage<IngestTableRequest>>,
    ) -> ServiceResult<IngestTableResponse> {
        // The first frame carries the target + schema (and, on desktop, the source
        // path). Read it up front; later frames carry only `arrow_ipc` data.
        let first = requests
            .next()
            .await
            .ok_or_else(|| ConnectError::invalid_argument("ingest stream was empty"))??
            .to_owned_message();

        if first.catalog.is_empty() || first.schema.is_empty() || first.table.is_empty() {
            return Err(ConnectError::invalid_argument(
                "first frame must set catalog, schema, and table",
            ));
        }
        let catalog = first.catalog.clone();
        let schema_name = first.schema.clone();
        let table = first.table.clone();
        let qualified = format!("{catalog}.{schema_name}.{table}");

        // The user-confirmed target schema, used to CREATE the table and to coerce
        // incoming batches. Required on the first frame.
        let target_schema = decode_ipc_schema(&first.target_schema_ipc)
            .map_err(|e| ConnectError::invalid_argument(format!("target_schema_ipc: {e}")))?;

        // Resolve the principal + session through the same engine path the query
        // surface uses (UC factory selected by the per-user token).
        let headers = ctx.headers().clone();
        let principal = crate::identity::principal_from_http_headers(&headers)
            .map_err(|e| ConnectError::invalid_argument(e.message))?;
        let uc_token = crate::identity::uc_token_from_http_headers(&headers);
        let session = self
            .service
            .session_for_principal(principal, uc_token.as_deref())
            .await
            .map_err(|s| ConnectError::internal(s.message().to_string()))?;
        let lineage = LineageContext {
            run_id: Some(Uuid::now_v7()),
            job_name: Some(crate::lineage::job_name_from_sql(&qualified)),
            sql: Some(format!("INGEST INTO {qualified}")),
            ..Default::default()
        };
        let lh = Arc::new(session.ctx());

        // Create the managed table when it is missing and the caller asked for it.
        // A CREATE that loses the race with an existing table is fine — we resolve
        // the (now-present) target below regardless.
        let mut created = false;
        let mut target =
            FlightSqlServiceImpl::resolve_managed_target(&lh, &catalog, &schema_name, &table)
                .await
                .map_err(|e| ConnectError::internal(e.to_string()))?;
        if target.is_none() {
            if !first.create_if_missing {
                return Err(ConnectError::invalid_argument(format!(
                    "table {qualified} is not a Unity Catalog managed table and create_if_missing is false"
                )));
            }
            self.create_managed_table(&session, &lineage, &qualified, &target_schema)
                .await?;
            created = true;
            target =
                FlightSqlServiceImpl::resolve_managed_target(&lh, &catalog, &schema_name, &table)
                    .await
                    .map_err(|e| ConnectError::internal(e.to_string()))?;
        }
        let target = target.ok_or_else(|| {
            ConnectError::internal(format!(
                "managed table {qualified} could not be resolved after create"
            ))
        })?;

        // Collect the batches to append. Desktop reads the local file by path; the
        // portable path decodes the streamed `arrow_ipc` frames. The first frame
        // may itself carry an `arrow_ipc` chunk.
        let batches =
            match first.source_path.as_deref().filter(|p| !p.is_empty()) {
                Some(source_path) => {
                    let source_path = source_path.to_owned();
                    tokio::task::spawn_blocking(move || read_parquet_batches(&source_path))
                        .await
                        .map_err(|e| ConnectError::internal(format!("read task panicked: {e}")))?
                        .map_err(ConnectError::invalid_argument)?
                }
                None => {
                    let mut batches = Vec::new();
                    if !first.arrow_ipc.is_empty() {
                        batches.extend(decode_ipc_batches(&first.arrow_ipc).map_err(|e| {
                            ConnectError::invalid_argument(format!("arrow_ipc: {e}"))
                        })?);
                    }
                    while let Some(item) = requests.next().await {
                        let frame = item?.to_owned_message();
                        if frame.arrow_ipc.is_empty() {
                            continue;
                        }
                        batches.extend(decode_ipc_batches(&frame.arrow_ipc).map_err(|e| {
                            ConnectError::invalid_argument(format!("arrow_ipc: {e}"))
                        })?);
                    }
                    batches
                }
            };

        let rows = self
            .service
            .append_managed_batches(&lh, &target, batches, &lineage)
            .await
            .map_err(|e| ConnectError::internal(e.to_string()))?;

        Response::ok(IngestTableResponse {
            rows_written: rows as u64,
            qualified_name: qualified,
            created,
            ..Default::default()
        })
    }
}

impl IngestAppState {
    /// Create the managed Delta table via the engine's SQL `CREATE TABLE … USING
    /// DELTA` path, which lowers to a Unity Catalog DDL extension node, registers
    /// the catalog on the session, and runs the Cedar gate at execution time
    /// (matching how managed DDL is authorized elsewhere). An empty `LOCATION`
    /// makes it managed (UC allocates the storage root).
    async fn create_managed_table(
        &self,
        session: &crate::engine::Session,
        lineage: &LineageContext,
        qualified: &str,
        target_schema: &SchemaRef,
    ) -> Result<(), ConnectError> {
        let columns = ddl_columns(target_schema)
            .map_err(|e| ConnectError::invalid_argument(format!("unsupported schema: {e}")))?;
        let ddl = format!("CREATE TABLE {qualified} ({columns}) USING DELTA");

        let lh = session.lakehouse_for_query(lineage.clone(), None);
        let plan = self
            .service
            .executor()
            .create_logical_plan(lh, ddl)
            .await
            .map_err(|e| ConnectError::internal(format!("error planning CREATE: {e}")))?;

        // Execute through the LakehouseCtx so the Cedar gate authorizes the DDL
        // (it rides as an Extension node) and the UC client runs it; collect the
        // result stream so the statement actually executes.
        let ctx = session.ctx();
        let df = ctx
            .execute_logical_plan(plan)
            .await
            .map_err(|e| ConnectError::internal(format!("error creating table: {e}")))?;
        df.collect()
            .await
            .map_err(|e| ConnectError::internal(format!("error creating table: {e}")))?;
        Ok(())
    }
}

/// Parse a Parquet file's schema + a capped sample of its rows. Returns the
/// inferred Arrow schema, up to `sample_rows` rows of sample batches, and the
/// file's total row count (from Parquet metadata).
fn read_parquet_preview(
    path: &str,
    sample_rows: usize,
) -> Result<(Schema, Vec<RecordBatch>, u64), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| format!("read {path}: {e}"))?;
    let schema = builder.schema().as_ref().clone();
    let total_rows = builder.metadata().file_metadata().num_rows().max(0) as u64;

    let reader = builder
        .with_limit(sample_rows)
        .with_batch_size(sample_rows.max(1))
        .build()
        .map_err(|e| format!("read {path}: {e}"))?;
    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch.map_err(|e| format!("read {path}: {e}"))?);
    }
    Ok((schema, batches, total_rows))
}

/// Read every record batch from a Parquet file (used by the desktop ingest path,
/// where the host has the file locally).
fn read_parquet_batches(path: &str) -> Result<Vec<RecordBatch>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| format!("read {path}: {e}"))?
        .build()
        .map_err(|e| format!("read {path}: {e}"))?;
    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch.map_err(|e| format!("read {path}: {e}"))?);
    }
    Ok(batches)
}

/// Decode an Arrow IPC stream's schema (schema message + EOS, no batches).
fn decode_ipc_schema(bytes: &[u8]) -> Result<SchemaRef, String> {
    let reader = StreamReader::try_new(std::io::Cursor::new(bytes), None)
        .map_err(|e| format!("arrow ipc: {e}"))?;
    Ok(reader.schema())
}

/// Decode an Arrow IPC stream into its record batches.
fn decode_ipc_batches(bytes: &[u8]) -> Result<Vec<RecordBatch>, String> {
    let reader = StreamReader::try_new(std::io::Cursor::new(bytes), None)
        .map_err(|e| format!("arrow ipc: {e}"))?;
    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch.map_err(|e| format!("arrow ipc: {e}"))?);
    }
    Ok(batches)
}

/// Render an Arrow schema as the column list of a `CREATE TABLE (...)` clause,
/// mapping each field to its SQL type. Only the common Parquet→Arrow types are
/// supported in phase 1; an unsupported type is a clear error rather than a
/// silently-wrong column.
fn ddl_columns(schema: &Schema) -> Result<String, String> {
    let cols = schema
        .fields()
        .iter()
        .map(|f| {
            let ty = sql_type(f)?;
            let null = if f.is_nullable() { "" } else { " NOT NULL" };
            // Quote the column name so reserved words / mixed case survive.
            Ok(format!("`{}` {ty}{null}", f.name()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(cols.join(", "))
}

/// Map an Arrow field's type to the SQL type used in the CREATE DDL.
fn sql_type(field: &Field) -> Result<&'static str, String> {
    Ok(match field.data_type() {
        DataType::Boolean => "BOOLEAN",
        DataType::Int8 | DataType::Int16 | DataType::Int32 => "INT",
        DataType::Int64 => "BIGINT",
        DataType::Float32 => "FLOAT",
        DataType::Float64 => "DOUBLE",
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => "STRING",
        DataType::Date32 | DataType::Date64 => "DATE",
        DataType::Timestamp(_, _) => "TIMESTAMP",
        other => {
            return Err(format!(
                "column '{}' has unsupported type {other:?}",
                field.name()
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Int64Array, StringArray};
    use datafusion::parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    /// Write a small Parquet file to a temp path and return it. The schema mixes a
    /// non-null Int64 + a nullable Utf8 so we exercise both nullability + types.
    fn write_fixture(rows: usize) -> tempfile::NamedTempFile {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));
        let ids: Vec<i64> = (0..rows as i64).collect();
        let names: Vec<Option<String>> = (0..rows).map(|i| Some(format!("n{i}"))).collect();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(names)),
            ],
        )
        .unwrap();

        let file = tempfile::Builder::new()
            .suffix(".parquet")
            .tempfile()
            .unwrap();
        let mut writer = ArrowWriter::try_new(file.reopen().unwrap(), schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        file
    }

    #[test]
    fn preview_reads_schema_sample_and_count() {
        let file = write_fixture(50);
        let (schema, batches, total) =
            read_parquet_preview(file.path().to_str().unwrap(), 10).unwrap();

        // Schema is inferred from the file, total comes from Parquet metadata, and
        // the sample is capped to the requested rows.
        assert_eq!(schema.fields().len(), 2);
        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(total, 50);
        let sample_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(sample_rows, 10);
    }

    #[test]
    fn read_batches_returns_all_rows() {
        let file = write_fixture(7);
        let batches = read_parquet_batches(file.path().to_str().unwrap()).unwrap();
        let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(rows, 7);
    }

    #[test]
    fn schema_ipc_round_trips() {
        // The preview encodes a schema-only IPC stream; decoding it must recover
        // the same fields (this is exactly the PreviewFile → target_schema_ipc
        // path the UI round-trips through the schema editor).
        let schema = Schema::new(vec![
            Field::new("a", DataType::Int64, false),
            Field::new("b", DataType::Utf8, true),
        ]);
        let ipc = encode_ipc_stream(&schema, &[]).unwrap();
        let decoded = decode_ipc_schema(&ipc).unwrap();
        assert_eq!(decoded.fields().len(), 2);
        assert_eq!(decoded.field(0).name(), "a");
        assert!(!decoded.field(0).is_nullable());
        assert!(decoded.field(1).is_nullable());
    }

    #[test]
    fn ddl_columns_maps_types_and_nullability() {
        let schema = Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
            Field::new("flag", DataType::Boolean, true),
        ]);
        let ddl = ddl_columns(&schema).unwrap();
        assert_eq!(ddl, "`id` BIGINT NOT NULL, `name` STRING, `flag` BOOLEAN");
    }

    #[test]
    fn ddl_columns_rejects_unsupported_type() {
        let schema = Schema::new(vec![Field::new("blob", DataType::Binary, true)]);
        let err = ddl_columns(&schema).unwrap_err();
        assert!(err.contains("unsupported type"), "got: {err}");
    }
}
