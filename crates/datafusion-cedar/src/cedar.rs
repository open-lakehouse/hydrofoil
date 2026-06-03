use std::sync::Arc;

use cedar_local_agent::public::simple::{Authorizer, AuthorizerConfigBuilder};
use cedar_local_agent::public::{SimpleEntityProvider, SimplePolicySetProvider};
use cedar_policy::Entities;
use datafusion::error::Result;
use datafusion::logical_expr::LogicalPlan;

use cedar_oci::{Decision, OciPolicyProvider};

use crate::policy::Policy;
use crate::principal::PrincipalIdentity;
use crate::visitor::authorize_plan;

/// A [`Policy`] backed by a Cedar [`Authorizer`].
///
/// Generic over any policy-set and entity provider (e.g. `cedar-oci`'s
/// [`OciPolicyProvider`]), so the policy source is pluggable.
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
    fn new(authorizer: Authorizer<P, E>) -> Self {
        Self { authorizer }
    }
}

impl CedarPolicy<OciPolicyProvider, OciPolicyProvider> {
    /// Build a Cedar policy that sources its policy set, schema, and entities
    /// from an OCI registry reference (e.g.
    /// `localhost:10100/hydrofoil/plan-policy:latest`).
    ///
    /// The same provider backs both the policy-set and entity providers.
    pub async fn from_oci(reference: &str) -> Result<Self> {
        let provider = Arc::new(
            OciPolicyProvider::from_reference(reference)
                .await
                .map_err(|e| {
                    datafusion::error::DataFusionError::Plan(format!(
                        "Failed to load Cedar policy from OCI reference '{reference}': {e}"
                    ))
                })?,
        );
        let config = AuthorizerConfigBuilder::default()
            .policy_set_provider(provider.clone())
            .entity_provider(provider)
            .build()
            .map_err(|e| {
                datafusion::error::DataFusionError::Plan(format!(
                    "Failed to build Cedar authorizer: {e}"
                ))
            })?;
        Ok(Self::new(Authorizer::new(config)))
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
        principal: &PrincipalIdentity,
    ) -> Result<Decision> {
        let requests = authorize_plan(logical_plan, principal)?;

        // Supply the principal as a request-time entity so policies can resolve
        // `principal.<attr>` references. cedar-local-agent merges this with the
        // entities the provider vends.
        let principal_entities = Entities::from_entities([principal.to_entity()], None)
            .unwrap_or_else(|_| Entities::empty());

        for request in requests {
            // Fail closed: any authorizer error denies the query rather than
            // letting it through.
            let decision = match self
                .authorizer
                .is_authorized(&request, &principal_entities)
                .await
            {
                Ok(response) => response.decision(),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "Cedar authorization failed; denying (fail-closed)"
                    );
                    return Ok(Decision::Deny);
                }
            };
            if decision == Decision::Deny {
                return Ok(Decision::Deny);
            }
        }
        Ok(Decision::Allow)
    }
}
