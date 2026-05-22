use datafusion::error::Result;
use datafusion::logical_expr::LogicalPlan;

use hydrofoil_policy::{Decision, EntityUid};

#[cfg(feature = "cedar")]
pub use cedar::{CedarPolicy, SimpleEntityProvider, SimplePolicySetProvider};

#[cfg(feature = "cedar")]
mod cedar;

#[async_trait::async_trait]
pub trait Policy: std::fmt::Debug + Send + Sync {
    async fn is_allowed(
        &self,
        logical_plan: &LogicalPlan,
        principal: &EntityUid,
    ) -> Result<Decision>;
}

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
