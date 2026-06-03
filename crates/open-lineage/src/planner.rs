//! A [`QueryPlanner`] wrapper that emits OpenLineage events around query
//! execution.
//!
//! `create_physical_plan` receives the full optimized [`LogicalPlan`] (the best
//! lineage signal) and the [`SessionState`] (which the context provider reads),
//! and lets us correlate START / COMPLETE / FAIL under one run id.
//!
//! START is emitted here at plan time. COMPLETE / FAIL are emitted at *end of
//! execution* by the [`OpenLineageExec`] node we wrap the physical plan in, so
//! a query that plans cleanly but errors mid-stream reports FAIL, not COMPLETE.
//! A planning failure (no plan to wrap) emits FAIL directly.

use std::sync::Arc;

use async_trait::async_trait;
use datafusion::error::Result;
use datafusion::execution::context::{QueryPlanner, SessionState};
use datafusion::logical_expr::LogicalPlan;
use datafusion::physical_plan::ExecutionPlan;
use uuid::Uuid;

use crate::builder::{complete_event, fail_event, start_event};
use crate::client::OpenLineageClient;
use crate::config::OpenLineageConfig;
use crate::context::LineageContextProvider;
use crate::exec::OpenLineageExec;
use crate::extract::extract;

/// Wraps an inner [`QueryPlanner`], emitting OpenLineage run events.
#[derive(Debug)]
pub struct OpenLineageQueryPlanner {
    inner: Arc<dyn QueryPlanner + Send + Sync>,
    client: OpenLineageClient,
    context: Arc<dyn LineageContextProvider>,
    config: OpenLineageConfig,
}

impl OpenLineageQueryPlanner {
    pub fn new(
        inner: Arc<dyn QueryPlanner + Send + Sync>,
        client: OpenLineageClient,
        context: Arc<dyn LineageContextProvider>,
        config: OpenLineageConfig,
    ) -> Self {
        Self {
            inner,
            client,
            context,
            config,
        }
    }
}

#[async_trait]
impl QueryPlanner for OpenLineageQueryPlanner {
    async fn create_physical_plan(
        &self,
        logical_plan: &LogicalPlan,
        session_state: &SessionState,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let mut lineage = extract(logical_plan, &self.config);
        let cx = self.context.context(session_state).await;
        // The SQL text isn't recoverable from the plan; take it from the
        // host-supplied context (absent on non-SQL paths, e.g. ingest).
        lineage.sql = cx.sql.clone();
        let run_id = cx.run_id.unwrap_or_else(Uuid::now_v7);

        self.client
            .emit(start_event(run_id, &lineage, &cx, &self.config));

        match self
            .inner
            .create_physical_plan(logical_plan, session_state)
            .await
        {
            Ok(plan) => {
                // Defer COMPLETE/FAIL to end of execution: wrap the root plan in
                // a node that emits the pre-built COMPLETE event (or FAIL) once
                // all output partitions finish, under the same run id as START.
                let complete = complete_event(run_id, &lineage, &cx, &self.config);
                Ok(OpenLineageExec::new(
                    plan,
                    self.client.clone(),
                    complete,
                    self.config.producer.clone(),
                ))
            }
            Err(err) => {
                // Planning failed outright — no plan to wrap, emit FAIL now.
                self.client.emit(fail_event(
                    run_id,
                    &lineage,
                    &cx,
                    &self.config,
                    &err.to_string(),
                ));
                Err(err)
            }
        }
    }
}
