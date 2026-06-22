//! ConnectRPC `QueryService` — server-streaming SQL for web/UI clients.
//!
//! `RunQuery` runs the *same* engine path as the buffered `POST /query`
//! ([`crate::http`]) — principal/session resolution, lineage, planning, the
//! DDL/DML gate, and the row-cap LIMIT all come from [`crate::query::plan_query`]
//! — then streams the result instead of collecting it. Each produced record
//! batch is encoded as a self-contained Arrow IPC stream
//! ([`crate::query::encode_ipc_batch`]) and sent as one `RunQueryResponse`, so a
//! browser decodes each chunk independently and renders rows progressively.
//!
//! The Cedar coarse gate (and, under the `governance` feature, row/column
//! masking) and the OpenLineage START/COMPLETE events fire inside
//! `create_physical_plan`, which runs in the streaming driver task — so a denied
//! query errors the stream before any batch is sent.
//!
//! **Trust boundary:** the principal is parsed from request headers
//! (`identity::principal_from_http_headers`), the same deferred-verification
//! story as [`crate::http`]. `RequestContext::headers()` yields the same
//! `http::HeaderMap` those helpers accept, so no extra header plumbing is needed.

use std::sync::Arc;

use arrow_flight::error::FlightError;
use connectrpc::{
    ConnectError, RequestContext, Response, ServiceRequest, ServiceResult, ServiceStream,
};
use datafusion::catalog::Session;
use datafusion::physical_plan::execute_stream;
use futures::StreamExt;
use tokio::runtime::Handle;
use tracing::debug;

use crate::proto::v1::{RunQueryRequest, RunQueryResponse};
use crate::query::{PlannedQuery, encode_ipc_batch, encode_ipc_schema_only, plan_query};
use crate::server::FlightSqlServiceImpl;
use crate::services::v1::QueryService;
use crate::stream::ReceiverStreamBuilder;

/// Handler state for the ConnectRPC query surface: the shared engine + session
/// store ([`FlightSqlServiceImpl`]) plus the query-limit policy. Mirrors
/// [`crate::http::AppState`]; both surfaces share one `Arc<FlightSqlServiceImpl>`.
#[derive(Clone)]
pub struct QueryAppState {
    pub service: Arc<FlightSqlServiceImpl>,
    pub query_default_limit: u32,
    pub query_max_limit: u32,
}

impl QueryAppState {
    /// Register the QueryService on a ConnectRPC router (thin wrapper over the
    /// generated `QueryServiceExt::register`, matching portal's `register_all`).
    pub fn register(self, router: connectrpc::Router) -> connectrpc::Router {
        use crate::services::v1::QueryServiceExt;
        Arc::new(self).register(router)
    }
}

impl QueryService for QueryAppState {
    async fn run_query(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RunQueryRequest>,
    ) -> ServiceResult<ServiceStream<RunQueryResponse>> {
        // Copy owned data out before building the response stream: stream items
        // must be `'static` and cannot borrow from `request`, `&self`, or `ctx`.
        let sql = request.sql.to_owned();
        let limit = request.limit;
        let headers = ctx.headers().clone();

        if sql.trim().is_empty() {
            return Err(ConnectError::invalid_argument("sql is required"));
        }
        tracing::info!("connect query");

        // Resolve principal + session and plan through the shared engine path
        // (UC resolution, per-session vending, DDL/DML gate, row-cap LIMIT).
        let planned = plan_query(
            &self.service,
            &headers,
            &sql,
            limit,
            self.query_default_limit,
            self.query_max_limit,
        )
        .await
        .map_err(ConnectError::invalid_argument)?;

        // Drive execution on the CPU runtime and stream Arrow IPC chunks.
        let stream = run_query_arrow_stream(planned, self.service.executor().handle())
            .map(|item| item.map_err(flight_to_connect));

        Ok(Response::stream(stream))
    }
}

/// Map a Flight-layer streaming error (the error type the shared
/// [`ReceiverStreamBuilder`] machinery uses) to a ConnectRPC error.
fn flight_to_connect(err: FlightError) -> ConnectError {
    ConnectError::internal(err.to_string())
}

/// Execute a planned query and stream its record batches as `RunQueryResponse`
/// messages, one self-contained Arrow IPC stream per batch.
///
/// Modeled on [`crate::stream::FlightDataReceiverStreamBuilder::execute_logical_plan`]:
/// it reuses [`ReceiverStreamBuilder`] for bounded buffering, panic propagation,
/// and automatic task cancellation when the receiver (the browser's stream) is
/// dropped mid-query. The driver runs on the CPU runtime `handle`.
///
/// A query that yields no batches still emits exactly one schema-only message so
/// the client learns the result's columns.
fn run_query_arrow_stream(
    planned: PlannedQuery,
    handle: &Handle,
) -> futures::stream::BoxStream<'static, Result<RunQueryResponse, FlightError>> {
    let mut builder = ReceiverStreamBuilder::<RunQueryResponse>::new(2);
    let tx = builder.tx();

    let PlannedQuery { lh, plan, .. } = planned;
    let ctx: Arc<dyn Session> = Arc::new(lh);

    let driver = async move {
        // `create_physical_plan` fires the Cedar gate + lineage events.
        let exec = match ctx.create_physical_plan(&plan).await {
            Ok(exec) => exec,
            Err(e) => {
                tx.send(Err(to_flight_err(e))).await.ok();
                debug!("Stopping execution: error creating physical plan");
                return Ok(());
            }
        };
        let schema = exec.schema();

        let mut stream = match execute_stream(exec, ctx.task_ctx()) {
            Ok(stream) => stream,
            Err(e) => {
                tx.send(Err(to_flight_err(e))).await.ok();
                return Ok(());
            }
        };

        let mut sent_any = false;
        while let Some(batch) = stream.next().await {
            let batch = match batch {
                Ok(batch) => batch,
                Err(e) => {
                    tx.send(Err(to_flight_err(e))).await.ok();
                    return Ok(());
                }
            };
            let item = encode_ipc_batch(schema.as_ref(), &batch)
                .map(|bytes| RunQueryResponse {
                    arrow_ipc: bytes,
                    num_rows: batch.num_rows() as u64,
                    ..Default::default()
                })
                .map_err(|e| FlightError::ExternalError(e.into()));
            let is_err = item.is_err();
            if tx.send(item).await.is_err() {
                // Receiver dropped (browser cancelled): stop driving the plan.
                debug!("Stopping execution: output is gone");
                return Ok(());
            }
            if is_err {
                return Ok(());
            }
            sent_any = true;
        }

        // No rows: emit one schema-only message so the client gets the columns.
        if !sent_any {
            let item = encode_ipc_schema_only(schema.as_ref())
                .map(|bytes| RunQueryResponse {
                    arrow_ipc: bytes,
                    num_rows: 0,
                    ..Default::default()
                })
                .map_err(|e| FlightError::ExternalError(e.into()));
            tx.send(item).await.ok();
        }

        Ok(())
    };

    builder.spawn_on(driver, handle);
    builder.build()
}

/// Convert a DataFusion error into the Flight-layer error the stream carries.
fn to_flight_err(error: datafusion::error::DataFusionError) -> FlightError {
    FlightError::from_external_error(Box::new(error))
}
