use std::sync::Arc;

use datafusion::catalog::TableProvider;
use datafusion::error::DataFusionError;
use datafusion::execution::TaskContext;
use datafusion::sql::TableReference;
use datafusion_policy::Posture;
use datafusion_policy_cedar::TableFacts;
use datafusion_unitycatalog::catalog::{ManagedReadState, resolve_managed_read_state};
use datafusion_unitycatalog::catalog::{TableProviderBuilder, TableProviderError};
use unitycatalog_client::UnityCatalogClient;
use unitycatalog_common::models::tables::v1::Table;
use url::Url;

use crate::catalog::policies::{NoPolicyBindingProvider, PolicyBindingProvider};
use crate::catalog::tags::{
    CatalogFactSinkExt, ConventionGovernedTagProvider, ConventionTagProvider, GovernedTagProvider,
    TagProvider, table_acl_facts,
};
use crate::session::TaskExt;

/// Builds Delta [`TableProvider`]s for Unity Catalog tables, backed by the
/// host session's task context.
///
/// The generic resolver in `deltalake-datafusion` handles UC metadata lookup,
/// credential vending, and object store registration; it delegates the actual
/// Delta provider construction here because that requires the log-store and
/// kernel-engine wiring owned by [`crate::session::LakehouseTaskContext`].
///
/// This is also the **resource/catalog PIP seam**: as each table resolves, its
/// catalog facts (owner / readers / writers / classification tags) are gathered
/// from the UC [`Table`] and recorded into the session's
/// [`CatalogFactSink`](datafusion_policy_cedar::CatalogFactSink) (read from the
/// `CatalogFactSinkExt` config extension) so the policy layer can fold them into
/// Cedar evaluation.
pub struct LakehouseTableProviderBuilder {
    ctx: Arc<TaskContext>,
    /// UC client used to call the `/delta/v1` loadTable endpoint, which tells us
    /// whether a table is catalog-managed and, if so, supplies its ratified
    /// commit tail + latest version (the filesystem `_delta_log/` is not
    /// authoritative for managed tables).
    client: UnityCatalogClient,
    tag_provider: Arc<dyn TagProvider>,
    governed_tag_provider: Arc<dyn GovernedTagProvider>,
    /// Sources the UC-ABAC row-filter / column-mask policy bindings for a table
    /// (the `/policies` API). Defaults to [`NoPolicyBindingProvider`] (offline /
    /// deterministic tests); `build_unity_resolver` selects the UC-API provider.
    policy_provider: Arc<dyn PolicyBindingProvider>,
    /// The runtime enforcement posture, threaded from the server's [`Governance`]
    /// profile so [`record_facts`](Self::record_facts) knows whether a policy
    /// sourcing failure / poison must fail closed ([`Posture::Enforcing`]) or be
    /// logged and tolerated (permissive / disabled dry-run).
    posture: Posture,
}

impl LakehouseTableProviderBuilder {
    pub fn new(ctx: Arc<TaskContext>, client: UnityCatalogClient) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            client,
            tag_provider: Arc::new(ConventionTagProvider),
            governed_tag_provider: Arc::new(ConventionGovernedTagProvider),
            policy_provider: Arc::new(NoPolicyBindingProvider),
            // Default to the safe stance; `build_unity_resolver` overrides it from
            // the session's governance profile.
            posture: Posture::Enforcing,
        })
    }

    /// Override the governed-tag provider (the UC ABAC key→value tags feeding the
    /// Cedar mask/filter path). Defaults to [`ConventionGovernedTagProvider`];
    /// `build_unity_resolver` selects the UC-API-backed provider in production.
    pub fn with_governed_tag_provider(
        self: Arc<Self>,
        governed_tag_provider: Arc<dyn GovernedTagProvider>,
    ) -> Arc<Self> {
        Arc::new(Self {
            ctx: self.ctx.clone(),
            client: self.client.clone(),
            tag_provider: self.tag_provider.clone(),
            governed_tag_provider,
            policy_provider: self.policy_provider.clone(),
            posture: self.posture,
        })
    }

    /// Override the policy-binding provider (the UC-ABAC row-filter / column-mask
    /// policies) and the posture governing how a sourcing failure is handled.
    /// `build_unity_resolver` selects the UC-`/policies` provider carrying the
    /// server's [`Governance`] posture in production.
    pub fn with_policy_provider(
        self: Arc<Self>,
        policy_provider: Arc<dyn PolicyBindingProvider>,
        posture: Posture,
    ) -> Arc<Self> {
        Arc::new(Self {
            ctx: self.ctx.clone(),
            client: self.client.clone(),
            tag_provider: self.tag_provider.clone(),
            governed_tag_provider: self.governed_tag_provider.clone(),
            policy_provider,
            posture,
        })
    }

    /// Gather this table's catalog facts and record them into the session's
    /// fact sink, keyed by the table's fully-qualified reference.
    ///
    /// Tag sourcing (classification + governed tags) is **best-effort**: an error
    /// is logged and treated as "no tags" — it must not block resolution; the
    /// reader/writer ACL facts (and the coarse gate) still apply, and the mask
    /// decision stays fail-closed at the policy layer.
    ///
    /// Policy-binding sourcing is **fail-closed**: under [`Posture::Enforcing`] a
    /// `/policies` API error or an unparsable (poisoned) binding errors table
    /// resolution rather than running the query under-constrained. Under a
    /// permissive / disabled dry-run it is logged and resolution proceeds with the
    /// successfully-parsed bindings so the dry run still exercises the path.
    async fn record_facts(&self, table: &Table) -> Result<(), TableProviderError> {
        let Some(ext) = self
            .ctx
            .session_config()
            .get_extension::<CatalogFactSinkExt>()
        else {
            return Ok(());
        };

        let (owner, readers, writers) = table_acl_facts(table);
        let classification = self
            .tag_provider
            .classify(table)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, table = %table.full_name, "tag classification failed; recording no tags");
                Default::default()
            });
        // Governed (key→value) tags for the Cedar mask/filter path. Best-effort:
        // a sourcing error records no governed tags rather than blocking
        // resolution — the mask decision stays fail-closed at the Cedar layer.
        let governed = self
            .governed_tag_provider
            .governed_tags(table)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, table = %table.full_name, "governed-tag sourcing failed; recording none");
                Default::default()
            });

        // Policy bindings (the UC-ABAC row-filter / column-mask policies). Unlike
        // tags, sourcing here is fail-closed under Enforcing (see fn docs).
        let policies = self.policy_bindings(table).await?;

        let table_ref = TableReference::full(
            table.catalog_name.clone(),
            table.schema_name.clone(),
            table.name.clone(),
        );
        ext.0.record(
            table_ref,
            TableFacts {
                owner,
                readers,
                writers,
                tags: classification.table_tags,
                column_tags: classification.column_tags,
                governed_tags: governed.table_tags,
                governed_column_tags: governed.column_tags,
                policies,
            },
        );
        Ok(())
    }

    /// Source the table's policy bindings, applying the fail-closed contract:
    /// under [`Posture::Enforcing`] a sourcing error or a poisoned (partially
    /// unparsable) result errors resolution; otherwise it is logged and the
    /// parsed bindings are returned so a dry run still proceeds.
    async fn policy_bindings(
        &self,
        table: &Table,
    ) -> Result<Vec<datafusion_policy::PolicyBinding>, TableProviderError> {
        let enforcing = self.posture == Posture::Enforcing;
        match self.policy_provider.bindings(table).await {
            Ok(policies) => {
                if policies.poisoned {
                    if enforcing {
                        return Err(DataFusionError::External(
                            format!(
                                "policy sourcing for '{}' produced an unparsable binding; \
                                 refusing to resolve under Enforcing (fail-closed)",
                                table.full_name
                            )
                            .into(),
                        ));
                    }
                    tracing::warn!(
                        table = %table.full_name, posture = ?self.posture,
                        "policy sourcing produced an unparsable binding; proceeding with the \
                         parsed subset (not Enforcing)"
                    );
                }
                Ok(policies.bindings)
            }
            Err(e) if enforcing => Err(DataFusionError::External(
                format!(
                    "policy sourcing for '{}' failed; refusing to resolve under Enforcing \
                     (fail-closed): {e}",
                    table.full_name
                )
                .into(),
            )),
            Err(e) => {
                tracing::warn!(
                    error = %e, table = %table.full_name, posture = ?self.posture,
                    "policy sourcing failed; proceeding with no bindings (not Enforcing)"
                );
                Ok(Vec::new())
            }
        }
    }

    /// Wrap a freshly built provider in a secured view (row filters + column
    /// masks baked into a plan DataFusion inlines) when policy governs this
    /// table, using the facts `record_facts` already recorded into the session's
    /// fact sink. Must run *after* `record_facts` so governed tags / ACLs are in
    /// the sink before `govern_provider_from_config` folds them.
    ///
    /// `govern_provider_from_config` marks the table on the shared
    /// `CatalogFactSink`, so the planner tier skips governing the inlined scan a
    /// second time; an ungoverned table is returned unwrapped. It fails closed
    /// on a constrain error (propagated here).
    async fn govern(
        &self,
        provider: Arc<dyn TableProvider>,
        table: &Table,
    ) -> Result<Arc<dyn TableProvider>, TableProviderError> {
        let table_ref = TableReference::full(
            table.catalog_name.clone(),
            table.schema_name.clone(),
            table.name.clone(),
        );
        datafusion_policy::govern_provider_from_config(
            self.ctx.session_config(),
            provider,
            &table_ref,
        )
        .await
    }
}

impl std::fmt::Debug for LakehouseTableProviderBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LakehouseTableProviderBuilder")
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl TableProviderBuilder for LakehouseTableProviderBuilder {
    async fn build_delta(
        &self,
        location: &Url,
        table: &Table,
    ) -> Result<Arc<dyn TableProvider>, TableProviderError> {
        // Gather catalog facts for this table. Tag facts are best-effort; policy
        // sourcing is fail-closed under Enforcing (an error here aborts the
        // provider build rather than resolving an under-governed table).
        self.record_facts(table).await?;

        // Ask the catalog whether this is a managed (coordinated-commit) table.
        // The `/delta/v1` loadTable response carries the table type and — for a
        // managed table — the unbackfilled commit tail + latest ratified version
        // a reader needs to materialize the catalog's snapshot.
        //
        // Time travel: the catalog-managed seam (`delta_managed_provider_for`)
        // accepts an `at_version`; the SQL `VERSION AS OF` plumbing into this
        // resolution path is a follow-up (see issue), so we pass `None` for now.
        let at_version = None;

        let loaded = match self
            .client
            .delta_v1()
            .load_table(&table.catalog_name, &table.schema_name, &table.name)
            .await
        {
            Ok(loaded) => loaded,
            // A6: `/delta/v1` is unavailable on this deployment (older OSS /
            // production Databricks). The legacy `tables` API already supplied the
            // storage location, so read the filesystem snapshot rather than failing
            // every Delta read. A genuine NoSuchTable / auth / other error
            // propagates (the predicate is the single source of truth, exported
            // from unitycatalog-rs).
            Err(e) if e.should_fall_back_to_legacy() => {
                tracing::debug!(
                    table = %table.full_name, error = %e,
                    "/delta/v1 loadTable unavailable; falling back to filesystem snapshot"
                );
                let provider = self
                    .ctx
                    .lh()
                    .delta_provider_for(location, at_version)
                    .await?;
                return self.govern(provider, table).await;
            }
            Err(e) => return Err(DataFusionError::External(Box::new(e))),
        };

        // Resolve the read state once via the shared helper (no dispatch/fallback
        // logic is duplicated here — it all lives in unitycatalog-rs).
        let provider = match resolve_managed_read_state(&loaded)? {
            // The catalog is the source of truth: build from the ratified commit
            // tail + latest version rather than scanning `_delta_log/`.
            ManagedReadState::Managed { commits, latest } => {
                self.ctx
                    .lh()
                    .delta_managed_provider_for(location, &commits, latest as i64, at_version)
                    .await?
            }
            // External / not-catalog-managed: the filesystem `_delta_log/` is
            // authoritative; the read must not set a catalog version.
            ManagedReadState::NotManaged => {
                self.ctx
                    .lh()
                    .delta_provider_for(location, at_version)
                    .await?
            }
        };
        self.govern(provider, table).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use datafusion::arrow::array::{Int64Array, StringArray};
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::datasource::MemTable;
    use datafusion::execution::config::SessionConfig;
    use datafusion::prelude::SessionContext;
    use datafusion_policy::{
        GovernedTableProvider, PolicyEngineExt, PrincipalExt, PrincipalIdentity,
    };
    use datafusion_policy_cedar::CedarPolicyEngine;

    use super::*;

    /// `record_facts` folds the governed-tag provider's output into the two
    /// governed `TableFacts` fields the Cedar mask path reads. A `Table` carrying
    /// `govtag.*` properties yields a fact record with populated governed tags.
    #[tokio::test]
    async fn record_facts_populates_governed_tags_from_convention() {
        // A task context whose session config carries the fact sink `record_facts`
        // writes into (no network / catalog access is exercised on this path).
        let config = SessionConfig::new().with_extension(Arc::new(CatalogFactSinkExt::default()));
        let ctx = Arc::new(TaskContext::default().with_session_config(config));
        let client =
            UnityCatalogClient::new_unauthenticated(Url::parse("http://localhost:0/").unwrap());
        let builder = LakehouseTableProviderBuilder::new(ctx.clone(), client);

        let table = Table {
            catalog_name: "hr".to_string(),
            schema_name: "security".to_string(),
            name: "employees".to_string(),
            properties: std::collections::HashMap::from([
                ("govtag.ssn.pii".to_string(), "ssn".to_string()),
                (
                    "govtag.classification".to_string(),
                    "restricted".to_string(),
                ),
            ]),
            ..Default::default()
        };

        builder.record_facts(&table).await.expect("record_facts");

        let sink = ctx
            .session_config()
            .get_extension::<CatalogFactSinkExt>()
            .unwrap();
        let facts = sink
            .0
            .get(&TableReference::full("hr", "security", "employees"))
            .expect("facts recorded for the table");
        assert_eq!(
            facts
                .governed_column_tags
                .get("ssn")
                .unwrap()
                .get("pii")
                .unwrap(),
            "ssn"
        );
        assert_eq!(
            facts
                .governed_tags
                .get("classification")
                .map(String::as_str),
            Some("restricted")
        );
    }

    // A minimal Cedar schema + policy: `read_table` is permitted for any
    // principal with a region; the `read_column` forbid masks a column tagged
    // `pii == "ssn"`. Mirrors the mask e2e in `session/mod.rs`, exercised here
    // through the resolver-tier `govern` seam rather than the planner tier.
    const SCHEMA: &str = r#"
        entity User { region: String };
        entity Table { region: String } tags String;
        entity Column in [Table] { name: String } tags String;
        action read_table appliesTo {
            principal: [User], resource: [Table],
            context: { catalog?: String, schema?: String, table?: String, columns?: Set<String> }
        };
        action read_column appliesTo {
            principal: [User], resource: [Column],
            context: { catalog?: String, schema?: String, table?: String }
        };
    "#;
    const POLICY: &str = r#"
        permit (principal, action == Action::"read_table", resource)
        when { principal has region };

        forbid (principal, action == Action::"read_column", resource)
        when { resource.hasTag("pii") && resource.getTag("pii") == "ssn" };
    "#;

    /// Build a `LakehouseTableProviderBuilder` whose task-context config carries
    /// exactly the extensions `create_session_for` attaches for the resolver
    /// tier: the Cedar engine (`PolicyEngineExt`), the principal (`PrincipalExt`),
    /// and the shared fact sink (`CatalogFactSinkExt`).
    fn governed_builder() -> (Arc<LakehouseTableProviderBuilder>, Arc<TaskContext>) {
        let engine = Arc::new(CedarPolicyEngine::from_sources(POLICY, "", Some(SCHEMA)).unwrap());
        let config = SessionConfig::new()
            .with_extension(Arc::new(CatalogFactSinkExt::default()))
            .with_extension(Arc::new(PolicyEngineExt(engine)))
            .with_extension(Arc::new(PrincipalExt(
                PrincipalIdentity::new("User::\"alice\"").with_attribute("region", "eu"),
            )));
        let ctx = Arc::new(TaskContext::default().with_session_config(config));
        let client =
            UnityCatalogClient::new_unauthenticated(Url::parse("http://localhost:0/").unwrap());
        let builder = LakehouseTableProviderBuilder::new(ctx.clone(), client);
        (builder, ctx)
    }

    /// A `(id, ssn)` MemTable standing in for the Delta provider `build_delta`
    /// would construct for a UC-resolved table.
    fn base_provider() -> Arc<dyn TableProvider> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("ssn", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec!["secret-a", "secret-b"])),
            ],
        )
        .unwrap();
        Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap())
    }

    fn table(catalog: &str, schema: &str, name: &str) -> Table {
        Table {
            catalog_name: catalog.to_string(),
            schema_name: schema.to_string(),
            name: name.to_string(),
            ..Default::default()
        }
    }

    /// A stub [`PolicyBindingProvider`] returning a fixed [`TablePolicies`] (to
    /// exercise the poison path) or an error (the API-outage path).
    #[derive(Debug)]
    enum StubPolicies {
        Poisoned,
        Erroring,
    }

    #[async_trait::async_trait]
    impl crate::catalog::policies::PolicyBindingProvider for StubPolicies {
        async fn bindings(
            &self,
            _table: &Table,
        ) -> Result<crate::catalog::policies::TablePolicies, DataFusionError> {
            match self {
                StubPolicies::Poisoned => Ok(crate::catalog::policies::TablePolicies {
                    bindings: Vec::new(),
                    poisoned: true,
                }),
                StubPolicies::Erroring => {
                    Err(DataFusionError::External("policies API unavailable".into()))
                }
            }
        }
    }

    /// A minimal builder whose task-context config carries only the fact sink,
    /// with a stub policy provider selected at `posture`.
    fn builder_with_policies(
        stub: StubPolicies,
        posture: Posture,
    ) -> Arc<LakehouseTableProviderBuilder> {
        let config = SessionConfig::new().with_extension(Arc::new(CatalogFactSinkExt::default()));
        let ctx = Arc::new(TaskContext::default().with_session_config(config));
        let client =
            UnityCatalogClient::new_unauthenticated(Url::parse("http://localhost:0/").unwrap());
        LakehouseTableProviderBuilder::new(ctx, client)
            .with_policy_provider(Arc::new(stub), posture)
    }

    /// Under `Enforcing`, a poisoned (unparsable) binding fails `record_facts`
    /// closed rather than resolving the table under-constrained.
    #[tokio::test]
    async fn poisoned_policy_fails_closed_under_enforcing() {
        let builder = builder_with_policies(StubPolicies::Poisoned, Posture::Enforcing);
        let err = builder
            .record_facts(&table("c", "s", "t"))
            .await
            .expect_err("poison must fail closed under Enforcing");
        assert!(err.to_string().contains("unparsable binding"), "got: {err}");
    }

    /// Under `Permissive`, the same poison is logged and resolution proceeds
    /// (the dry-run stance) — `record_facts` succeeds.
    #[tokio::test]
    async fn poisoned_policy_tolerated_under_permissive() {
        let builder = builder_with_policies(StubPolicies::Poisoned, Posture::Permissive);
        builder
            .record_facts(&table("c", "s", "t"))
            .await
            .expect("permissive tolerates a poisoned binding");
    }

    /// A `/policies` API outage fails `record_facts` closed under `Enforcing`.
    #[tokio::test]
    async fn policies_api_error_fails_closed_under_enforcing() {
        let builder = builder_with_policies(StubPolicies::Erroring, Posture::Enforcing);
        let err = builder
            .record_facts(&table("c", "s", "t"))
            .await
            .expect_err("API error must fail closed under Enforcing");
        assert!(err.to_string().contains("policy sourcing"), "got: {err}");
    }

    /// The same API outage is tolerated under `Disabled` (no enforcement at all).
    #[tokio::test]
    async fn policies_api_error_tolerated_under_disabled() {
        let builder = builder_with_policies(StubPolicies::Erroring, Posture::Disabled);
        builder
            .record_facts(&table("c", "s", "t"))
            .await
            .expect("disabled tolerates a policies API error");
    }

    /// A table whose `ssn` column carries the governed `pii = "ssn"` tag comes
    /// back from `govern` wrapped in a `GovernedTableProvider` (the secured
    /// view), the shared sink is marked governed, and planning a `SELECT` over
    /// the registered provider inlines the secured view — `SubqueryAlias(<table>)`
    /// over a `Projection` that masks `ssn` — proving enforcement lands at
    /// resolution, before the planner tier runs.
    #[tokio::test]
    async fn govern_wraps_resolved_table_in_secured_view() {
        let (builder, ctx) = governed_builder();
        let table = table("hr", "security", "employees");

        // What `record_facts` records after UC resolution: `ssn` tagged `pii`.
        builder
            .ctx
            .session_config()
            .get_extension::<CatalogFactSinkExt>()
            .unwrap()
            .0
            .record(
                TableReference::full("hr", "security", "employees"),
                TableFacts {
                    governed_column_tags: HashMap::from([(
                        "ssn".to_string(),
                        BTreeMap::from([("pii".to_string(), "ssn".to_string())]),
                    )]),
                    ..Default::default()
                },
            );

        let governed = builder.govern(base_provider(), &table).await.unwrap();

        // Wrapped at resolution, and the shared sink marks the table so the
        // planner tier won't govern the inlined scan a second time.
        assert!(
            (governed.as_ref() as &dyn std::any::Any)
                .downcast_ref::<GovernedTableProvider>()
                .is_some(),
            "governed table must come back as a GovernedTableProvider"
        );
        let sink = ctx
            .session_config()
            .get_extension::<CatalogFactSinkExt>()
            .unwrap();
        assert!(
            sink.0
                .is_governed(&TableReference::full("hr", "security", "employees"))
        );

        // Planning a query over the registered provider inlines the secured
        // view: the logical plan carries the `SubqueryAlias` + masking
        // `Projection`, i.e. enforcement landed at resolution.
        let sctx = SessionContext::new();
        sctx.register_table(TableReference::bare("employees"), governed)
            .unwrap();
        let plan = sctx
            .sql("SELECT id, ssn FROM employees")
            .await
            .unwrap()
            .logical_plan()
            .clone();
        let rendered = format!("{}", plan.display_indent());
        assert!(
            rendered.contains("SubqueryAlias"),
            "secured view inlines under a SubqueryAlias: {rendered}"
        );

        // And the mask is real: the raw ssn values never survive execution.
        let batches = sctx
            .sql("SELECT id, ssn FROM employees")
            .await
            .unwrap()
            .collect()
            .await
            .unwrap();
        let out = datafusion::arrow::util::pretty::pretty_format_batches(&batches)
            .unwrap()
            .to_string();
        assert!(
            !out.contains("secret-a") && !out.contains("secret-b"),
            "raw ssn must not leak through the secured view: {out}"
        );
    }

    /// A table with no governed tags / no matching policy is returned unwrapped —
    /// `govern` is a no-op, so the resolver adds no plan-shape overhead for
    /// ungoverned tables.
    #[tokio::test]
    async fn govern_returns_ungoverned_table_unwrapped() {
        let (builder, _ctx) = governed_builder();
        let table = table("hr", "public", "widgets");

        // No facts recorded for this table: the `read_column` forbid can't fire.
        let out = builder.govern(base_provider(), &table).await.unwrap();
        assert!(
            (out.as_ref() as &dyn std::any::Any)
                .downcast_ref::<GovernedTableProvider>()
                .is_none(),
            "ungoverned table must be returned unwrapped (no secured view)"
        );
    }
}
