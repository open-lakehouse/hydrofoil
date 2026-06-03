use datafusion::error::Result;
use datafusion::logical_expr::LogicalPlan;

use cedar_oci::{Decision, EntityUid};

/// Access-control policy applied during query planning.
///
/// Layer 1 of the policy stack: a coarse allow/deny over the tables and actions
/// a query references. Implementations inspect the [`LogicalPlan`] and decide
/// whether `principal` may execute it.
#[async_trait::async_trait]
pub trait Policy: std::fmt::Debug + Send + Sync {
    async fn is_allowed(
        &self,
        logical_plan: &LogicalPlan,
        principal: &EntityUid,
    ) -> Result<Decision>;
}

/// A [`Policy`] that returns the same decision for every query.
///
/// Used as the default when no real policy engine is wired (e.g.
/// `StaticPolicy::new(Decision::Allow)` for an open, ungoverned server).
#[derive(Debug, Clone)]
pub struct StaticPolicy {
    decision: Decision,
}

impl StaticPolicy {
    pub fn new(decision: Decision) -> Self {
        Self { decision }
    }
}

#[async_trait::async_trait]
impl Policy for StaticPolicy {
    async fn is_allowed(
        &self,
        _logical_plan: &LogicalPlan,
        _principal: &EntityUid,
    ) -> Result<Decision> {
        Ok(self.decision)
    }
}
