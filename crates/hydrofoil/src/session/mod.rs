use std::{any::Any, collections::HashMap, sync::Arc};

use arrow::array::RecordBatch;
use cedar_oci::Decision;
use datafusion::{
    catalog::{
        AsyncCatalogProviderList as _, CatalogProvider as _, MemoryCatalogProvider, Session,
    },
    common::{DFSchema, exec_err},
    config::{ConfigOptions, TableOptions},
    datasource::{TableProvider, provider_as_source},
    error::Result,
    execution::{
        SessionState, SessionStateBuilder, TaskContext, context::ExecutionProps,
        runtime_env::RuntimeEnv,
    },
    logical_expr::{AggregateUDF, LogicalPlan, LogicalPlanBuilder, ScalarUDF, WindowUDF},
    physical_plan::{ExecutionPlan, PhysicalExpr},
    prelude::{DataFrame, Expr, SessionConfig, SessionContext},
};
use datafusion_cedar::PrincipalIdentity;
use datafusion_open_lineage::{OpenLineageClient, OpenLineageConfig, instrument_session_state};
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

use deltalake_datafusion::catalog::unity::UnityCatalogProviderList;
use unitycatalog_object_store::UnityObjectStoreFactory;

use datafusion_open_lineage::context::LineageContext;

use crate::{
    catalog::{DeltaTableFactory, LakehouseSchemaProvider, LakehouseTableProviderBuilder},
    lineage::LineageContextExt,
    policy::Policy,
};

mod log_store;

use log_store::DataFusionLogStore;

/// Inject fine-grained governance (row filters + column masks) into a plan
/// before optimization.
///
/// With the `governance` feature this delegates to
/// [`datafusion_cedar::govern_plan`]; without it (the default), it is a no-op
/// that returns the plan unchanged — the coarse access gate still applies.
#[cfg(feature = "governance")]
async fn govern_plan(
    plan: &LogicalPlan,
    policy: &dyn Policy,
    principal: &PrincipalIdentity,
) -> Result<LogicalPlan> {
    datafusion_cedar::govern_plan(plan, policy, principal).await
}

#[cfg(not(feature = "governance"))]
async fn govern_plan(
    plan: &LogicalPlan,
    _policy: &dyn Policy,
    _principal: &PrincipalIdentity,
) -> Result<LogicalPlan> {
    Ok(plan.clone())
}

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

    pub fn session(&self) -> LakehouseSession {
        LakehouseSession {
            inner: self.inner.state(),
            policy: self.policy.clone(),
            principal: self.principal.clone(),
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
            policy: self.policy.clone(),
            principal: self.principal.clone(),
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
        let governed = govern_plan(&plan, self.policy.as_ref(), &self.principal).await?;
        let optimized_plan = self.inner.state().optimize(&governed)?;
        if self
            .policy
            .is_allowed(&optimized_plan, &self.principal)
            .await?
            == Decision::Deny
        {
            return exec_err!(
                "Principal '{}' is not authorized to execute this query",
                self.principal.uid
            );
        }
        self.inner.execute_logical_plan(governed).await
    }
}

/// Custom session providing Lakehouse specific features on top of DataFusion's [`SessionState`].
///
/// This is the main entry point for query execution and planning, and is where we will
/// enforce policies and manage state related to the lakehouse.
#[derive(Debug, Clone)]
pub struct LakehouseSession {
    /// The underlying DataFusion session state that manages most of the execution context.
    inner: SessionState,
    /// The policy engine used to enforce access control
    /// and other policies during query planning and execution.
    policy: Arc<dyn Policy>,
    /// The principal (user or service) on behalf of whom queries are executed.
    principal: PrincipalIdentity,
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
    /// context).
    pub fn new(
        inner: SessionState,
        policy: Arc<dyn Policy>,
        principal: PrincipalIdentity,
        unity: Option<Arc<UnityCatalogProviderList>>,
    ) -> Self {
        Self {
            inner,
            policy,
            principal,
            unity,
        }
    }

    pub async fn create_logical_plan(
        &self,
        query: impl AsRef<str> + Send + 'static,
    ) -> Result<LogicalPlan> {
        let query = query.as_ref();
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

        // Inject fine-grained governance (row filters + column masks) before
        // optimization, so the injected predicate pushes into the scan and
        // masked-away columns are pruned. Then optimize, so the coarse gate
        // sees projections/filters pushed down to the table scan level and can
        // authorize based on the actual data being accessed.
        let governed = govern_plan(logical_plan, self.policy.as_ref(), &self.principal).await?;
        let optimized_plan = self.inner.optimize(&governed)?;
        if self
            .policy
            .is_allowed(&optimized_plan, &self.principal)
            .await?
            == Decision::Deny
        {
            return exec_err!(
                "Principal '{}' is not authorized to execute this query",
                self.principal.uid
            );
        }
        self.inner
            .query_planner()
            .create_physical_plan(&optimized_plan, &self.inner)
            .await
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
    #[instrument(skip(self), level = "info")]
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

    /// Build a logical scan of a Delta table by object-store URL (optionally at a
    /// specific version). Retained as a seam for direct-by-location scans; the
    /// catalog path resolves tables by name instead, so this isn't wired in yet.
    #[allow(dead_code)]
    #[instrument(skip(self), level = "info")]
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
) -> Result<SessionContext> {
    create_session_for(session_id, lineage, None)
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
    principal: Option<PrincipalIdentity>,
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

    // Bind the principal (the access-control analog of the lineage context) into
    // the config so the policy layer / providers can resolve it from a
    // `SessionState` during planning.
    if let Some(principal) = principal {
        session_config = crate::identity::with_principal(session_config, principal);
    }

    let mut session_state = SessionStateBuilder::new_with_default_features()
        .with_session_id(session_id.into().unwrap_or_else(Uuid::new_v4).to_string())
        .with_config(session_config)
        .with_table_factory(
            DeltaTableFactory::FILE_FORMAT.to_string(),
            DeltaTableFactory::instance(),
        )
        .with_physical_optimizer_rule(instrument_rule)
        .build();

    // Emit OpenLineage events around physical planning when a client is wired.
    // The context provider reads the per-request `LineageContext` the server
    // attaches to the `SessionConfig` via `LakehouseCtx::session_with_lineage`
    // (parent-run facet + SQL text); when none is attached it resolves to empty.
    if let Some(client) = lineage {
        session_state = instrument_session_state(
            session_state,
            client,
            Arc::new(crate::lineage::HydrofoilContextProvider),
            OpenLineageConfig::default(),
        );
    }

    let ctx = SessionContext::new_with_state(session_state);

    update_session(&ctx.state())?;

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
) -> Arc<UnityCatalogProviderList> {
    let runtime = ctx.runtime_env();
    let builder = LakehouseTableProviderBuilder::new(ctx.task_ctx());
    Arc::new(UnityCatalogProviderList::new(factory, runtime, builder))
}

fn update_session(session: &dyn Session) -> Result<()> {
    add_seaweedfs(session, Handle::current())?;
    Ok(())
}

fn add_seaweedfs(session: &dyn Session, handle: Handle) -> Result<()> {
    let object_store = Arc::new(
        AmazonS3Builder::new()
            .with_http_connector(SpawnedReqwestConnector::new(handle))
            .with_access_key_id("seaweed-key-id")
            .with_secret_access_key("seaweed-access-key")
            .with_endpoint("http://localhost:8333/")
            .with_bucket_name("open-lakehouse")
            .with_allow_http(true)
            .build()?,
    );
    let instrumented = instrument_object_store(object_store, "seaweedfs");
    let url = url::Url::parse("s3://open-lakehouse/").unwrap();
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

    use cedar_oci::{Decision, EntityUid};
    use datafusion_cedar::PrincipalIdentity;
    use datafusion_open_lineage::OpenLineageClient;
    use datafusion_open_lineage::event::{RunEvent, RunEventType};
    use datafusion_open_lineage::transport::{Transport, TransportError};

    use super::*;
    use crate::policy::{Policy, StaticPolicy};

    fn principal(name: &str) -> PrincipalIdentity {
        use std::str::FromStr as _;
        PrincipalIdentity::new(EntityUid::from_str(&format!("User::\"{name}\"")).unwrap())
            .with_attribute("region", "eu")
    }

    async fn ctx_with_table(
        policy: Arc<dyn Policy>,
        lineage: Option<OpenLineageClient>,
    ) -> LakehouseCtx {
        let session = create_session(None, lineage).unwrap();
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

    /// A policy that denies everything — the coarse gate must reject the query.
    #[derive(Debug)]
    struct DenyAll;

    #[async_trait]
    impl Policy for DenyAll {
        async fn is_allowed(
            &self,
            _plan: &LogicalPlan,
            _p: &PrincipalIdentity,
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
        use datafusion_open_lineage::context::LineageContext;
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

    /// With the governance feature, a `TablePolicy` filters rows and masks
    /// columns end-to-end through the real session path.
    #[cfg(feature = "governance")]
    #[tokio::test]
    async fn governance_filters_and_masks_rows() {
        use datafusion::common::DFSchema;
        use datafusion::logical_expr::{col, lit};
        use datafusion::sql::TableReference;
        use datafusion_cedar::TablePolicy;
        use std::collections::HashMap;

        #[derive(Debug)]
        struct GovPolicy;
        #[async_trait]
        impl Policy for GovPolicy {
            async fn is_allowed(
                &self,
                _p: &LogicalPlan,
                _pr: &PrincipalIdentity,
            ) -> Result<Decision> {
                Ok(Decision::Allow)
            }
            async fn table_policy(
                &self,
                _table: &TableReference,
                _schema: &DFSchema,
                _principal: &PrincipalIdentity,
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
}
