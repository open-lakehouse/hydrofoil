//! A [`QueryPlanner`] wrapper that emits OpenLineage events around physical
//! planning.
//!
//! `create_physical_plan` receives the full optimized [`LogicalPlan`] (the best
//! lineage signal) and the [`SessionState`] (which the context provider reads),
//! and lets us correlate START / COMPLETE / FAIL under one run id.
//!
//! NOTE (v1 limitation): COMPLETE/FAIL are emitted at *plan creation*, not at
//! end of execution. The planned follow-up wraps the physical plan to move
//! COMPLETE/FAIL to end-of-stream. See the plan's follow-up section.

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
        let lineage = extract(logical_plan, &self.config);
        let cx = self.context.context(session_state).await;
        let run_id = cx.run_id.unwrap_or_else(Uuid::now_v7);

        self.client
            .emit(start_event(run_id, &lineage, &cx, &self.config));

        match self
            .inner
            .create_physical_plan(logical_plan, session_state)
            .await
        {
            Ok(plan) => {
                self.client
                    .emit(complete_event(run_id, &lineage, &cx, &self.config));
                Ok(plan)
            }
            Err(err) => {
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
