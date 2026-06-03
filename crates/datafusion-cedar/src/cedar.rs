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

#[cfg(feature = "governance")]
use {
    crate::govern::TablePolicy,
    crate::translate::{CedarResidualTranslator, ResidualTranslator},
    cedar_policy::{Context, EntityTypeName, RequestBuilder},
    datafusion::common::DFSchema,
    datafusion::logical_expr::lit,
    datafusion::sql::TableReference,
    std::str::FromStr as _,
};

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

    #[cfg(feature = "governance")]
    async fn table_policy(
        &self,
        table: &TableReference,
        _schema: &DFSchema,
        principal: &PrincipalIdentity,
    ) -> Result<TablePolicy> {
        use cedar_oci::{EntityId, EntityUid};

        // Partial request: concrete principal + read_table action, but an
        // *unknown* resource of type `Table`. Policies that gate on
        // `resource.<attr>` come back as residuals over the row.
        let read_action = EntityUid::from_type_name_and_id(
            "Action".parse().unwrap(),
            EntityId::new("read_table"),
        );
        let table_type = EntityTypeName::from_str("Table")
            .map_err(|e| datafusion::error::DataFusionError::Plan(e.to_string()))?;

        let request = RequestBuilder::default()
            .principal(principal.uid.clone())
            .action(read_action)
            .unknown_resource_with_type(table_type)
            .context(Context::empty())
            .build();

        let principal_entities = Entities::from_entities([principal.to_entity()], None)
            .unwrap_or_else(|_| Entities::empty());

        let response = match self
            .authorizer
            .is_authorized_partial(&request, &principal_entities)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // Fail closed: if partial evaluation fails we cannot prove what
                // is safe, so deny all rows for this table.
                tracing::warn!(error = %e, table = %table, "partial authorization failed; masking all rows (fail-closed)");
                return Ok(TablePolicy {
                    row_filters: vec![lit(false)],
                    column_masks: Default::default(),
                });
            }
        };

        let translator = CedarResidualTranslator;
        let mut tp = TablePolicy::default();

        for residual in response.nontrivial_residuals() {
            let filter_type = residual.annotation("filter_type").unwrap_or("row_filter");
            // `Ok(Some(e))` -> predicate; `Ok(None)` -> trivially true (no
            // constraint); `Err` -> untranslatable, fail closed.
            let translated = translator.to_predicate(&residual);
            match filter_type {
                "row_filter" => match translated {
                    Ok(Some(pred)) => tp.row_filters.push(pred),
                    Ok(None) => {} // trivially permitted; no filter needed
                    Err(e) => {
                        tracing::warn!(error = %e, table = %table, "untranslatable row filter; denying all rows (fail-closed)");
                        tp.row_filters.push(lit(false));
                    }
                },
                "deny_override" => match translated {
                    // Keep rows where the deny condition does NOT hold.
                    Ok(Some(pred)) => tp.row_filters.push(!pred),
                    Ok(None) => {} // deny condition trivially false; nothing denied
                    Err(e) => {
                        tracing::warn!(error = %e, table = %table, "untranslatable deny_override; denying all rows (fail-closed)");
                        tp.row_filters.push(lit(false));
                    }
                },
                "column_mask" => {
                    // A surviving residual means the mask's exemption condition
                    // is not fully discharged -> mask the column.
                    if let Some(column) = residual.annotation("column") {
                        tp.column_masks.insert(column.to_string(), lit("***"));
                    } else {
                        tracing::warn!(table = %table, "column_mask residual without @column annotation; ignoring (resolver should expand tags to concrete columns)");
                    }
                }
                other => {
                    tracing::warn!(filter_type = other, "unknown filter_type annotation; ignoring");
                }
            }
        }

        Ok(tp)
    }
}
