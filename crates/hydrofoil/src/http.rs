//! HTTP query surface.
//!
//! A catalog-native `POST /query` endpoint that lets a thin caller (the
//! UC-quickstart UI server) run SQL through the full Hydrofoil engine over plain
//! HTTP — replacing the standalone Rust "query sidecar"
//! (`unitycatalog-quickstart/ui/query-sidecar`). Where that sidecar was a dumb
//! stateless executor — the UI server listed Delta tables, fetched their storage
//! locations, and vended per-table AWS credentials, then handed them in — this
//! endpoint does none of that on the caller's behalf: Hydrofoil resolves Unity
//! Catalog tables, vends credentials per-principal (with per-session credential
//! isolation), enforces the Cedar policy gate, and emits OpenLineage, all via the
//! same path the Flight SQL statement RPC uses (`server::get_flight_info_statement`
//! + `do_get_statement`), just collected instead of streamed.
//!
//! The response is Arrow IPC stream bytes, byte-for-byte the shape the sidecar
//! returned (`{arrow_ipc, row_count, elapsed_ms, tables_registered}`), so the
//! UI's JavaScript Arrow decode path is unchanged. View types (`Utf8View` /
//! `BinaryView`) are normalized to `Utf8` / `Binary` for older browser
//! `apache-arrow` clients, mirroring the sidecar's `arrow_out`.
//!
//! Runs alongside the Flight SQL gRPC server on its own port (see `main.rs`);
//! the two share one [`FlightSqlServiceImpl`] so sessions, the engine, and the
//! UC/Cedar/lineage wiring are common.
//!
//! **Trust boundary:** the principal is parsed from request headers
//! (`identity::principal_from_http_headers`) for local/dev use; exchanging the UC
//! `Authorization: Bearer` token for a verified identity is the same deferred
//! interceptor work flagged in `crate::identity`.

use std::sync::Arc;
use std::time::Instant;

use arrow::array::{Array, RecordBatch, new_empty_array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::ipc::writer::StreamWriter;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use datafusion::catalog::Session as _;
use datafusion::logical_expr::{LogicalPlan, LogicalPlanBuilder};
use datafusion::prelude::SQLOptions;
use datafusion_open_lineage::context::LineageContext;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::server::FlightSqlServiceImpl;

/// Shared handler state: the Flight SQL service (engine + session store +
/// UC/Cedar/lineage wiring) plus the query-limit policy.
#[derive(Clone)]
pub struct AppState {
    pub service: Arc<FlightSqlServiceImpl>,
    pub query_default_limit: u32,
    pub query_max_limit: u32,
}

/// Build the HTTP query router. Mounted by `main.rs` on the configured
/// `http_port`.
pub fn router(state: AppState) -> Router {
    Router::new()
        // `/healthz` matches the sidecar's health path so its Docker/UI health
        // check is a drop-in.
        .route("/healthz", get(|| async { "ok" }))
        .route("/query", post(query))
        .with_state(state)
}

/// `POST /query` request — SQL plus an optional default namespace and row limit.
///
/// `catalog`/`schema` are optional: fully-qualified `catalog.schema.table`
/// references in the SQL resolve against Unity Catalog regardless; they set the
/// session default namespace for bare names (the sidecar's callers always passed
/// catalog + schema).
#[derive(Debug, Deserialize)]
struct QueryRequest {
    sql: String,
    // Accepted for sidecar API parity. Bare-name defaulting against these is not
    // yet wired (the plan path takes a single statement); fully-qualified
    // references in `sql` resolve against Unity Catalog regardless. Retained so
    // the wire contract matches and to log the caller's intended namespace.
    #[serde(default)]
    catalog: Option<String>,
    #[serde(default)]
    schema: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

/// `POST /query` success response — the exact shape the query sidecar returned,
/// so the UI's JavaScript Arrow client is unchanged.
#[derive(Debug, Serialize)]
struct QueryResponse {
    /// Result rows encoded as an Arrow IPC *stream*.
    arrow_ipc: Vec<u8>,
    row_count: u64,
    elapsed_ms: u64,
    /// The Unity Catalog table references the query resolved against.
    tables_registered: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<ErrorBody>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorBody { error: msg.into() }),
    )
}

async fn query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    match execute(&state, &headers, req).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(msg) => bad_request(msg).into_response(),
    }
}

/// Run a query end-to-end through the engine and collect it into an Arrow IPC
/// response. Errors are returned as a flat message (rendered as a 400 by
/// [`query`]), matching the sidecar's `{ "error": … }` contract.
async fn execute(
    state: &AppState,
    headers: &HeaderMap,
    req: QueryRequest,
) -> Result<QueryResponse, String> {
    let started = Instant::now();

    if req.sql.trim().is_empty() {
        return Err("sql is required".into());
    }
    tracing::info!(
        catalog = req.catalog.as_deref().unwrap_or(""),
        schema = req.schema.as_deref().unwrap_or(""),
        "http query"
    );

    // Resolve the principal from headers, then its session — the same engine
    // (UC factory + Cedar policy) the Flight path uses. UC table resolution and
    // per-session credential vending happen inside planning/execution.
    let principal = crate::identity::principal_from_http_headers(headers).map_err(|e| e.message)?;
    let session = state
        .service
        .session_for_principal(principal)
        .await
        .map_err(|s| s.message().to_string())?;

    // Pin a lineage run id and snapshot the per-request context (SQL text). The
    // UI's HTTP calls carry no OpenLineage parent-run facet, so this is a fresh
    // root run rather than the metadata-derived context the Flight path builds.
    let lineage = LineageContext {
        run_id: Some(Uuid::now_v7()),
        // Derive a stable per-statement job name (the SQL hash) so distinct
        // queries are distinct Marquez jobs, matching the Flight path. The HTTP
        // surface carries no client job header, so the hash fallback is used.
        job_name: Some(crate::lineage::job_name_from_sql(&req.sql)),
        sql: Some(req.sql.clone()),
        ..Default::default()
    };
    let lh = session.lakehouse_for_query(lineage, None);

    // Plan off the async runtime (planning can be CPU-heavy). UC DDL detection +
    // table resolution happen here, exactly as for the Flight statement RPC.
    // Fully-qualified `catalog.schema.table` references resolve against Unity
    // Catalog; `req.catalog`/`req.schema` are accepted for sidecar API parity
    // but bare-name defaulting is not yet wired (see module docs).
    let plan = state
        .service
        .executor()
        .create_logical_plan(lh.clone(), req.sql.clone())
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
    // (clamp rather than reject — the safer governance default). The gate below
    // then sees the final, capped plan.
    let limit = req
        .limit
        .unwrap_or(state.query_default_limit)
        .min(state.query_max_limit) as usize;
    let plan = LogicalPlanBuilder::from(plan)
        .limit(0, Some(limit))
        .map_err(|e| format!("Error applying limit: {e}"))?
        .build()
        .map_err(|e| format!("Error applying limit: {e}"))?;

    // Execute through the per-query session: `create_physical_plan` fires the
    // Cedar coarse gate (and, under the `governance` feature, row/column
    // masking) and the OpenLineage START/COMPLETE events.
    let physical = lh
        .create_physical_plan(&plan)
        .await
        .map_err(|e| e.to_string())?;
    let schema = physical.schema();
    let batches = datafusion::physical_plan::collect(physical, lh.task_ctx())
        .await
        .map_err(|e| e.to_string())?;

    let row_count = batches.iter().map(|b| b.num_rows() as u64).sum();
    let arrow_ipc = encode_ipc_stream(schema.as_ref(), &batches)?;

    Ok(QueryResponse {
        arrow_ipc,
        row_count,
        elapsed_ms: started.elapsed().as_millis() as u64,
        tables_registered,
    })
}

/// The table references a plan resolved against, as `catalog.schema.table`
/// strings (deduped + sorted for stable output), read from its table scans.
fn referenced_tables(plan: &LogicalPlan) -> Vec<String> {
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
/// clients (ported from the sidecar's `arrow_out`).
fn encode_ipc_stream(schema: &Schema, batches: &[RecordBatch]) -> Result<Vec<u8>, String> {
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
        let bytes = encode_ipc_stream(&schema, &[]).unwrap();
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

        let bytes = encode_ipc_stream(&schema, std::slice::from_ref(&batch)).unwrap();
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
