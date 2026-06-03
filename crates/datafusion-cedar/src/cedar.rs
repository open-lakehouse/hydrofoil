use cedar_local_agent::public::simple::Authorizer;
use cedar_local_agent::public::{SimpleEntityProvider, SimplePolicySetProvider};
use cedar_policy::Entities;
use datafusion::common::plan_datafusion_err;
use datafusion::error::Result;
use datafusion::logical_expr::LogicalPlan;

use cedar_oci::{Decision, EntityUid};

use crate::policy::Policy;
use crate::visitor::authorize_plan;

/// A [`Policy`] backed by a Cedar [`Authorizer`].
///
/// Generic over any policy-set and entity provider (e.g. `cedar-oci`'s
/// `OciPolicyProvider`), so the policy source is pluggable.
#[derive(Debug)]
pub struct CedarPolicy<P, E>
where
    P: SimplePolicySetProvider + 'static,
    E: SimpleEntityProvider + 'static,
{
    authorizer: Authorizer<P, E>,
}

impl<P, E> CedarPolicy<P, E>
where
    P: SimplePolicySetProvider + 'static,
    E: SimpleEntityProvider + 'static,
{
    // Public `from_oci` constructor lands in Phase 1; `new` stays private until then.
    #[allow(dead_code)]
    fn new(authorizer: Authorizer<P, E>) -> Self {
        Self { authorizer }
    }
}

#[async_trait::async_trait]
impl<P, E> Policy for CedarPolicy<P, E>
where
    P: SimplePolicySetProvider + 'static,
    E: SimpleEntityProvider + 'static,
{
    async fn is_allowed(
        &self,
        logical_plan: &LogicalPlan,
        principal: &EntityUid,
    ) -> Result<Decision> {
        let requests = authorize_plan(logical_plan, principal)?;
        for request in requests {
            let decision = self
                .authorizer
                .is_authorized(&request, &Entities::empty())
                .await
                .map_err(|e| plan_datafusion_err!("Failed to authorize plan: {}", e))?
                .decision();
            if decision == Decision::Deny {
                return Ok(Decision::Deny);
            }
        }
        Ok(Decision::Allow)
    }
}
