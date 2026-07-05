use std::{any::Any, collections::HashMap, sync::Arc};

use arrow::array::RecordBatch;
use datafusion::{
    catalog::{
        AsyncCatalogProviderList as _, CatalogProvider as _, MemoryCatalogProvider, Session,
    },
    common::DFSchema,
    config::{ConfigOptions, TableOptions},
    datasource::{TableProvider, provider_as_source},
    error::Result,
    execution::{
        SessionState, SessionStateBuilder, TaskContext, context::ExecutionProps,
        runtime_env::RuntimeEnv,
    },
    logical_expr::{
        AggregateUDF, Extension, HigherOrderUDF, LogicalPlan, LogicalPlanBuilder, ScalarUDF,
        WindowUDF, registry::ExtensionTypeRegistryRef,
    },
    physical_plan::{ExecutionPlan, PhysicalExpr},
    prelude::{DataFrame, Expr, SessionConfig, SessionContext},
};
use datafusion_openlineage::{OpenLineageClient, OpenLineageConfig};
use datafusion_policy_cedar::{
    EvalContextProvider as _, PrincipalIdentity, SessionConfigEvalContextProvider,
};
use datafusion_tracing::{
    InstrumentationOptions, instrument_with_info_spans, pretty_format_compact_batch,
};
use delta_kernel::Version;
use deltalake_core::{DeltaTableConfig, delta_datafusion::DeltaScanNext, kernel::Snapshot};
use deltalake_core::{
    Path,
    delta_datafusion::engine::AsObjectStoreUrl as _,
    logstore::{LogStore, StorageConfig, logstore_with},
};
use deltalake_core::{delta_datafusion::engine::DataFusionEngine, logstore::LogStoreConfig};
use instrumented_object_store::instrument_object_store;
use object_store::{aws::AmazonS3Builder, client::SpawnedReqwestConnector, prefix::PrefixStore};
use tokio::runtime::Handle;
use tracing::{instrument, warn};
use url::Url;
use uuid::Uuid;

use datafusion_unitycatalog::catalog::{UnityCatalogProviderList, build_catalog_managed_snapshot};
use deltalake_datafusion::sql::{UnityCatalogPlanner, UnityClientExtension, UnityFactoryExt};
use unitycatalog_common::models::delta::v1::DeltaCommit;
use unitycatalog_object_store::UnityObjectStoreFactory;

use datafusion_openlineage::context::LineageContext;

use crate::{
    catalog::{DeltaTableFactory, LakehouseSchemaProvider, LakehouseTableProviderBuilder},
    lineage::LineageContextExt,
    policy::Policy,
};

mod log_store;

use log_store::DataFusionLogStore;

// The taint-ledger `SessionConfig` extension lives in `datafusion-policy-cedar`
// (the crate owns its per-request seam), alongside the `SessionConfigEvalContextProvider`
// that assembles the `EvalContext` from these extensions. Re-export it so
// `crate::engine` keeps populating it via `crate::session::FactStoreExt`.
pub(crate) use datafusion_policy_cedar::FactStoreExt;

/// Context for executing queries in a Lakehouse.
///
/// This context is used to execute queries in a Lakehouse. It contains the
/// underlying DataFusion session state, the policy used to enforce access
/// control during query planning and execution, and the principal (user or
/// service) on behalf of whom queries are executed.
#[derive(Clone)]
pub struct LakehouseCtx {
    /// The underlying DataFusion session state that manages most of the execution context.
    inner: SessionContext,
    /// The policy used to enforce access control during query planning and execution.
    policy: Arc<dyn Policy>,
    /// The principal (user or service) on behalf of whom queries are executed.
    principal: PrincipalIdentity,
    /// Optional Unity Catalog resolver used to back catalog/schema/table
    /// resolution with a live Unity Catalog instance during planning.
    unity: Option<Arc<UnityCatalogProviderList>>,
}

impl LakehouseCtx {
    pub fn new(
        inner: SessionContext,
        policy: Arc<dyn Policy>,
        principal: PrincipalIdentity,
    ) -> Self {
        Self {
            inner,
            policy,
            principal,
            unity: None,
        }
    }

    /// Attach a Unity Catalog resolver so qualified `catalog.schema.table`
    /// references resolve against a live Unity Catalog instance.
    pub fn with_unity(mut self, unity: Arc<UnityCatalogProviderList>) -> Self {
        self.unity = Some(unity);
        self
    }

    pub fn ctx(&self) -> &SessionContext {
        &self.inner
    }

    /// Resolve `table_ref` to a [`TableProvider`], registering its Unity Catalog
    /// catalog onto the session's catalog list first when a UC resolver is
    /// wired. The statement path registers UC catalogs lazily inside
    /// `create_logical_plan`; the ingest/write path does not parse SQL, so it
    /// must trigger the same registration before `table_provider` can resolve a
    /// `catalog.schema.table` reference.
    pub async fn resolve_table_provider(
        &self,
        table_ref: datafusion::sql::TableReference,
    ) -> Result<Arc<dyn TableProvider>> {
        if let Some(unity) = self.unity.as_ref() {
            let resolved = unity
                .resolve(
                    std::slice::from_ref(&table_ref),
                    self.inner.state().config(),
                )
                .await?;
            let state = self.inner.state();
            for name in resolved.catalog_names() {
                if let Some(catalog) = resolved.catalog(&name) {
                    state.catalog_list().register_catalog(name, catalog);
                }
            }
        }
        self.inner.table_provider(table_ref).await
    }

    pub fn session(&self) -> LakehouseSession {
        LakehouseSession {
            inner: self.inner.state(),
            unity: self.unity.clone(),
        }
    }

    /// Build a [`LakehouseSession`] for a single request, attaching the
    /// per-request OpenLineage [`LineageContext`] (parsed from the request's
    /// parent-run headers) to the session's `SessionConfig`.
    ///
    /// Parent-run context is per-request, so it is attached to a fresh
    /// `SessionState` clone here rather than baked into the long-lived,
    /// per-principal cached `LakehouseCtx` (which would otherwise pin the first
    /// caller's parent run onto every later request). The
    /// `HydrofoilContextProvider` reads it back at planning time via
    /// `SessionConfig::get_extension`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn session_with_lineage(&self, context: LineageContext) -> LakehouseSession {
        let mut inner = self.inner.state();
        inner
            .config_mut()
            .set_extension(Arc::new(LineageContextExt(context)));
        LakehouseSession {
            inner,
            unity: self.unity.clone(),
        }
    }

    pub async fn execute_logical_plan(&self, plan: LogicalPlan) -> Result<DataFrame> {
        // This path (ingest / delta-connect) executes on the inner
        // `SessionContext`, which does NOT route through
        // `LakehouseSession::create_physical_plan`, so it must enforce the gate
        // itself. Inject fine-grained governance (row filters + column masks)
        // before optimization, then gate on the optimized plan to match the
        // statement path's contract (projections/filters pushed down first).
        // Catalog facts (resource folding) come from the session's fact sink;
        // the taint ledger is absent on this ingest/delta-connect path (it is a
        // write path, and taint recording tracks reads at the statement path).
        let governed = self.authorize_logical_plan(plan).await?;
        self.inner.execute_logical_plan(governed).await
    }

    /// Run the access-control gate over `plan` without executing it, returning
    /// the governed plan on `Allow` (or an authorization error on `Deny`).
    ///
    /// This is the gate half of [`Self::execute_logical_plan`], factored out so
    /// write paths that bypass DataFusion execution — notably the managed-table
    /// bulk-ingest append, which writes through the Unity Catalog committer
    /// instead of a physical plan — authorize under the identical Cedar policy a
    /// regular `INSERT INTO … Append` would face. The caller passes the same
    /// insert plan it would otherwise execute; a writer authorized for an
    /// external table is thus authorized for a managed one under one policy.
    pub async fn authorize_logical_plan(&self, plan: LogicalPlan) -> Result<LogicalPlan> {
        // Same enforcement sequence the `PolicyQueryPlanner` runs on the statement
        // path (govern → optimize → gate), through the one shared implementation.
        // This write path executes on the inner `SessionContext` and so does not
        // route through the policy planner; it enforces the identical policy here.
        let state = self.inner.state();
        let eval = SessionConfigEvalContextProvider.eval_context(&state).await;
        datafusion_policy_cedar::authorize_and_govern(
            &state,
            &plan,
            self.policy.as_ref(),
            &self.principal,
            &eval,
        )
        .await
    }
}

/// Custom session providing Lakehouse specific features on top of DataFusion's [`SessionState`].
///
/// This is the main entry point for query execution and planning, and is where we will
/// enforce policies and manage state related to the lakehouse.
#[derive(Debug, Clone)]
pub struct LakehouseSession {
    /// The underlying DataFusion session state that manages most of the execution context.
    ///
    /// Access-control enforcement (govern + gate) now lives in the session's
    /// `PolicyQueryPlanner`, installed on this state when the session was built;
    /// `create_physical_plan` delegates through it. So the session no longer
    /// carries the policy/principal itself — they reach the planner via the
    /// `SessionConfig` extensions and the crate's default providers.
    inner: SessionState,
    /// Optional Unity Catalog resolver used to back catalog/schema/table
    /// resolution during planning.
    unity: Option<Arc<UnityCatalogProviderList>>,
}

impl LakehouseSession {
    /// Construct a session directly from its parts.
    ///
    /// The per-connection [`crate::engine::Session`] uses this to build a
    /// per-query session from a (cheaply cloned) `SessionState` it has already
    /// decorated with per-request `SessionConfig` extensions (lineage / agent
    /// context) and the policy planner.
    pub fn new(inner: SessionState, unity: Option<Arc<UnityCatalogProviderList>>) -> Self {
        Self { inner, unity }
    }

    pub async fn create_logical_plan(
        &self,
        query: impl AsRef<str> + Send + 'static,
    ) -> Result<LogicalPlan> {
        let query = query.as_ref();

        // Recognize Unity Catalog DDL (`CREATE`/`DROP CATALOG`/`SCHEMA`) up
        // front via hydrofoil's custom parser. Such statements are lowered to an
        // `ExecuteUnityCatalogPlanNode` extension that runs against the live UC
        // client at execution time; authorization is the Cedar gate's job (the
        // node is an `Extension`, deliberately authorized in `create_physical_plan`,
        // not waved through the `SQLOptions` DDL gate). Everything else falls
        // through to DataFusion's planner.
        if let Some(plan) = self.try_plan_unity_ddl(query)? {
            return Ok(plan);
        }

        let Some(unity) = self.unity.as_ref() else {
            return self.inner.create_logical_plan(query).await;
        };

        // Resolve any Unity Catalog references in the query once, before
        // planning. UC catalogs are overlaid onto the live catalog list (which
        // keeps the local `hydrofoil.default` catalog); unknown references fall
        // through to existing catalogs.
        let dialect = self.inner.config().options().sql_parser.dialect;
        let statement = self.inner.sql_to_statement(query, &dialect)?;
        let references = self.inner.resolve_table_references(&statement)?;
        let resolved = unity.resolve(&references, self.inner.config()).await?;

        let state = self.inner.clone();
        for name in resolved.catalog_names() {
            if let Some(catalog) = resolved.catalog(&name) {
                state.catalog_list().register_catalog(name, catalog);
            }
        }
        state.statement_to_plan(statement).await
    }

    /// If `query` is a single Unity Catalog DDL statement, lower it to an
    /// `ExecuteUnityCatalogPlanNode` extension plan; otherwise return `None` so
    /// the caller falls through to DataFusion's planner.
    ///
    /// Only the leading keywords hydrofoil's parser recognizes (`CREATE`/`DROP
    /// CATALOG`/`SCHEMA`) produce a UC statement; for anything else `HFParser`
    /// delegates to DataFusion and we hand control back. A parse error here is
    /// also handed back so DataFusion produces the canonical diagnostic.
    fn try_plan_unity_ddl(&self, query: &str) -> Result<Option<LogicalPlan>> {
        use deltalake_datafusion::sql::{ExecuteUnityCatalogPlanNode, HFParser, Statement};

        let Ok(mut statements) = HFParser::parse_sql(query) else {
            return Ok(None);
        };
        // A UC DDL request is exactly one statement; multi-statement or non-UC
        // input is not ours to plan.
        if statements.len() != 1 {
            return Ok(None);
        }
        match statements.pop_front() {
            Some(Statement::UnityCatalog(statement)) => {
                let node = ExecuteUnityCatalogPlanNode { statement };
                Ok(Some(LogicalPlan::Extension(Extension {
                    node: Arc::new(node),
                })))
            }
            _ => Ok(None),
        }
    }
}

#[async_trait::async_trait]
impl Session for LakehouseSession {
    fn session_id(&self) -> &str {
        self.inner.session_id()
    }

    fn config(&self) -> &SessionConfig {
        self.inner.config()
    }

    fn config_options(&self) -> &ConfigOptions {
        self.config().options()
    }

    async fn create_physical_plan(
        &self,
        logical_plan: &LogicalPlan,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        // Per-query agent / governance context, if the request carried any. For
        // now we only observe it; it is the seam the deferred agent PEP / taint
        // ledger will read (see docs/adr/0005-per-query-agent-governance-context.md).
        if let Some(agent) = self
            .inner
            .config()
            .get_extension::<crate::agent::AgentContextExt>()
        {
            let agent = &agent.0;
            tracing::info!(
                agent.id = agent.agent_id.as_deref().unwrap_or(""),
                agent.session = agent.agent_session.as_deref().unwrap_or(""),
                agent.task = agent.task.as_deref().unwrap_or(""),
                "executing query with agent context"
            );
        }

        // Governance (row filters + column masks) and the coarse access gate now
        // run inside the session's `PolicyQueryPlanner` (installed by the engine),
        // which the inner `SessionState` routes through here. So this just
        // delegates: the planner extracts the per-query facts + principal from the
        // session extensions, governs and gates, then hands off to physical
        // planning (composing with the lineage planner when one is installed).
        self.inner.create_physical_plan(logical_plan).await
    }

    fn create_physical_expr(
        &self,
        expr: Expr,
        df_schema: &DFSchema,
    ) -> Result<Arc<dyn PhysicalExpr>> {
        self.inner.create_physical_expr(expr, df_schema)
    }

    fn scalar_functions(&self) -> &HashMap<String, Arc<ScalarUDF>> {
        self.inner.scalar_functions()
    }

    fn aggregate_functions(&self) -> &HashMap<String, Arc<AggregateUDF>> {
        self.inner.aggregate_functions()
    }

    fn window_functions(&self) -> &HashMap<String, Arc<WindowUDF>> {
        self.inner.window_functions()
    }

    fn higher_order_functions(&self) -> &HashMap<String, Arc<HigherOrderUDF>> {
        self.inner.higher_order_functions()
    }

    fn extension_type_registry(&self) -> &ExtensionTypeRegistryRef {
        self.inner.extension_type_registry()
    }

    fn runtime_env(&self) -> &Arc<RuntimeEnv> {
        self.inner.runtime_env()
    }

    fn execution_props(&self) -> &ExecutionProps {
        self.inner.execution_props()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn table_options(&self) -> &TableOptions {
        self.inner.table_options()
    }

    fn default_table_options(&self) -> TableOptions {
        self.inner.default_table_options()
    }

    fn table_options_mut(&mut self) -> &mut TableOptions {
        self.inner.table_options_mut()
    }

    fn task_ctx(&self) -> Arc<TaskContext> {
        self.inner.task_ctx()
    }
}

pub struct LakehouseTaskContext {
    inner: Arc<TaskContext>,
}

impl LakehouseTaskContext {
    pub(crate) fn delta_logstore_for(&self, location: &Url) -> Result<Arc<dyn LogStore>> {
        let object_store_url = location.as_object_store_url();
        let root_store = self.inner.runtime_env().object_store(object_store_url)?;
        let table_path = Path::from_url_path(location.path())?;
        let prefixed_store = Arc::new(PrefixStore::new(root_store.clone(), table_path));
        let storage_config = StorageConfig::default();
        Ok(
            logstore_with(root_store.clone(), location, storage_config.clone()).unwrap_or_else(
                |_| {
                    warn!(
                        "No registered log store factory for scheme '{}'. Using default.",
                        location.scheme()
                    );
                    DataFusionLogStore::new(
                        prefixed_store,
                        root_store,
                        LogStoreConfig::new(location, storage_config),
                        self.inner.clone(),
                    )
                },
            ),
        )
    }

    pub(crate) async fn delta_snapshot_for(
        &self,
        location: &Url,
        version: Option<Version>,
    ) -> Result<Arc<Snapshot>> {
        let engine = DataFusionEngine::new_from_context(self.inner.clone());

        let snapshot = Snapshot::try_new_with_engine(
            engine,
            location.clone(),
            DeltaTableConfig {
                // since we are not going through the eager snapshot, this should
                // not matter, but we set it to false to avoid any surprises
                require_files: false,
                // This might still be used somewhere but should have no effect
                // when also using the datafusion engine.
                log_buffer_size: 10,
                // also should not matter here
                log_batch_size: self.inner.session_config().options().execution.batch_size,
                // we are setting the spawning service when we build the objects stores,
                // and should not additionally integrate with this layer.
                io_runtime: None,
                // should not matter here as we go through the datafusion engine.
                skip_stats: false,
            },
            version,
        )
        .await?;

        Ok(snapshot.into())
    }

    /// Build a Delta [`TableProvider`] for the table rooted at `location`.
    ///
    /// Resolves the log store and snapshot from the current task context and
    /// returns a [`DeltaScanNext`] provider. The object store backing
    /// `location` must already be registered on the runtime (e.g. via the
    /// Unity Catalog routing store) so reads succeed at scan time.
    #[instrument(
        skip(self),
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_TASK,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    pub async fn delta_provider_for(
        &self,
        location: &Url,
        version: Option<Version>,
    ) -> Result<Arc<dyn TableProvider>> {
        let log_store = self.delta_logstore_for(location)?;
        let snapshot = self.delta_snapshot_for(location, version).await?;

        let provider = DeltaScanNext::builder()
            .with_snapshot(snapshot)
            .with_log_store(log_store)
            .await?;

        Ok(provider)
    }

    /// Build a Delta [`TableProvider`] for a **catalog-managed** (coordinated-
    /// commit) table, where the catalog — not `_delta_log/` — is the source of
    /// truth for the latest version.
    #[instrument(
        skip(self, commits),
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_TASK,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    pub async fn delta_managed_provider_for(
        &self,
        location: &Url,
        commits: &[DeltaCommit],
        latest: i64,
        at_version: Option<Version>,
    ) -> Result<Arc<dyn TableProvider>> {
        let log_store = self.delta_logstore_for(location)?;
        let engine = DataFusionEngine::new_from_context(self.inner.clone());
        let snapshot =
            build_catalog_managed_snapshot(engine.as_ref(), location, commits, latest, at_version)?;

        let provider = DeltaScanNext::builder()
            .with_snapshot(Arc::new(snapshot))
            .with_log_store(log_store)
            .await?;

        Ok(provider)
    }

    /// Build a logical scan of a Delta table by object-store URL (optionally at a
    /// specific version). Retained as a seam for direct-by-location scans; the
    /// catalog path resolves tables by name instead, so this isn't wired in yet.
    #[allow(dead_code)]
    #[instrument(
        skip(self),
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_TASK,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    pub async fn scan_delta_table(
        &self,
        location: &Url,
        version: Option<Version>,
    ) -> Result<LogicalPlan> {
        let snapshot = self.delta_snapshot_for(location, version).await?;

        let table_name = snapshot
            .metadata()
            .name()
            .map(|n| n.to_string())
            .unwrap_or_else(|| format!("__delta__{}", snapshot.metadata().id()));

        let provider = self.delta_provider_for(location, version).await?;

        LogicalPlanBuilder::scan(table_name, provider_as_source(provider), None)?.build()
    }
}

pub trait TaskExt {
    fn lh(&self) -> LakehouseTaskContext;
}

impl TaskExt for Arc<TaskContext> {
    fn lh(&self) -> LakehouseTaskContext {
        LakehouseTaskContext {
            inner: self.clone(),
        }
    }
}

impl TaskExt for SessionState {
    fn lh(&self) -> LakehouseTaskContext {
        LakehouseTaskContext {
            inner: self.task_ctx(),
        }
    }
}

impl TaskExt for SessionContext {
    fn lh(&self) -> LakehouseTaskContext {
        LakehouseTaskContext {
            inner: self.task_ctx(),
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn create_session(
    session_id: impl Into<Option<Uuid>>,
    lineage: Option<OpenLineageClient>,
    lineage_config: OpenLineageConfig,
) -> Result<SessionContext> {
    // A disabled-posture governance profile: no enforcement installed (the
    // allow-all analog of the former `StaticPolicy(Allow)`), for the standalone /
    // metadata-only session path.
    let governance = datafusion_policy::Governance::builder()
        .posture(datafusion_policy::Posture::Disabled)
        .build()
        .expect("disabled governance builds without an engine");
    create_session_for(
        session_id,
        lineage,
        lineage_config,
        None,
        None,
        &governance,
        &crate::config::ObjectStoreConfig::default(),
    )
}

/// Build a session context, optionally binding a [`PrincipalIdentity`] into the
/// session config so planning-time providers can read it back.
///
/// Each call constructs its own `RuntimeEnv` and registers the seaweedfs object
/// store on it — so per-connection sessions are isolated (vended Unity Catalog
/// credentials registered on one session's runtime cannot leak to another). See
/// `docs/adr/0004-per-session-credential-isolation.md`.
pub fn create_session_for(
    session_id: impl Into<Option<Uuid>>,
    lineage: Option<OpenLineageClient>,
    lineage_config: OpenLineageConfig,
    principal: Option<PrincipalIdentity>,
    unity_factory: Option<Arc<UnityObjectStoreFactory>>,
    governance: &Arc<datafusion_policy::Governance>,
    object_store: &crate::config::ObjectStoreConfig,
) -> Result<SessionContext> {
    let options = InstrumentationOptions::builder()
        .record_metrics(true)
        .preview_limit(5)
        .preview_fn(Arc::new(|batch: &RecordBatch| {
            pretty_format_compact_batch(batch, 64, 3, 10).map(|fmt| fmt.to_string())
        }))
        .build();

    let instrument_rule = instrument_with_info_spans!(
        options: options,
    );

    let mut session_config = SessionConfig::from_env()?
        .with_information_schema(true)
        .with_default_catalog_and_schema("hydrofoil", "default");

    // Always attach a catalog fact sink, so `build_delta` (during resolution) has
    // somewhere to write gathered facts even on the no-principal / metadata-only
    // path. When a principal is present, `governance.attach` overwrites this with
    // its own fresh per-session sink (and adds the principal + engine + fact-store
    // extensions in the same call) — the fresh sink still wins because it is set
    // last.
    session_config.set_extension(Arc::new(
        datafusion_policy_cedar::CatalogFactSinkExt::default(),
    ));

    // Attach ALL the enforcement-layer `SessionConfig` extensions in one call:
    // the principal (`PrincipalExt`), a fresh per-session `CatalogFactSinkExt`
    // (so `build_delta`'s gathered facts and the policy layer share one sink),
    // the `PolicyEngineExt` (the engine the secured-view resolver seam
    // `govern_provider_from_config` discovers — the same engine the composed
    // planner receives below), and the `FactStoreExt` / any profile
    // `FunctionResolverExt`. `attach` takes the principal as a required argument,
    // so "forgot to bind the principal" is unrepresentable here. With no
    // principal (metadata-only paths / tests) skip it; the planner fails closed
    // under Enforcing if a principal is genuinely absent downstream.
    if let Some(principal) = principal {
        session_config = governance.attach(session_config, principal);
    }

    // When Unity Catalog is wired, make its client available to the physical
    // planner so `CREATE`/`DROP CATALOG`/`SCHEMA` DDL can execute against it.
    // The factory is also exposed (carrying per-table credential vending) so the
    // managed-table write paths — bulk-ingest append and managed `CREATE TABLE` —
    // can stage and publish commits through the catalog committer.
    if let Some(factory) = unity_factory.as_ref() {
        session_config.set_extension(Arc::new(UnityClientExtension(
            factory.unity_client().clone(),
        )));
        session_config.set_extension(Arc::new(UnityFactoryExt(factory.clone())));

        // Resolve policy-named mask functions (`@mask_fn`) against the UC
        // Functions API. This is inherently *per-session* (it binds this session's
        // UC factory, which may carry a per-user token), so it overrides any
        // profile-level `FunctionResolverExt` `governance.attach` set —
        // `SessionConfigEvalContextProvider` reads it into
        // `EvalContext.function_resolver`; a name that fails to resolve masks the
        // column with the default literal (fail-closed at the policy layer).
        session_config.set_extension(Arc::new(datafusion_policy_cedar::FunctionResolverExt(
            Arc::new(crate::catalog::UnityFunctionResolver::new(
                factory.unity_client().clone(),
            )),
        )));
    }

    let policy = governance.engine().clone();

    // The single session query planner composes policy + lineage over one shared
    // physical planner (see `GovernedLineagePlanner`): it governs + gates first
    // (a denied query errors here, emitting no lineage), then — if lineage is
    // wired — emits START and wraps the governed plan in a `LineageMarker`, then
    // delegates lowering to one `DefaultPhysicalPlanner` carrying both the Unity
    // DDL lowerer (`UnityCatalogPlanner`, for `ExecuteUnityCatalogPlanNode`) and
    // the `LineageExtensionPlanner`. The principal + per-query facts are read from
    // the `SessionConfig` extensions attached above via the crate's default
    // providers; the lineage context provider reads the per-request `LineageContext`
    // the server attaches via `LakehouseCtx::session_with_lineage`.
    let query_planner: Arc<dyn datafusion::execution::context::QueryPlanner + Send + Sync> =
        Arc::new(crate::planner::GovernedLineagePlanner::new(
            policy,
            governance.posture(),
            Arc::new(datafusion_policy_cedar::SessionConfigPrincipalProvider),
            Arc::new(datafusion_policy_cedar::SessionConfigEvalContextProvider),
            vec![Arc::new(UnityCatalogPlanner)],
            lineage.map(|client| {
                (
                    client,
                    Arc::new(crate::lineage::HydrofoilContextProvider) as Arc<_>,
                    lineage_config,
                )
            }),
        ));

    let session_state = SessionStateBuilder::new_with_default_features()
        .with_session_id(session_id.into().unwrap_or_else(Uuid::new_v4).to_string())
        .with_config(session_config)
        .with_query_planner(query_planner)
        .with_table_factory(
            DeltaTableFactory::FILE_FORMAT.to_string(),
            DeltaTableFactory::instance(),
        )
        .with_physical_optimizer_rule(instrument_rule)
        .build();

    let ctx = SessionContext::new_with_state(session_state);

    update_session(&ctx.state(), object_store)?;

    let catalog = MemoryCatalogProvider::new();
    catalog.register_schema(
        "default",
        LakehouseSchemaProvider::new(Arc::new(ctx.state())),
    )?;
    ctx.register_catalog("hydrofoil", Arc::new(catalog));

    Ok(ctx)
}

/// Build a Unity Catalog resolver for `ctx`, backed by `factory`.
///
/// The resolver looks tables up in Unity Catalog, vends credentials, and
/// registers per-table object stores on `ctx`'s runtime. Delta provider
/// construction is delegated to a [`LakehouseTableProviderBuilder`] over the
/// session's task context.
pub fn build_unity_resolver(
    ctx: &SessionContext,
    factory: Arc<UnityObjectStoreFactory>,
    posture: datafusion_policy::Posture,
) -> Arc<UnityCatalogProviderList> {
    let runtime = ctx.runtime_env();
    let builder =
        LakehouseTableProviderBuilder::new(ctx.task_ctx(), factory.unity_client().clone())
            // Source governed (key→value) tags from the UC entity-tag-assignments API —
            // the real ABAC governed-tag source, reachable via the same client. Offline /
            // deterministic tests keep the convention provider (the builder's default).
            .with_governed_tag_provider(Arc::new(crate::catalog::UnityGovernedTagProvider::new(
                factory.unity_client().clone(),
            )))
            // Source the UC-ABAC row-filter / column-mask policy bindings from the
            // `/policies` API (`include_inherited=true`). Carries the server's
            // enforcement posture so a sourcing failure fails closed under Enforcing.
            .with_policy_provider(
                Arc::new(crate::catalog::UnityPolicyBindingProvider::new(
                    factory.unity_client().clone(),
                )),
                posture,
            );
    Arc::new(UnityCatalogProviderList::new(factory, runtime, builder))
}

fn update_session(
    session: &dyn Session,
    object_store: &crate::config::ObjectStoreConfig,
) -> Result<()> {
    register_object_store(session, Handle::current(), object_store)?;
    Ok(())
}

/// Register the configured S3-compatible object store on the session's runtime,
/// keyed by `s3://<bucket>/`. Defaults reproduce the historical seaweedfs dev
/// gateway (see [`crate::config::ObjectStoreConfig`]); a deployment overrides
/// them via `[object-store]`.
fn register_object_store(
    session: &dyn Session,
    handle: Handle,
    cfg: &crate::config::ObjectStoreConfig,
) -> Result<()> {
    let object_store = Arc::new(
        AmazonS3Builder::new()
            .with_http_connector(SpawnedReqwestConnector::new(handle))
            .with_access_key_id(&cfg.access_key_id)
            .with_secret_access_key(&cfg.secret_access_key)
            .with_endpoint(&cfg.endpoint)
            .with_bucket_name(&cfg.bucket)
            .with_allow_http(cfg.allow_http)
            .build()?,
    );
    let instrumented = instrument_object_store(object_store, "object-store");
    let url = url::Url::parse(&format!("s3://{}/", cfg.bucket))
        .map_err(|e| datafusion::error::DataFusionError::External(Box::new(e)))?;
    session
        .runtime_env()
        .register_object_store(&url, instrumented);
    Ok(())
}

#[cfg(test)]
mod integration_tests {
    //! End-to-end tests over `LakehouseCtx`/`LakehouseSession` — the govern →
    //! optimize → gate pipeline and OpenLineage emission — without standing up a
    //! Flight SQL socket. The transport seam is exercised directly here; the
    //! Flight layer is thin and covered by upstream `arrow-flight`.
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use datafusion::arrow::array::{Int64Array, StringArray};
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use datafusion::arrow::record_batch::RecordBatch;
    use datafusion::datasource::MemTable;
    use datafusion::logical_expr::LogicalPlan;

    use datafusion_openlineage::OpenLineageClient;
    use datafusion_openlineage::event::{RunEvent, RunEventType};
    use datafusion_openlineage::transport::{Transport, TransportError};
    use datafusion_policy_cedar::{Decision, EvalContext, PrincipalIdentity};

    use super::*;
    use crate::policy::{Policy, StaticPolicy};

    fn principal(name: &str) -> PrincipalIdentity {
        PrincipalIdentity::new(format!("User::\"{name}\"")).with_attribute("region", "eu")
    }

    /// An `Enforcing` governance profile wrapping `policy` — the test analog of
    /// the production profile, so `create_session_for` / `Engine::new` enforce the
    /// given engine on the statement path.
    fn governance_for(policy: Arc<dyn Policy>) -> Arc<datafusion_policy::Governance> {
        datafusion_policy::Governance::builder()
            .posture(datafusion_policy::Posture::Enforcing)
            .engine(policy)
            .build()
            .expect("enforcing governance with an engine builds")
    }

    async fn ctx_with_table(
        policy: Arc<dyn Policy>,
        lineage: Option<OpenLineageClient>,
    ) -> LakehouseCtx {
        ctx_with_table_at_posture(policy, datafusion_policy::Posture::Enforcing, lineage).await
    }

    async fn ctx_with_table_at_posture(
        policy: Arc<dyn Policy>,
        posture: datafusion_policy::Posture,
        lineage: Option<OpenLineageClient>,
    ) -> LakehouseCtx {
        // Build the session *with* the policy so its `PolicyQueryPlanner` enforces
        // it on the statement path (mirroring `Engine::new_session`), and bind the
        // principal so the planner's default provider can resolve it. The same
        // policy is also handed to `LakehouseCtx` for the write path's explicit gate.
        let governance = datafusion_policy::Governance::builder()
            .posture(posture)
            .engine(policy.clone())
            .build()
            .expect("governance with an engine builds");
        let session = create_session_for(
            None,
            lineage,
            OpenLineageConfig::default(),
            Some(principal("alice")),
            None,
            &governance,
            &crate::config::ObjectStoreConfig::default(),
        )
        .unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("region", DataType::Utf8, true),
            Field::new("ssn", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec!["eu", "us", "eu"])),
                Arc::new(StringArray::from(vec!["a", "b", "c"])),
            ],
        )
        .unwrap();
        let table = MemTable::try_new(schema, vec![vec![batch]]).unwrap();
        session.register_table("t", Arc::new(table)).unwrap();
        LakehouseCtx::new(session, policy, principal("alice"))
    }

    #[tokio::test]
    async fn session_attaches_a_readable_catalog_fact_sink() {
        // `create_session_for` attaches a `CatalogFactSinkExt` so `build_delta`
        // (during resolution) and the policy layer share one sink. Assert it is
        // present on the session state and round-trips facts.
        let session = create_session(None, None, OpenLineageConfig::default()).unwrap();
        let ext = session
            .state()
            .config()
            .get_extension::<datafusion_policy_cedar::CatalogFactSinkExt>()
            .expect("catalog fact sink attached to the session config");

        use datafusion_policy_cedar::TableFacts;
        let facts = TableFacts {
            tags: ["pii".to_string()].into_iter().collect(),
            ..Default::default()
        };
        ext.0
            .record(datafusion::sql::TableReference::full("c", "s", "t"), facts);
        assert_eq!(
            ext.0
                .get(&datafusion::sql::TableReference::full("c", "s", "t"))
                .unwrap()
                .tags,
            ["pii".to_string()].into_iter().collect()
        );
    }

    /// A policy that denies everything — the coarse gate must reject the query.
    #[derive(Debug)]
    struct DenyAll;

    #[async_trait]
    impl Policy for DenyAll {
        async fn is_allowed(
            &self,
            _plan: &LogicalPlan,
            _p: &PrincipalIdentity,
            _eval: &EvalContext,
        ) -> Result<Decision> {
            Ok(Decision::Deny)
        }
    }

    #[derive(Debug, Default, Clone)]
    struct RecordingTransport {
        events: Arc<Mutex<Vec<RunEvent>>>,
    }

    #[async_trait]
    impl Transport for RecordingTransport {
        async fn emit(&self, event: &RunEvent) -> Result<(), TransportError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    /// The coarse gate denies a disallowed principal at physical-plan time
    /// (statement path).
    #[tokio::test]
    async fn create_physical_plan_denies_on_policy() {
        let ctx = ctx_with_table(Arc::new(DenyAll), None).await;
        let session = ctx.session();
        let plan = session
            .create_logical_plan("SELECT id FROM t")
            .await
            .unwrap();
        let err = session.create_physical_plan(&plan).await.unwrap_err();
        assert!(
            err.to_string().contains("not authorized"),
            "expected an authorization error, got: {err}"
        );
    }

    /// Under `Permissive` posture the same `DenyAll` policy does **not** block:
    /// the planner evaluates + logs `would_block` but plans the query anyway
    /// (the dry-run stance). The mirror of `create_physical_plan_denies_on_policy`,
    /// proving the posture threads from `Governance` through the composed planner.
    #[tokio::test]
    async fn permissive_posture_does_not_block_a_deny() {
        let ctx = ctx_with_table_at_posture(
            Arc::new(DenyAll),
            datafusion_policy::Posture::Permissive,
            None,
        )
        .await;
        let session = ctx.session();
        let plan = session
            .create_logical_plan("SELECT id FROM t")
            .await
            .unwrap();
        // A Deny under Permissive must not error — the query plans.
        session
            .create_physical_plan(&plan)
            .await
            .expect("permissive must not block a Deny");
    }

    /// The same gate also guards the ingest/delta-connect path
    /// (`execute_logical_plan`).
    #[tokio::test]
    async fn execute_logical_plan_denies_on_policy() {
        let ctx = ctx_with_table(Arc::new(DenyAll), None).await;
        let plan = ctx
            .session()
            .create_logical_plan("SELECT id FROM t")
            .await
            .unwrap();
        let err = ctx.execute_logical_plan(plan).await.unwrap_err();
        assert!(err.to_string().contains("not authorized"), "got: {err}");
    }

    /// An allow-all policy lets the query through and returns rows.
    #[tokio::test]
    async fn allow_all_returns_rows() {
        let ctx = ctx_with_table(Arc::new(StaticPolicy::new(Decision::Allow)), None).await;
        let session = ctx.session();
        let plan = session
            .create_logical_plan("SELECT id FROM t")
            .await
            .unwrap();
        let physical = session.create_physical_plan(&plan).await.unwrap();
        let batches = datafusion::physical_plan::collect(physical, ctx.ctx().task_ctx())
            .await
            .unwrap();
        let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(rows, 3);
    }

    /// OpenLineage emits START + COMPLETE (sharing a run id) around a real query
    /// run through the session, and the SQL/parent context flows from a
    /// per-request session into the event.
    #[tokio::test]
    async fn lineage_start_and_complete_share_run_id() {
        use datafusion_openlineage::context::LineageContext;
        let transport = RecordingTransport::default();
        let client = OpenLineageClient::new(Arc::new(transport.clone()));
        let ctx = ctx_with_table(Arc::new(StaticPolicy::new(Decision::Allow)), Some(client)).await;

        // Attach a per-request lineage context (as the server's `session_for` does).
        let session = ctx.session_with_lineage(LineageContext {
            sql: Some("SELECT id FROM t".to_string()),
            ..Default::default()
        });
        let plan = session
            .create_logical_plan("SELECT id FROM t")
            .await
            .unwrap();
        let physical = session.create_physical_plan(&plan).await.unwrap();
        let _ = datafusion::physical_plan::collect(physical, ctx.ctx().task_ctx())
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let events = transport.events.lock().unwrap();
        let start = events
            .iter()
            .find(|e| e.event_type == RunEventType::Start)
            .expect("START");
        let complete = events
            .iter()
            .find(|e| e.event_type == RunEventType::Complete)
            .expect("COMPLETE");
        assert_eq!(
            start.run.run_id, complete.run.run_id,
            "START/COMPLETE share a run id"
        );
        // The SQL facet flowed from the per-request context.
        let sql = start
            .job
            .facets
            .sql
            .as_ref()
            .expect("sql job facet present");
        assert_eq!(sql.query, "SELECT id FROM t");
    }

    /// A query denied by the policy gate emits NO lineage: the single planner
    /// runs govern+gate first, so a Deny errors before the lineage step ever
    /// fires — no START, no FAIL. (The audit trail for blocked access, if wanted,
    /// is the policy layer's concern, not lineage's.)
    #[tokio::test]
    async fn denied_query_emits_no_lineage() {
        use datafusion_openlineage::context::LineageContext;
        let transport = RecordingTransport::default();
        let client = OpenLineageClient::new(Arc::new(transport.clone()));
        // `DenyAll` gate + lineage wired: the gate runs first, before any lineage.
        let ctx = ctx_with_table(Arc::new(DenyAll), Some(client)).await;

        let session = ctx.session_with_lineage(LineageContext {
            sql: Some("SELECT id FROM t".to_string()),
            ..Default::default()
        });
        let plan = session
            .create_logical_plan("SELECT id FROM t")
            .await
            .unwrap();
        let err = session.create_physical_plan(&plan).await.unwrap_err();
        assert!(err.to_string().contains("not authorized"), "got: {err}");

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            transport.events.lock().unwrap().is_empty(),
            "a denied query emits no lineage events"
        );
    }

    /// With the governance feature, a `TablePolicy` filters rows and masks
    /// columns end-to-end through the real session path.
    #[tokio::test]
    async fn governance_filters_and_masks_rows() {
        use datafusion::common::DFSchema;
        use datafusion::logical_expr::{col, lit};
        use datafusion::sql::TableReference;
        use datafusion_policy_cedar::TablePolicy;
        use std::collections::HashMap;

        #[derive(Debug)]
        struct GovPolicy;
        #[async_trait]
        impl Policy for GovPolicy {
            async fn is_allowed(
                &self,
                _p: &LogicalPlan,
                _pr: &PrincipalIdentity,
                _eval: &EvalContext,
            ) -> Result<Decision> {
                Ok(Decision::Allow)
            }
            async fn constrain(
                &self,
                _table: &TableReference,
                _schema: &DFSchema,
                _principal: &PrincipalIdentity,
                _eval: &EvalContext,
            ) -> Result<TablePolicy> {
                let mut masks = HashMap::new();
                masks.insert("ssn".to_string(), lit("***"));
                Ok(TablePolicy {
                    row_filters: vec![col("region").eq(lit("eu"))],
                    column_masks: masks,
                })
            }
        }

        let ctx = ctx_with_table(Arc::new(GovPolicy), None).await;
        let session = ctx.session();
        let plan = session
            .create_logical_plan("SELECT id, region, ssn FROM t")
            .await
            .unwrap();
        let physical = session.create_physical_plan(&plan).await.unwrap();
        let batches = datafusion::physical_plan::collect(physical, ctx.ctx().task_ctx())
            .await
            .unwrap();

        // Only the two region='eu' rows survive, and ssn is masked to '***'.
        let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(rows, 2, "row filter keeps only region='eu'");
        let pretty = datafusion::arrow::util::pretty::pretty_format_batches(&batches)
            .unwrap()
            .to_string();
        assert!(pretty.contains("***"), "ssn masked: {pretty}");
        assert!(
            !pretty.contains(" a ") && !pretty.contains(" c "),
            "raw ssn must not leak: {pretty}"
        );
    }

    /// End-to-end taint recording through the real session path: a query that
    /// reads a pii-tagged column accrues "pii" in the engine's session fact
    /// store, keyed by the session's correlation id. Stands up the engine and a
    /// registered table, and seeds the catalog fact sink directly (rather than
    /// resolving through Unity Catalog).
    #[tokio::test]
    async fn governance_records_observed_taints() {
        use datafusion::sql::TableReference;
        use datafusion_policy_cedar::TableFacts;
        use std::collections::HashMap;

        use crate::engine::Engine;
        use crate::policy::StaticPolicy;

        let engine = Engine::new(
            governance_for(Arc::new(StaticPolicy::new(Decision::Allow))),
            None,
            None,
            OpenLineageConfig::default(),
        );
        let session = engine
            .new_session(principal("alice"), None)
            .await
            .expect("session");

        // Register table `t` on the session context.
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("ssn", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec!["a", "b"])),
            ],
        )
        .unwrap();
        let table = MemTable::try_new(schema, vec![vec![batch]]).unwrap();
        session
            .session_context()
            .register_table("t", Arc::new(table))
            .unwrap();

        // Seed the catalog fact sink: `t.ssn` is classified `pii` (what
        // `build_delta` would record after Unity Catalog resolution).
        session.catalog_fact_sink().record(
            TableReference::bare("t"),
            TableFacts {
                column_tags: HashMap::from([(
                    "ssn".to_string(),
                    ["pii".to_string()].into_iter().collect(),
                )]),
                ..Default::default()
            },
        );

        // Run a query that reads the tagged column through the real gate path.
        let lh = session.lakehouse_for_query(LineageContext::default(), None);
        let plan = lh
            .create_logical_plan("SELECT id, ssn FROM t")
            .await
            .unwrap();
        lh.create_physical_plan(&plan).await.unwrap();

        // The session ledger gained "pii", keyed by this session's correlation id.
        assert_eq!(
            engine
                .fact_store()
                .observed_taints(&session.correlation_id()),
            ["pii".to_string()].into_iter().collect()
        );
    }

    /// End-to-end column mask through the **real** Cedar engine: a governed tag
    /// `pii = "ssn"` on the `ssn` column flows facts → sink → `EvalContext` →
    /// `column_is_masked` → `resolve_mask_expr`. A standard-clearance principal
    /// sees `ssn` masked (the default `***` literal, since no `@mask_fn` resolver
    /// is wired on this session path); a high-clearance principal — who discharges
    /// the forbid via `unless { principal.clearance == "high" }` — sees it raw.
    ///
    /// The engine is stood up in-process from Cedar policy + schema text via
    /// `CedarPolicyEngine::from_sources`, so the whole governed-tag mask contract
    /// is exercised without an OCI registry or a live Unity Catalog.
    #[tokio::test]
    async fn governed_tag_masks_column_end_to_end() {
        use std::collections::{BTreeMap, HashMap};

        use datafusion::sql::TableReference;
        use datafusion_policy_cedar::{CedarPolicyEngine, TableFacts};

        use crate::engine::Engine;

        // A minimal self-contained schema + policy (the showcase's `pii=ssn`
        // column mask), embedded so the test stays hermetic. `read_table` is
        // permitted for any principal with a region; the `read_column` forbid
        // masks a column tagged `pii == "ssn"` unless the principal is
        // high-clearance.
        const SCHEMA: &str = r#"
            entity UserGroup in [UserGroup];
            entity User in [UserGroup] { region: String, clearance?: String };
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
            when { resource.hasTag("pii") && resource.getTag("pii") == "ssn" }
            unless { principal has clearance && principal.clearance == "high" };
        "#;

        // Run the query as `principal`, seeding `t.ssn` with the governed tag, and
        // return the pretty-printed result batches.
        async fn run(principal: PrincipalIdentity) -> String {
            let policy =
                Arc::new(CedarPolicyEngine::from_sources(POLICY, "", Some(SCHEMA)).unwrap());
            let engine = Engine::new(
                governance_for(policy),
                None,
                None,
                OpenLineageConfig::default(),
            );
            let session = engine.new_session(principal, None).await.expect("session");

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
            let table = MemTable::try_new(schema, vec![vec![batch]]).unwrap();
            session
                .session_context()
                .register_table("t", Arc::new(table))
                .unwrap();

            // `t.ssn` carries the governed tag `pii = "ssn"` — what `record_facts`
            // records after Unity Catalog resolution.
            session.catalog_fact_sink().record(
                TableReference::bare("t"),
                TableFacts {
                    governed_column_tags: HashMap::from([(
                        "ssn".to_string(),
                        BTreeMap::from([("pii".to_string(), "ssn".to_string())]),
                    )]),
                    ..Default::default()
                },
            );

            let lh = session.lakehouse_for_query(LineageContext::default(), None);
            let plan = lh
                .create_logical_plan("SELECT id, ssn FROM t")
                .await
                .unwrap();
            let physical = lh.create_physical_plan(&plan).await.unwrap();
            let batches = datafusion::physical_plan::collect(physical, lh.task_ctx())
                .await
                .unwrap();
            datafusion::arrow::util::pretty::pretty_format_batches(&batches)
                .unwrap()
                .to_string()
        }

        // Standard-clearance principal: the forbid fires, `ssn` is masked to the
        // default literal and the raw values never surface.
        let masked = run(principal("alice")).await;
        assert!(
            masked.contains("***"),
            "ssn masked for standard clearance: {masked}"
        );
        assert!(
            !masked.contains("secret-a") && !masked.contains("secret-b"),
            "raw ssn must not leak: {masked}"
        );

        // High-clearance principal: the `unless` clause discharges the forbid, so
        // `ssn` is returned raw.
        let high = principal("carol").with_attribute("clearance", "high");
        let unmasked = run(high).await;
        assert!(
            unmasked.contains("secret-a") && unmasked.contains("secret-b"),
            "high-clearance principal sees raw ssn: {unmasked}"
        );
        assert!(
            !unmasked.contains("***"),
            "no mask for high clearance: {unmasked}"
        );
    }

    /// End-to-end **row filter** through the real Cedar engine: a governed table
    /// tag gates a `read_table` permit whose residual is a per-row predicate, so
    /// the query returns only the rows the principal is entitled to.
    ///
    /// The permit fires only for a table carrying the governed tag `access =
    /// "row-scoped"` (folded into the Cedar `Table` entity's native tags by the
    /// resource PIP — breakwater S2). Its `resource.region == principal.region`
    /// clause has no static answer for `resource.region` (a table column), so TPE
    /// leaves it as the residual `col("region") == <principal region>`. Before
    /// S2's table-tag folding the `hasTag` guard was unsatisfiable and this
    /// scenario failed closed for everyone; this is the full-stack proof of the
    /// fix — an `eu` principal sees only `eu` rows, a `us` principal only `us`.
    #[tokio::test]
    async fn governed_tag_row_filter_end_to_end() {
        use std::collections::BTreeMap;

        use datafusion::sql::TableReference;
        use datafusion_policy_cedar::{CedarPolicyEngine, TableFacts};

        use crate::engine::Engine;

        // `read_table` is permitted only when the table carries the governed
        // `access` tag AND the row's `region` matches the principal's region; the
        // region comparison survives TPE as a row filter. (No `read_column` rule,
        // so no masking — this isolates the row-filter path.)
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
        // Two `read_table` permits:
        //   1. the coarse gate — permit any principal with a region, so the
        //      concrete `is_allowed` gate lets the read through (governed tags are
        //      not folded on the concrete gate path, only under TPE, so the coarse
        //      permit must not depend on `hasTag`);
        //   2. the row-filter grant — gated on the governed table tag
        //      `access = "row-scoped"`, so it only contributes a residual once the
        //      resource PIP has folded the tag into the `Table` entity's native
        //      Cedar tags (breakwater S2). Under TPE `resource.hasTag("access")`
        //      is discharged by the folded tag while `resource.region` (a column)
        //      stays unknown, leaving the per-row residual
        //      `col("region") == <principal region>`. Without S2's folding the
        //      `hasTag` guard is unsatisfiable, no residual survives, and the row
        //      filter silently vanishes — the regression this test guards.
        const POLICY: &str = r#"
            permit (principal, action == Action::"read_table", resource)
            when { principal has region };

            permit (principal, action == Action::"read_table", resource)
            when {
                resource.hasTag("access") && resource.getTag("access") == "row-scoped"
                && resource.region == principal.region
            };
        "#;

        // Run `SELECT region, amount FROM t` as `principal`, seeding `t` with the
        // governed table tag, and return the pretty-printed result batches.
        async fn run(principal: PrincipalIdentity) -> String {
            let policy =
                Arc::new(CedarPolicyEngine::from_sources(POLICY, "", Some(SCHEMA)).unwrap());
            let engine = Engine::new(
                governance_for(policy),
                None,
                None,
                OpenLineageConfig::default(),
            );
            let session = engine.new_session(principal, None).await.expect("session");

            let schema = Arc::new(Schema::new(vec![
                Field::new("region", DataType::Utf8, false),
                Field::new("amount", DataType::Int64, false),
            ]));
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(StringArray::from(vec!["eu", "us", "eu", "us"])),
                    Arc::new(Int64Array::from(vec![10, 20, 30, 40])),
                ],
            )
            .unwrap();
            let table = MemTable::try_new(schema, vec![vec![batch]]).unwrap();
            session
                .session_context()
                .register_table("t", Arc::new(table))
                .unwrap();

            // `t` carries the governed table tag `access = "row-scoped"` — what
            // `record_facts` records after Unity Catalog resolution.
            session.catalog_fact_sink().record(
                TableReference::bare("t"),
                TableFacts {
                    governed_tags: BTreeMap::from([(
                        "access".to_string(),
                        "row-scoped".to_string(),
                    )]),
                    ..Default::default()
                },
            );

            let lh = session.lakehouse_for_query(LineageContext::default(), None);
            let plan = lh
                .create_logical_plan("SELECT region, amount FROM t ORDER BY amount")
                .await
                .unwrap();
            let physical = lh.create_physical_plan(&plan).await.unwrap();
            let batches = datafusion::physical_plan::collect(physical, lh.task_ctx())
                .await
                .unwrap();
            datafusion::arrow::util::pretty::pretty_format_batches(&batches)
                .unwrap()
                .to_string()
        }

        // `eu` principal: the residual filters to `region == "eu"`, so only the
        // two `eu` rows (10, 30) survive; the `us` rows never surface.
        let eu = run(principal("alice")).await; // `principal` seeds region = "eu"
        assert!(
            eu.contains("| eu     | 10"),
            "eu principal sees eu rows: {eu}"
        );
        assert!(
            eu.contains("| eu     | 30"),
            "eu principal sees all eu rows: {eu}"
        );
        assert!(
            !eu.contains("| us"),
            "eu principal must not see us rows: {eu}"
        );

        // `us` principal: the same policy filters to `region == "us"`, a disjoint
        // row set — proving the filter is principal-scoped, not a static grant.
        let bob = principal("bob").with_attribute("region", "us");
        let us = run(bob).await;
        assert!(
            us.contains("| us     | 20") && us.contains("| us     | 40"),
            "us principal sees us rows: {us}"
        );
        assert!(
            !us.contains("| eu"),
            "us principal must not see eu rows: {us}"
        );
    }

    /// The coarse write gate still fires under the resolver-tier secured-view
    /// wrapper: `authorize_logical_plan` (the write/ingest path's explicit gate,
    /// the same one Flight ingest and managed-append run) authorizes an `INSERT`
    /// under the `write_table` action. The secured-view wrapper handles only the
    /// fine-grained (row/column) path and marks the table governed — it does not
    /// swallow the coarse `is_allowed` gate — so a writer authorized by the ACL
    /// facts appends, while a non-writer is rejected.
    #[tokio::test]
    async fn governed_write_gate_still_fires_end_to_end() {
        use datafusion_policy_cedar::CedarPolicyEngine;

        const SCHEMA: &str = r#"
            entity User { region: String };
            entity Table { region: String } tags String;
            action read_table appliesTo {
                principal: [User], resource: [Table],
                context: { catalog?: String, schema?: String, table?: String, columns?: Set<String> }
            };
            action write_table appliesTo { principal: [User], resource: [Table] };
        "#;
        // Reads are open; only alice may write (the analog of a table `writers`
        // ACL — here a direct principal match, since Cedar policies match the
        // principal entity, not a uid string).
        const POLICY: &str = r#"
            permit (principal, action == Action::"read_table", resource)
            when { principal has region };

            permit (principal == User::"alice", action == Action::"write_table", resource);
        "#;

        // Build a `LakehouseCtx` over an in-memory `t`, seed `t`'s writers ACL,
        // then authorize an `INSERT` as `principal`. Returns the gate's result.
        async fn insert_as(principal: PrincipalIdentity) -> Result<()> {
            let policy =
                Arc::new(CedarPolicyEngine::from_sources(POLICY, "", Some(SCHEMA)).unwrap());
            // Bind the *writing* principal so the write-path gate resolves it.
            let session = create_session_for(
                None,
                None,
                OpenLineageConfig::default(),
                Some(principal.clone()),
                None,
                &governance_for(policy.clone()),
                &crate::config::ObjectStoreConfig::default(),
            )
            .unwrap();
            let schema = Arc::new(Schema::new(vec![
                Field::new("id", DataType::Int64, false),
                Field::new("region", DataType::Utf8, true),
            ]));
            let batch = RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(Int64Array::from(vec![1])),
                    Arc::new(StringArray::from(vec!["eu"])),
                ],
            )
            .unwrap();
            let table = MemTable::try_new(schema, vec![vec![batch]]).unwrap();
            session.register_table("t", Arc::new(table)).unwrap();

            let ctx = LakehouseCtx::new(session, policy, principal);
            let lh = ctx.session();
            let plan = lh
                .create_logical_plan("INSERT INTO t VALUES (2, 'us')")
                .await?;
            // The write path's explicit gate (govern → optimize → coarse gate).
            ctx.authorize_logical_plan(plan).await.map(|_| ())
        }

        // alice is in the writers ACL: the write is authorized.
        insert_as(principal("alice"))
            .await
            .expect("authorized writer's INSERT must pass the gate");

        // bob is not: the coarse `write_table` gate denies (proving the wrapper
        // did not swallow it).
        let denied = insert_as(principal("bob"))
            .await
            .expect_err("non-writer's INSERT must be rejected");
        assert!(
            denied.to_string().contains("not authorized"),
            "write denied for a non-writer: {denied}"
        );
    }

    /// The agent-tool PEP seam: `Session::authorize_tool_call` reads the
    /// session's observed taints back and runs the tool policy. Here a guardrail
    /// policy forbids `send_external` once `pii` has been observed.
    #[tokio::test]
    async fn authorize_tool_call_forbids_on_observed_pii() {
        use std::collections::BTreeSet;

        use datafusion_policy_cedar::Decision;

        use crate::engine::Engine;
        use crate::policy::Policy;

        // A policy whose tool guardrail forbids send_external when pii is seen.
        #[derive(Debug)]
        struct Guardrail;
        #[async_trait]
        impl Policy for Guardrail {
            async fn is_allowed(
                &self,
                _p: &LogicalPlan,
                _pr: &PrincipalIdentity,
                _e: &EvalContext,
            ) -> Result<Decision> {
                Ok(Decision::Allow)
            }
            async fn tool_policy(
                &self,
                action: &str,
                _principal: &PrincipalIdentity,
                observed_taints: &BTreeSet<String>,
            ) -> Result<Decision> {
                if action == "send_external" && observed_taints.contains("pii") {
                    Ok(Decision::Deny)
                } else {
                    Ok(Decision::Allow)
                }
            }
        }

        let engine = Engine::new(
            governance_for(Arc::new(Guardrail)),
            None,
            None,
            OpenLineageConfig::default(),
        );
        let session = engine
            .new_session(principal("alice"), None)
            .await
            .expect("session");

        // Clean session: the tool call is permitted.
        assert_eq!(
            session.authorize_tool_call("send_external").await.unwrap(),
            Decision::Allow
        );

        // After the ledger accrues pii (as the governance PEP would), the same
        // tool call is forbidden.
        engine
            .fact_store()
            .record_taint(&session.correlation_id(), "pii");
        assert_eq!(
            session.authorize_tool_call("send_external").await.unwrap(),
            Decision::Deny
        );
    }
}
