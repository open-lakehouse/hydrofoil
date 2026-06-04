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
    cedar_policy::{EntityTypeName, RequestBuilder},
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
        let provider = Arc::new(OciPolicyProvider::from_reference(reference).await.map_err(
            |e| {
                datafusion::error::DataFusionError::Plan(format!(
                    "Failed to load Cedar policy from OCI reference '{reference}': {e}"
                ))
            },
        )?);
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

        // Carry the table identity (catalog/schema/table) in the context so
        // row-filter policies can condition on `context.catalog/schema/table`,
        // matching the Layer-1 gate. The resource stays unknown so per-row
        // `resource.<col>` conditions come back as residuals. No columns are
        // listed (governance is per-table, not per-projection).
        let context = crate::visitor::table_context(table, &[])?;

        let request = RequestBuilder::default()
            .principal(principal.uid.clone())
            .action(read_action)
            .unknown_resource_with_type(table_type)
            .context(context)
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
            // `Ok(Some(e))` -> predicate; `Ok(None)` -> trivially true (no
            // constraint); `Err` -> untranslatable, fail closed.
            let translated = translator.to_predicate(&residual);
            // A residual whose intent is unstated or unrecognized is an
            // undischarged condition we cannot enforce -> fail closed.
            match residual.annotation("filter_type") {
                Some("row_filter") => match translated {
                    Ok(Some(pred)) => tp.row_filters.push(pred),
                    Ok(None) => {} // trivially permitted; no filter needed
                    Err(e) => {
                        tracing::warn!(error = %e, table = %table, "untranslatable row filter; denying all rows (fail-closed)");
                        tp.row_filters.push(lit(false));
                    }
                },
                Some("deny_override") => match translated {
                    // Keep rows where the deny condition does NOT hold.
                    Ok(Some(pred)) => tp.row_filters.push(!pred),
                    Ok(None) => {} // deny condition trivially false; nothing denied
                    Err(e) => {
                        tracing::warn!(error = %e, table = %table, "untranslatable deny_override; denying all rows (fail-closed)");
                        tp.row_filters.push(lit(false));
                    }
                },
                Some("column_mask") => {
                    // A surviving residual means the mask's exemption condition
                    // is not fully discharged -> mask the column. A custom mask
                    // value may be supplied via `@mask_value`; default to "***".
                    if let Some(column) = residual.annotation("column") {
                        let mask = residual.annotation("mask_value").unwrap_or("***");
                        tp.column_masks.insert(column.to_string(), lit(mask));
                    } else {
                        // We cannot tell which column to mask, so we cannot
                        // prove the column is protected -> deny all rows.
                        tracing::warn!(table = %table, "column_mask residual without @column annotation; denying all rows (fail-closed; resolver should expand tags to concrete columns)");
                        tp.row_filters.push(lit(false));
                    }
                }
                other => {
                    // Missing (`None`) or unrecognized `filter_type`: the policy
                    // left a condition we cannot classify, so fail closed.
                    tracing::warn!(filter_type = ?other, table = %table, "missing or unknown filter_type annotation on surviving residual; denying all rows (fail-closed)");
                    tp.row_filters.push(lit(false));
                }
            }
        }

        Ok(tp)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;
    use std::sync::Arc;

    use async_trait::async_trait;
    use cedar_local_agent::public::{
        EntityProviderError, PolicySetProviderError, SimpleEntityProvider, SimplePolicySetProvider,
    };
    use cedar_policy::{Entities, EntityUid, PolicySet, Request};
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::logical_expr::logical_plan::builder::table_scan;

    use super::*;
    use crate::principal::PrincipalIdentity;

    /// In-memory provider holding a fixed policy set + entities, for tests.
    #[derive(Debug)]
    struct InMemory {
        policies: Arc<PolicySet>,
    }

    impl InMemory {
        fn new(src: &str) -> Self {
            Self {
                policies: Arc::new(PolicySet::from_str(src).expect("valid policy set")),
            }
        }
    }

    #[async_trait]
    impl SimplePolicySetProvider for InMemory {
        async fn get_policy_set(
            &self,
            _: &Request,
        ) -> Result<Arc<PolicySet>, PolicySetProviderError> {
            Ok(self.policies.clone())
        }
    }

    #[async_trait]
    impl SimpleEntityProvider for InMemory {
        async fn get_entities(&self, _: &Request) -> Result<Arc<Entities>, EntityProviderError> {
            Ok(Arc::new(Entities::empty()))
        }
    }

    /// A policy-set provider that always errors, to exercise fail-closed.
    #[derive(Debug)]
    struct ErrProvider;

    #[async_trait]
    impl SimplePolicySetProvider for ErrProvider {
        async fn get_policy_set(
            &self,
            _: &Request,
        ) -> Result<Arc<PolicySet>, PolicySetProviderError> {
            Err(PolicySetProviderError::General("boom".into()))
        }
    }

    #[async_trait]
    impl SimpleEntityProvider for ErrProvider {
        async fn get_entities(&self, _: &Request) -> Result<Arc<Entities>, EntityProviderError> {
            Ok(Arc::new(Entities::empty()))
        }
    }

    fn policy<P, E>(p: P, e: E) -> CedarPolicy<P, E>
    where
        P: SimplePolicySetProvider + 'static,
        E: SimpleEntityProvider + 'static,
    {
        let config = AuthorizerConfigBuilder::default()
            .policy_set_provider(Arc::new(p))
            .entity_provider(Arc::new(e))
            .build()
            .unwrap();
        CedarPolicy::new(Authorizer::new(config))
    }

    fn alice() -> PrincipalIdentity {
        PrincipalIdentity::new(EntityUid::from_str("User::\"alice\"").unwrap())
            .with_attribute("region", "eu")
    }

    fn scan_plan() -> LogicalPlan {
        let schema = Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("region", DataType::Utf8, true),
        ]);
        table_scan(Some("t"), &schema, None)
            .unwrap()
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn is_allowed_permits_matching_principal() {
        let pol = policy(
            InMemory::new(
                r#"permit(principal == User::"alice", action == Action::"read_table", resource);"#,
            ),
            InMemory::new(""),
        );
        let decision = pol.is_allowed(&scan_plan(), &alice()).await.unwrap();
        assert_eq!(decision, Decision::Allow);
    }

    #[tokio::test]
    async fn is_allowed_denies_non_matching_principal() {
        // Policy only permits bob; alice is denied by default-deny.
        let pol = policy(
            InMemory::new(
                r#"permit(principal == User::"bob", action == Action::"read_table", resource);"#,
            ),
            InMemory::new(""),
        );
        let decision = pol.is_allowed(&scan_plan(), &alice()).await.unwrap();
        assert_eq!(decision, Decision::Deny);
    }

    #[tokio::test]
    async fn is_allowed_fails_closed_on_provider_error() {
        let pol = policy(ErrProvider, ErrProvider);
        let decision = pol.is_allowed(&scan_plan(), &alice()).await.unwrap();
        assert_eq!(decision, Decision::Deny);
    }

    #[cfg(feature = "governance")]
    mod governance {
        use super::*;
        use datafusion::common::DFSchema;
        use datafusion::logical_expr::{col, lit};
        use datafusion::sql::TableReference;

        fn table() -> TableReference {
            TableReference::bare("t")
        }

        fn empty_schema() -> DFSchema {
            DFSchema::empty()
        }

        #[tokio::test]
        async fn row_filter_residual_becomes_predicate() {
            // The exemption is `resource.region == principal.region`; with a
            // concrete principal (region=eu) and unknown resource, the residual
            // is `resource.region == "eu"` -> col("region") == "eu".
            let pol = policy(
                InMemory::new(
                    r#"@filter_type("row_filter")
                       permit(principal, action == Action::"read_table", resource)
                       when { resource.region == principal.region };"#,
                ),
                InMemory::new(""),
            );
            let tp = pol
                .table_policy(&table(), &empty_schema(), &alice())
                .await
                .unwrap();
            assert_eq!(tp.row_filters, vec![col("region").eq(lit("eu"))]);
            assert!(tp.column_masks.is_empty());
        }

        #[tokio::test]
        async fn column_mask_with_column_masks_it() {
            let pol = policy(
                InMemory::new(
                    r#"@filter_type("column_mask")
                       @column("ssn")
                       permit(principal, action == Action::"read_table", resource)
                       when { resource.region == principal.region };"#,
                ),
                InMemory::new(""),
            );
            let tp = pol
                .table_policy(&table(), &empty_schema(), &alice())
                .await
                .unwrap();
            assert_eq!(tp.column_masks.get("ssn"), Some(&lit("***")));
        }

        #[tokio::test]
        async fn column_mask_honors_mask_value() {
            let pol = policy(
                InMemory::new(
                    r#"@filter_type("column_mask")
                       @column("ssn")
                       @mask_value("REDACTED")
                       permit(principal, action == Action::"read_table", resource)
                       when { resource.region == principal.region };"#,
                ),
                InMemory::new(""),
            );
            let tp = pol
                .table_policy(&table(), &empty_schema(), &alice())
                .await
                .unwrap();
            assert_eq!(tp.column_masks.get("ssn"), Some(&lit("REDACTED")));
        }

        #[tokio::test]
        async fn column_mask_without_column_denies_all_rows() {
            // A surviving column_mask residual with no @column cannot be applied,
            // so fail closed (deny all rows), not silently ignored.
            let pol = policy(
                InMemory::new(
                    r#"@filter_type("column_mask")
                       permit(principal, action == Action::"read_table", resource)
                       when { resource.region == principal.region };"#,
                ),
                InMemory::new(""),
            );
            let tp = pol
                .table_policy(&table(), &empty_schema(), &alice())
                .await
                .unwrap();
            assert_eq!(tp.row_filters, vec![lit(false)]);
            assert!(tp.column_masks.is_empty());
        }

        #[tokio::test]
        async fn unknown_filter_type_denies_all_rows() {
            let pol = policy(
                InMemory::new(
                    r#"@filter_type("bogus")
                       permit(principal, action == Action::"read_table", resource)
                       when { resource.region == principal.region };"#,
                ),
                InMemory::new(""),
            );
            let tp = pol
                .table_policy(&table(), &empty_schema(), &alice())
                .await
                .unwrap();
            assert_eq!(tp.row_filters, vec![lit(false)]);
        }

        #[tokio::test]
        async fn missing_filter_type_denies_all_rows() {
            // No @filter_type annotation on a surviving residual -> fail closed.
            let pol = policy(
                InMemory::new(
                    r#"permit(principal, action == Action::"read_table", resource)
                       when { resource.region == principal.region };"#,
                ),
                InMemory::new(""),
            );
            let tp = pol
                .table_policy(&table(), &empty_schema(), &alice())
                .await
                .unwrap();
            assert_eq!(tp.row_filters, vec![lit(false)]);
        }
    }
}
