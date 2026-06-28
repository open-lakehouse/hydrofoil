//! Query planning + Arrow IPC encoding shared by the HTTP and ConnectRPC query
//! surfaces.
//!
//! Both the buffered `POST /query` endpoint ([`crate::http`]) and the
//! server-streaming ConnectRPC `QueryService.RunQuery`
//! ([`crate::query_service`]) run the *same* engine path: resolve the principal
//! and its session, attach a lineage context, plan the SQL, block native
//! DDL/DML, and clamp the result with an outer `LIMIT`. They diverge only at
//! execution — the HTTP path collects all batches, the Connect path streams
//! them — so the planning pipeline lives here as [`plan_query`] and the Arrow
//! IPC encoders are shared.

use std::sync::Arc;

use arrow::array::{Array, RecordBatch, new_empty_array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::ipc::writer::StreamWriter;
use axum::http::HeaderMap;
use datafusion::logical_expr::{LogicalPlan, LogicalPlanBuilder};
use datafusion::prelude::SQLOptions;
use datafusion_openlineage::context::LineageContext;
use uuid::Uuid;

use crate::server::FlightSqlServiceImpl;
use crate::session::LakehouseSession;

/// A query that has been planned and is ready to execute: the per-query
/// [`LakehouseSession`] (with its lineage context attached), the capped,
/// DDL/DML-gated logical plan, and the table references it resolved against.
///
/// `create_physical_plan` is deliberately *not* called here — it fires the
/// Cedar gate and OpenLineage START/COMPLETE events, which belong to the
/// execution step. The HTTP path runs it then `collect`s; the Connect path runs
/// it inside the streaming driver.
pub(crate) struct PlannedQuery {
    pub lh: LakehouseSession,
    pub plan: LogicalPlan,
    pub tables_registered: Vec<String>,
}

/// Resolve the principal + session, plan the SQL, block native DDL/DML, and
/// clamp with an outer `LIMIT`.
///
/// This is the engine path both query surfaces share — see the module docs. The
/// principal is parsed from request headers (`identity::principal_from_http_headers`);
/// a per-user UC token (`Authorization: Bearer <uc-jwt>`) selects that user's UC
/// factory so UC enforces their permissions. Errors are returned as a flat
/// message (rendered as a 400 by HTTP, an invalid-argument by Connect).
pub(crate) async fn plan_query(
    service: &FlightSqlServiceImpl,
    headers: &HeaderMap,
    sql: &str,
    limit: Option<u32>,
    default_limit: u32,
    max_limit: u32,
) -> Result<PlannedQuery, String> {
    let principal = crate::identity::principal_from_http_headers(headers).map_err(|e| e.message)?;
    let uc_token = crate::identity::uc_token_from_http_headers(headers);
    let session = service
        .session_for_principal(principal, uc_token.as_deref())
        .await
        .map_err(|s| s.message().to_string())?;

    // Pin a lineage run id and snapshot the per-request context (SQL text). The
    // UI's calls carry no OpenLineage parent-run facet, so this is a fresh root
    // run rather than the metadata-derived context the Flight path builds. A
    // stable per-statement job name (the SQL hash) makes distinct queries
    // distinct Marquez jobs, matching the Flight path.
    let lineage = LineageContext {
        run_id: Some(Uuid::now_v7()),
        job_name: Some(crate::lineage::job_name_from_sql(sql)),
        sql: Some(sql.to_string()),
        ..Default::default()
    };
    let lh = session.lakehouse_for_query(lineage, None);

    // Plan off the async runtime (planning can be CPU-heavy). UC DDL detection +
    // table resolution happen here. Fully-qualified `catalog.schema.table`
    // references resolve against Unity Catalog regardless of any session default
    // namespace.
    let plan = service
        .executor()
        .create_logical_plan(lh.clone(), sql.to_string())
        .await
        .map_err(|e| format!("Error building plan: {e}"))?;

    // Block DataFusion-native DDL/DML, matching `server::do_get_handle`. Unity
    // Catalog DDL rides through as an `Extension` node and is authorized by the
    // Cedar gate in `create_physical_plan`, not here.
    SQLOptions::new()
        .with_allow_ddl(false)
        .with_allow_dml(false)
        .verify_plan(&plan)
        .map_err(|e| format!("{e:?}"))?;

    let tables_registered = referenced_tables(&plan);

    // Enforce the row cap as an outer LIMIT on the planned query, so a request
    // can never pull more than the configured maximum regardless of the SQL
    // (clamp rather than reject — the safer governance default). The gate that
    // fires in `create_physical_plan` then sees the final, capped plan.
    let limit = limit.unwrap_or(default_limit).min(max_limit) as usize;
    let plan = LogicalPlanBuilder::from(plan)
        .limit(0, Some(limit))
        .map_err(|e| format!("Error applying limit: {e}"))?
        .build()
        .map_err(|e| format!("Error applying limit: {e}"))?;

    Ok(PlannedQuery {
        lh,
        plan,
        tables_registered,
    })
}

/// The table references a plan resolved against, as `catalog.schema.table`
/// strings (deduped + sorted for stable output), read from its table scans.
pub(crate) fn referenced_tables(plan: &LogicalPlan) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    collect_table_names(plan, &mut set);
    set.into_iter().collect()
}

fn collect_table_names(plan: &LogicalPlan, out: &mut std::collections::BTreeSet<String>) {
    if let LogicalPlan::TableScan(scan) = plan {
        out.insert(scan.table_name.to_string());
    }
    for child in plan.inputs() {
        collect_table_names(child, out);
    }
}

/// Encode record batches as an Arrow IPC *stream*, normalizing `Utf8View` /
/// `BinaryView` columns to `Utf8` / `Binary` for older browser `apache-arrow`
/// clients (ported from the query sidecar's `arrow_out`). Used by the buffered
/// HTTP path, which collects every batch into one stream.
pub(crate) fn encode_ipc_stream(
    schema: &Schema,
    batches: &[RecordBatch],
) -> Result<Vec<u8>, String> {
    let normalized_schema = Arc::new(normalize_schema(schema));
    let mut buf = Vec::new();
    {
        let mut writer = StreamWriter::try_new(&mut buf, normalized_schema.as_ref())
            .map_err(|e| format!("arrow ipc: {e}"))?;
        // An empty result still produces a valid stream: the writer emits the
        // schema message on construction, then `finish` closes it.
        for batch in batches {
            let normalized = normalize_batch(batch, &normalized_schema)?;
            writer
                .write(&normalized)
                .map_err(|e| format!("arrow ipc: {e}"))?;
        }
        writer.finish().map_err(|e| format!("arrow ipc: {e}"))?;
    }
    Ok(buf)
}

/// Encode a single record batch as a *self-contained* Arrow IPC stream (schema
/// message + the batch + EOS), with the same `Utf8View`/`BinaryView`
/// normalization as [`encode_ipc_stream`]. Used by the streaming Connect path:
/// one such message per batch, each independently decodable with `tableFromIPC`.
pub(crate) fn encode_ipc_batch(schema: &Schema, batch: &RecordBatch) -> Result<Vec<u8>, String> {
    encode_ipc_stream(schema, std::slice::from_ref(batch))
}

/// Encode a schema-only Arrow IPC stream (schema message + EOS, no batches). The
/// streaming Connect path emits exactly one of these when a query yields no
/// rows, so the browser still learns the result's columns.
pub(crate) fn encode_ipc_schema_only(schema: &Schema) -> Result<Vec<u8>, String> {
    encode_ipc_stream(schema, &[])
}

/// Map `Utf8View`/`BinaryView` fields to their non-view counterparts; all other
/// fields pass through unchanged.
fn normalize_schema(schema: &Schema) -> Schema {
    let fields: Vec<Field> = schema
        .fields()
        .iter()
        .map(|f| {
            let dt = match f.data_type() {
                DataType::Utf8View => DataType::Utf8,
                DataType::BinaryView => DataType::Binary,
                other => other.clone(),
            };
            Field::new(f.name(), dt, f.is_nullable())
        })
        .collect();
    Schema::new(fields)
}

/// Cast any `Utf8View`/`BinaryView` columns of `batch` to match
/// `target_schema`, leaving other columns untouched.
fn normalize_batch(
    batch: &RecordBatch,
    target_schema: &Arc<Schema>,
) -> Result<RecordBatch, String> {
    let columns = batch
        .columns()
        .iter()
        .zip(target_schema.fields())
        .map(|(col, field)| {
            if col.data_type() == field.data_type() {
                Ok(col.clone())
            } else if batch.num_rows() == 0 {
                // Preserve an empty column's target type without a cast.
                Ok(new_empty_array(field.data_type()))
            } else {
                arrow::compute::cast(col, field.data_type())
                    .map_err(|e| format!("arrow cast {}: {e}", field.name()))
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    RecordBatch::try_new(target_schema.clone(), columns).map_err(|e| format!("arrow batch: {e}"))
}

#[cfg(test)]
mod tests {
    use arrow::array::{Int64Array, StringArray, StringViewArray};
    use arrow::ipc::reader::StreamReader;

    use super::*;

    /// Decode an Arrow IPC stream back into batches + schema.
    fn decode(bytes: &[u8]) -> (Arc<Schema>, Vec<RecordBatch>) {
        let reader = StreamReader::try_new(std::io::Cursor::new(bytes), None).unwrap();
        let schema = reader.schema();
        let batches = reader.map(|b| b.unwrap()).collect();
        (schema, batches)
    }

    #[test]
    fn round_trips_a_plain_batch() {
        let schema = Schema::new(vec![Field::new("id", DataType::Int64, false)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema.clone()),
            vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
        )
        .unwrap();

        let bytes = encode_ipc_stream(&schema, std::slice::from_ref(&batch)).unwrap();
        let (out_schema, out) = decode(&bytes);
        assert_eq!(out_schema.field(0).data_type(), &DataType::Int64);
        assert_eq!(out.iter().map(|b| b.num_rows()).sum::<usize>(), 3);
    }

    #[test]
    fn empty_result_still_encodes_a_valid_schema_only_stream() {
        let schema = Schema::new(vec![Field::new("id", DataType::Int64, false)]);
        let bytes = encode_ipc_schema_only(&schema).unwrap();
        let (out_schema, out) = decode(&bytes);
        assert_eq!(out_schema.field(0).name(), "id");
        assert_eq!(out.iter().map(|b| b.num_rows()).sum::<usize>(), 0);
    }

    /// `Utf8View` columns are normalized to `Utf8` so older browser
    /// `apache-arrow` clients can decode the stream (sidecar parity).
    #[test]
    fn normalizes_utf8_view_to_utf8() {
        let schema = Schema::new(vec![Field::new("name", DataType::Utf8View, true)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema.clone()),
            vec![Arc::new(StringViewArray::from(vec![
                Some("a"),
                None,
                Some("c"),
            ]))],
        )
        .unwrap();

        let bytes = encode_ipc_batch(&schema, &batch).unwrap();
        let (out_schema, out) = decode(&bytes);
        assert_eq!(
            out_schema.field(0).data_type(),
            &DataType::Utf8,
            "Utf8View must be normalized to Utf8 on the wire"
        );
        let col = out[0]
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("decoded as Utf8");
        assert_eq!(col.value(0), "a");
        assert!(col.is_null(1));
        assert_eq!(col.value(2), "c");
    }

    #[test]
    fn referenced_tables_are_deduped_and_sorted() {
        use datafusion::arrow::datatypes::Schema as DfSchema;
        use datafusion::datasource::empty::EmptyTable;
        use datafusion::logical_expr::LogicalPlanBuilder;
        use datafusion::prelude::SessionContext;

        let ctx = SessionContext::new();
        let provider = Arc::new(EmptyTable::new(Arc::new(DfSchema::empty())));
        // Two scans of the same table union'd: the name must appear once.
        let scan = LogicalPlanBuilder::scan(
            "cat.sch.t",
            datafusion::datasource::provider_as_source(provider),
            None,
        )
        .unwrap();
        let plan = scan
            .clone()
            .union(scan.build().unwrap())
            .unwrap()
            .build()
            .unwrap();
        let _ = &ctx; // session not needed beyond constructing the provider source
        assert_eq!(referenced_tables(&plan), vec!["cat.sch.t".to_string()]);
    }
}
