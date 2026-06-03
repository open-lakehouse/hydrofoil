//! One-call helper to add OpenLineage instrumentation to a `SessionState`.

use std::sync::Arc;

use datafusion::execution::SessionStateBuilder;
use datafusion::execution::context::SessionState;

use crate::client::OpenLineageClient;
use crate::config::OpenLineageConfig;
use crate::context::{LineageContextProvider, StaticContextProvider};
use crate::planner::OpenLineageQueryPlanner;

/// Wrap `state`'s query planner so OpenLineage events are emitted around
/// physical planning. Preserves any existing custom `QueryPlanner`.
///
/// Mirrors `datafusion_tracing::instrument_session_state`. Use
/// [`StaticContextProvider::default`] for the `context` argument when there is
/// no orchestration context to inject.
pub fn instrument_session_state(
    state: SessionState,
    client: OpenLineageClient,
    context: Arc<dyn LineageContextProvider>,
    config: OpenLineageConfig,
) -> SessionState {
    let inner = state.query_planner().clone();
    let planner = Arc::new(OpenLineageQueryPlanner::new(inner, client, context, config));
    SessionStateBuilder::from(state)
        .with_query_planner(planner)
        .build()
}

/// Convenience wrapper using a [`StaticContextProvider`] with default (empty)
/// context.
pub fn instrument_session_state_simple(
    state: SessionState,
    client: OpenLineageClient,
    config: OpenLineageConfig,
) -> SessionState {
    instrument_session_state(
        state,
        client,
        Arc::new(StaticContextProvider::default()),
        config,
    )
}
