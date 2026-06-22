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

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use datafusion::catalog::Session as _;
use serde::{Deserialize, Serialize};

use crate::query::{PlannedQuery, encode_ipc_stream, plan_query};
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
///
/// Serves three surfaces on one port:
///   - `GET /healthz` — health check (matches the sidecar's path).
///   - `POST /query` — the buffered, Arrow-IPC query endpoint.
///   - The ConnectRPC `QueryService` (server-streaming SQL) as a
///     `fallback_service`, so its `/hydrofoil.query.v1.QueryService/*` paths fall
///     through to the connect dispatcher. Both share one `FlightSqlServiceImpl`.
pub fn router(state: AppState) -> Router {
    let connect = crate::query_service::QueryAppState {
        service: state.service.clone(),
        query_default_limit: state.query_default_limit,
        query_max_limit: state.query_max_limit,
    }
    .register(connectrpc::Router::new());

    Router::new()
        // `/healthz` matches the sidecar's health path so its Docker/UI health
        // check is a drop-in.
        .route("/healthz", get(|| async { "ok" }))
        .route("/query", post(query))
        .with_state(state)
        .fallback_service(connect.into_axum_service())
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

    // Resolve the principal + session and plan the SQL through the shared engine
    // path (UC table resolution, per-session credential vending, DDL/DML gate,
    // row-cap LIMIT) — the same pipeline the streaming Connect surface uses.
    // `req.catalog`/`req.schema` are accepted for sidecar API parity but
    // bare-name defaulting is not yet wired (see module docs); fully-qualified
    // references in `sql` resolve against Unity Catalog regardless.
    let PlannedQuery {
        lh,
        plan,
        tables_registered,
    } = plan_query(
        &state.service,
        headers,
        &req.sql,
        req.limit,
        state.query_default_limit,
        state.query_max_limit,
    )
    .await?;

    // Execute through the per-query session: `create_physical_plan` fires the
    // Cedar coarse gate (and, under the `governance` feature, row/column
    // masking) and the OpenLineage START/COMPLETE events. This surface collects
    // all batches before responding (the Connect surface streams them).
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
