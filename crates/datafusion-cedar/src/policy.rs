use datafusion::error::Result;
use datafusion::logical_expr::LogicalPlan;

use cedar_oci::Decision;

use crate::principal::PrincipalIdentity;

#[cfg(feature = "governance")]
use datafusion::common::DFSchema;
#[cfg(feature = "governance")]
use datafusion::sql::TableReference;

/// Access-control policy applied during query planning.
///
/// Layer 1 of the policy stack: a coarse allow/deny over the tables and actions
/// a query references. Implementations inspect the [`LogicalPlan`] and decide
/// whether `principal` may execute it. The principal is passed as a
/// [`PrincipalIdentity`] (uid + attributes) so attribute-based policies
/// (`principal.role == ...`) can be evaluated.
///
/// With the `governance` feature, the trait also exposes [`Policy::table_policy`]
/// (Layer 2): the per-table row filters and column masks that apply to a
/// principal's access.
#[async_trait::async_trait]
pub trait Policy: std::fmt::Debug + Send + Sync {
    async fn is_allowed(
        &self,
        logical_plan: &LogicalPlan,
        principal: &PrincipalIdentity,
    ) -> Result<Decision>;

    /// Resolve the fine-grained enforcement (row filters + column masks) that
    /// apply when `principal` reads `table` with schema `schema`.
    ///
    /// Default: no fine-grained enforcement. The Cedar implementation derives
    /// filters and masks from policy residuals; see `crate::govern`.
    #[cfg(feature = "governance")]
    async fn table_policy(
        &self,
        _table: &TableReference,
        _schema: &DFSchema,
        _principal: &PrincipalIdentity,
    ) -> Result<crate::govern::TablePolicy> {
        Ok(crate::govern::TablePolicy::default())
    }
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
        _principal: &PrincipalIdentity,
    ) -> Result<Decision> {
        Ok(self.decision)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use datafusion::logical_expr::LogicalPlanBuilder;

    use cedar_oci::EntityUid;

    use super::*;

    fn principal() -> PrincipalIdentity {
        PrincipalIdentity::new(EntityUid::from_str("User::\"alice\"").unwrap())
    }

    #[tokio::test]
    async fn static_policy_returns_its_decision() {
        let plan = LogicalPlanBuilder::empty(false).build().unwrap();
        let allow = StaticPolicy::new(Decision::Allow);
        let deny = StaticPolicy::new(Decision::Deny);
        assert_eq!(
            allow.is_allowed(&plan, &principal()).await.unwrap(),
            Decision::Allow
        );
        assert_eq!(
            deny.is_allowed(&plan, &principal()).await.unwrap(),
            Decision::Deny
        );
    }
}
