//! The session layer: a long-lived [`Engine`], per-connection [`Session`]s, and
//! the [`SessionStore`] that owns their lifecycle.
//!
//! This is the three-layer context model
//! (`docs/adr/0001-layered-session-context-model.md`):
//!
//! ```text
//! Engine    — one per process; identity-independent inputs for building sessions
//!   └─ Session   — one per client connection; OWNS its RuntimeEnv / object-store
//!        │          registry (credential isolation), principal binding, statements
//!        └─ LakehouseSession — one per query; cheap SessionState clone + per-request
//!                              SessionConfig extensions (lineage / agent context)
//! ```
//!
//! **Credential isolation** is load-bearing here
//! (`docs/adr/0004-per-session-credential-isolation.md`): Unity Catalog vends
//! short-lived credentials and registers them as object stores on a session's
//! `RuntimeEnv`, keyed by table URL. If sessions shared one `RuntimeEnv` (e.g. by
//! forking from a single base via `SessionStateBuilder::new_from_existing`), one
//! principal's vended (possibly elevated) store would be visible to another's
//! queries. So each [`Session`] is built fresh via [`create_session`] and owns
//! its own runtime; the `Engine` shares only identity-independent inputs.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use datafusion::error::Result;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SessionContext;
use datafusion_cedar::{IdentityProvider, PrincipalClaims, PrincipalEnrichment, PrincipalIdentity};
use datafusion_open_lineage::OpenLineageClient;
use datafusion_open_lineage::config::OpenLineageConfig;
use datafusion_open_lineage::context::LineageContext;
use datafusion_unitycatalog::catalog::UnityCatalogProviderList;
use unitycatalog_object_store::UnityObjectStoreFactory;
use uuid::Uuid;

use crate::agent::{AgentContext, AgentContextExt};
use crate::lineage::LineageContextExt;
use crate::policy::Policy;
use crate::session::{LakehouseCtx, LakehouseSession, build_unity_resolver, create_session_for};

/// A planned statement awaiting execution, scoped to the [`Session`] that
/// planned it.
///
/// Holds the lineage `run_id` minted at plan time so the later `do_get_*` RPC
/// can reuse it: OpenLineage START (plan time) and COMPLETE/FAIL (execution
/// time) then carry one shared `runId`
/// (`docs/adr/0003-per-statement-run-id-correlation.md`).
#[derive(Clone)]
pub struct StoredStatement {
    pub plan: LogicalPlan,
    /// Snapshot of the lineage context captured from the planning RPC's
    /// metadata (parent-run facet + sql), with the minted `run_id` pinned in
    /// `lineage.run_id`. Reattached verbatim at execution so START and
    /// COMPLETE/FAIL correlate on one run id.
    pub lineage: LineageContext,
    pub created_at: Instant,
}

/// Long-lived, process-wide factory for sessions.
///
/// Holds only the identity-independent inputs used to build a session — **no
/// live `RuntimeEnv` / object stores**, so no vended credentials can leak across
/// principals (see module docs).
pub struct Engine {
    policy: Arc<dyn Policy>,
    unity_factory: Option<Arc<UnityObjectStoreFactory>>,
    lineage: Option<OpenLineageClient>,
    /// Static OpenLineage config (job namespace, producer, engine identity),
    /// built once at startup and used when instrumenting each session's planner.
    lineage_config: OpenLineageConfig,
    /// The principal/identity PIP: resolves a principal's attributes + group
    /// membership from external systems (`docs/adr/0008-...`). Defaults to a
    /// no-op provider (empty enrichment) so an unconfigured server / the
    /// anonymous dev path still works.
    identity: Arc<dyn IdentityProvider>,
    /// Process-wide enrichment cache, keyed by principal uid. v1 resolves once
    /// per session and reuses; the cache being here (rather than baked into a
    /// `Session`) is what lets enrichment move to a per-query lookup with a TTL
    /// later without re-plumbing.
    enrichment_cache: DashMap<String, PrincipalEnrichment>,
    /// Process-wide session fact store (the taint ledger), keyed by correlation
    /// id. Shared across sessions, as a future shared-KV backend would be. The
    /// governance PEP records taints here; a later PEP reads them back.
    #[cfg(feature = "governance")]
    fact_store: Arc<dyn datafusion_cedar::FactStore>,
}

impl Engine {
    pub fn new(
        policy: Arc<dyn Policy>,
        unity_factory: Option<Arc<UnityObjectStoreFactory>>,
        lineage: Option<OpenLineageClient>,
        lineage_config: OpenLineageConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            policy,
            unity_factory,
            lineage,
            lineage_config,
            identity: Arc::new(NoopIdentityProvider),
            enrichment_cache: DashMap::new(),
            #[cfg(feature = "governance")]
            fact_store: Arc::new(datafusion_cedar::InMemoryFactStore::new()),
        })
    }

    /// The process-wide session fact store (taint ledger). The agent-tool PEP
    /// reads observed taints back from here by correlation id; until that PEP is
    /// wired (PR7) this is exercised only by tests.
    #[cfg(feature = "governance")]
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn fact_store(&self) -> Arc<dyn datafusion_cedar::FactStore> {
        self.fact_store.clone()
    }

    /// Replace the identity provider (e.g. a config- or IdP-backed one). The
    /// default is a no-op provider returning empty enrichment.
    pub fn with_identity_provider(
        mut self: Arc<Self>,
        identity: Arc<dyn IdentityProvider>,
    ) -> Arc<Self> {
        // `Engine` is only shared after construction, so this unwrap holds at
        // wiring time (before any clone of the Arc escapes).
        let engine = Arc::get_mut(&mut self).expect("Engine not yet shared");
        engine.identity = identity;
        self
    }

    /// Resolve (and cache) the enrichment for `uid`, then apply it to
    /// `principal`. Keyed by uid on the process-wide cache so concurrent
    /// sessions of the same user share one provider call. Fail-closed: a
    /// provider error propagates so the caller can fail the session.
    pub async fn enrich(&self, principal: PrincipalIdentity) -> Result<PrincipalIdentity> {
        let key = principal.uid.to_string();
        if let Some(cached) = self.enrichment_cache.get(&key) {
            return Ok(principal.enriched(cached.clone()));
        }
        let enrichment = self
            .identity
            .enrich(&principal.uid, &PrincipalClaims::default())
            .await
            .map_err(|e| {
                datafusion::error::DataFusionError::External(Box::new(std::io::Error::other(
                    e.to_string(),
                )))
            })?;
        self.enrichment_cache.insert(key, enrichment.clone());
        Ok(principal.enriched(enrichment))
    }

    /// Build a fresh, principal-scoped [`Session`].
    ///
    /// Each session gets its own `SessionContext` (and therefore its own
    /// `RuntimeEnv` / object-store registry) via [`create_session`], binds the
    /// principal into the session config, and attaches a Unity Catalog resolver
    /// bound to *this* session's runtime — so vended credentials land on this
    /// session's registry and nowhere else.
    pub fn new_session(&self, principal: PrincipalIdentity) -> Result<Arc<Session>> {
        let ctx = create_session_for(
            Uuid::new_v4(),
            self.lineage.clone(),
            self.lineage_config.clone(),
            Some(principal.clone()),
            self.unity_factory.clone(),
        )?;

        let unity = self
            .unity_factory
            .clone()
            .map(|factory| build_unity_resolver(&ctx, factory));

        Ok(Arc::new(Session {
            ctx,
            policy: self.policy.clone(),
            principal,
            unity,
            statements: DashMap::new(),
            created_at: Instant::now(),
            last_used_ms: AtomicU64::new(0),
            #[cfg(feature = "governance")]
            fact_store: self.fact_store.clone(),
        }))
    }
}

/// The default identity provider: returns empty enrichment for every principal.
///
/// Keeps an unconfigured server (and the anonymous dev path) working — an empty
/// enrichment is a *success*, leaving the principal with only its request-time
/// attributes and no group membership.
#[derive(Debug)]
struct NoopIdentityProvider;

#[async_trait::async_trait]
impl IdentityProvider for NoopIdentityProvider {
    async fn enrich(
        &self,
        _uid: &datafusion_cedar::EntityUid,
        _claims: &PrincipalClaims,
    ) -> std::result::Result<PrincipalEnrichment, datafusion_cedar::IdentityError> {
        Ok(PrincipalEnrichment::default())
    }
}

/// A per-connection session: its own DataFusion context (and runtime), the bound
/// principal, and the statements it has planned.
pub struct Session {
    /// Owns this session's `RuntimeEnv` / object-store registry. Vended UC
    /// credentials registered here are isolated to this session.
    ctx: SessionContext,
    policy: Arc<dyn Policy>,
    principal: PrincipalIdentity,
    unity: Option<Arc<UnityCatalogProviderList>>,
    /// Statements this session has planned, keyed by handle.
    statements: DashMap<Uuid, StoredStatement>,
    created_at: Instant,
    /// Millis since `created_at` of the last use, for idle-TTL eviction.
    last_used_ms: AtomicU64,
    /// The process-wide taint ledger, shared from the [`Engine`]. Threaded into
    /// each per-query [`LakehouseSession`] so the governance PEP can record
    /// taints keyed by this session's correlation id.
    #[cfg(feature = "governance")]
    fact_store: Arc<dyn datafusion_cedar::FactStore>,
}

impl Session {
    /// Record activity, resetting the idle timer.
    pub fn touch(&self) {
        let elapsed = self.created_at.elapsed().as_millis() as u64;
        self.last_used_ms.store(elapsed, Ordering::Relaxed);
    }

    /// How long this session has been idle since its last use.
    pub fn idle(&self) -> Duration {
        let last = self.last_used_ms.load(Ordering::Relaxed);
        self.created_at
            .elapsed()
            .saturating_sub(Duration::from_millis(last))
    }

    /// The principal this session runs as. The policy gate reads the principal
    /// off the per-query `LakehouseSession`; the server reads it here to stamp
    /// lineage provenance (the `hydrofoil` run facet).
    pub fn principal(&self) -> &PrincipalIdentity {
        &self.principal
    }

    pub fn statements(&self) -> &DashMap<Uuid, StoredStatement> {
        &self.statements
    }

    /// A [`LakehouseCtx`] over this session, for the ingest / delta-connect
    /// paths and the CPU-runtime planning call sites that take an
    /// `Arc<LakehouseCtx>`.
    pub fn ctx(&self) -> LakehouseCtx {
        let mut ctx = LakehouseCtx::new(
            self.ctx.clone(),
            self.policy.clone(),
            self.principal.clone(),
        );
        if let Some(unity) = self.unity.clone() {
            ctx = ctx.with_unity(unity);
        }
        ctx
    }

    /// A per-query [`LakehouseSession`] with no per-request extensions, for
    /// metadata RPCs and planning where no lineage/agent context applies.
    /// (Query paths use [`Self::lakehouse_for_query`]; this is the plain variant.)
    #[allow(dead_code)]
    pub fn lakehouse(&self) -> LakehouseSession {
        let mut state = self.ctx.state();
        self.attach_fact_store(&mut state);
        LakehouseSession::new(
            state,
            self.policy.clone(),
            self.principal.clone(),
            self.unity.clone(),
        )
    }

    /// A per-query [`LakehouseSession`] decorated with the request's lineage
    /// context (run_id pinned) and optional agent/governance context.
    ///
    /// Clones the session's `SessionState` (cheap — internals are `Arc`-shared)
    /// and attaches the per-request context as `SessionConfig` extensions, which
    /// the OpenLineage provider and the policy layer read back at planning time.
    /// The extensions live only on this clone; the long-lived session is
    /// untouched.
    pub fn lakehouse_for_query(
        &self,
        lineage: LineageContext,
        agent: Option<AgentContext>,
    ) -> LakehouseSession {
        let mut state = self.ctx.state();
        state
            .config_mut()
            .set_extension(Arc::new(LineageContextExt(lineage)));
        if let Some(agent) = agent {
            state
                .config_mut()
                .set_extension(Arc::new(AgentContextExt(agent)));
        }
        self.attach_fact_store(&mut state);
        LakehouseSession::new(
            state,
            self.policy.clone(),
            self.principal.clone(),
            self.unity.clone(),
        )
    }

    /// Attach the session's taint ledger to a per-query state so the governance
    /// PEP records into it. No-op without the `governance` feature.
    #[cfg(feature = "governance")]
    fn attach_fact_store(&self, state: &mut datafusion::execution::context::SessionState) {
        state
            .config_mut()
            .set_extension(Arc::new(crate::session::FactStoreExt(
                self.fact_store.clone(),
            )));
    }

    #[cfg(not(feature = "governance"))]
    fn attach_fact_store(&self, _state: &mut datafusion::execution::context::SessionState) {}

    /// The agent-tool PEP: decide whether this session's principal may invoke
    /// the tool named `action`, given the taints the session has observed so far
    /// (read back from the fact store by this session's correlation id).
    ///
    /// This is the read-back counterpart to the governance PEP's taint recording
    /// — a query that read a `pii` column accrues `pii`, and a later
    /// `send_external` tool call can be forbidden on that basis (the data-flow
    /// control that survives prompt injection).
    ///
    // SEAM: not yet invoked. A future agent-tool RPC in `server.rs` (a
    // `do_action`-style handler) would resolve the session, read the
    // `x-hydrofoil-agent-*` metadata, and call this before dispatching the tool.
    #[cfg(feature = "governance")]
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn authorize_tool_call(&self, action: &str) -> Result<datafusion_cedar::Decision> {
        let correlation_id = self.ctx.state().session_id().to_string();
        let taints = self.fact_store.observed_taints(&correlation_id);
        self.policy
            .tool_policy(action, &self.principal, &taints)
            .await
    }

    /// Drop statements idle longer than `ttl` (backstops `get_flight_info` calls
    /// that mint a handle but are never fetched).
    pub fn sweep_statements(&self, ttl: Duration) {
        self.statements.retain(|_, s| s.created_at.elapsed() < ttl);
    }

    #[cfg(test)]
    pub(crate) fn runtime_env(&self) -> Arc<datafusion::execution::runtime_env::RuntimeEnv> {
        self.ctx.runtime_env()
    }

    /// The session's catalog fact sink (the instance `build_delta` writes to and
    /// the policy layer reads). For tests that populate facts without standing
    /// up Unity Catalog.
    #[cfg(test)]
    pub(crate) fn catalog_fact_sink(&self) -> datafusion_cedar::CatalogFactSink {
        self.ctx
            .state()
            .config()
            .get_extension::<crate::catalog::CatalogFactSinkExt>()
            .map(|ext| ext.0.clone())
            .unwrap_or_default()
    }

    /// This session's correlation id (its session id), the key under which the
    /// governance PEP records taints.
    #[cfg(test)]
    pub(crate) fn correlation_id(&self) -> String {
        self.ctx.state().session_id().to_string()
    }

    /// The session's `SessionContext`, for tests that register tables directly.
    #[cfg(test)]
    pub(crate) fn session_context(&self) -> &SessionContext {
        &self.ctx
    }
}

/// Owns the set of live [`Session`]s, keyed by session id, with idle-TTL
/// eviction.
pub struct SessionStore {
    engine: Arc<Engine>,
    sessions: DashMap<String, Arc<Session>>,
    ttl: Duration,
}

impl SessionStore {
    pub fn new(engine: Arc<Engine>, ttl: Duration) -> Arc<Self> {
        Arc::new(Self {
            engine,
            sessions: DashMap::new(),
            ttl,
        })
    }

    /// Enrich a principal via the engine's identity PIP (attributes + group
    /// membership) before it is bound into a session. Fail-closed: propagates a
    /// provider error so the caller fails the session rather than proceeding
    /// un-enriched.
    pub async fn enrich(&self, principal: PrincipalIdentity) -> Result<PrincipalIdentity> {
        self.engine.enrich(principal).await
    }

    /// Mint a brand-new session for a connection, returning its id and handle.
    pub fn create(&self, principal: PrincipalIdentity) -> Result<(String, Arc<Session>)> {
        let id = Uuid::new_v4().to_string();
        let session = self.engine.new_session(principal)?;
        self.sessions.insert(id.clone(), session.clone());
        Ok((id, session))
    }

    /// Resolve a session by id, touching its idle timer. `None` if unknown.
    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        let session = self.sessions.get(id)?.clone();
        session.touch();
        Some(session)
    }

    /// Resolve (or create) the stable ephemeral session for a principal that did
    /// not establish a session via handshake.
    ///
    /// Keyed by `ephemeral:{principal_uid}` so the two RPCs of one logical query
    /// (`get_flight_info_*` then `do_get_*`) share a statement store even without
    /// a client-provided session id. This preserves today's no-handshake demo
    /// behaviour (DuckDB, plain pyarrow) under the new model.
    pub fn ephemeral_for(&self, principal: PrincipalIdentity) -> Result<Arc<Session>> {
        let key = format!("ephemeral:{}", principal.uid);
        if let Some(session) = self.get(&key) {
            return Ok(session);
        }
        let session = self.engine.new_session(principal)?;
        // `entry` guards against a concurrent insert racing between the `get`
        // above and here.
        let session = self.sessions.entry(key).or_insert(session).clone();
        session.touch();
        Ok(session)
    }

    /// Evict sessions idle longer than the configured TTL, and prune stale
    /// statements from survivors.
    pub fn sweep(&self) {
        let ttl = self.ttl;
        self.sessions.retain(|_, s| s.idle() < ttl);
        for s in self.sessions.iter() {
            s.sweep_statements(ttl);
        }
    }

    /// Spawn a background sweeper on the **main** tokio runtime.
    ///
    /// Must not run on the CPU runtime, whose handle has IO/time disabled.
    pub fn spawn_sweeper(self: &Arc<Self>) {
        let store = self.clone();
        let interval = std::cmp::max(self.ttl / 4, Duration::from_secs(30));
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            loop {
                tick.tick().await;
                store.sweep();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use cedar_oci::{Decision, EntityUid};

    use super::*;
    use crate::policy::StaticPolicy;

    fn principal(name: &str) -> PrincipalIdentity {
        PrincipalIdentity::new(EntityUid::from_str(&format!("User::\"{name}\"")).unwrap())
    }

    /// A `Transport` that records every emitted event, for asserting on lineage
    /// emission in tests.
    #[derive(Debug, Default, Clone)]
    struct Recorder {
        events: Arc<std::sync::Mutex<Vec<datafusion_open_lineage::event::RunEvent>>>,
    }

    #[async_trait::async_trait]
    impl datafusion_open_lineage::transport::Transport for Recorder {
        async fn emit(
            &self,
            event: &datafusion_open_lineage::event::RunEvent,
        ) -> std::result::Result<(), datafusion_open_lineage::transport::TransportError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    /// A lineage-instrumented engine + a recorder capturing its events.
    fn recording_engine() -> (Arc<Engine>, Recorder) {
        let recorder = Recorder::default();
        let client = OpenLineageClient::new(Arc::new(recorder.clone()));
        let engine = Engine::new(
            Arc::new(StaticPolicy::new(Decision::Allow)),
            None,
            Some(client),
            OpenLineageConfig::default(),
        );
        (engine, recorder)
    }

    fn engine() -> Arc<Engine> {
        Engine::new(
            Arc::new(StaticPolicy::new(Decision::Allow)),
            None,
            None,
            OpenLineageConfig::default(),
        )
    }

    #[tokio::test]
    async fn store_create_and_resolve() {
        let store = SessionStore::new(engine(), Duration::from_secs(60));
        let (id, session) = store.create(principal("alice")).unwrap();

        let resolved = store.get(&id).expect("session resolves by id");
        assert!(
            Arc::ptr_eq(&session, &resolved),
            "same Arc<Session> handed back"
        );
        assert!(store.get("unknown").is_none());
    }

    #[tokio::test]
    async fn ttl_evicts_idle_sessions() {
        // Zero TTL: any positive idle time evicts on the next sweep.
        let store = SessionStore::new(engine(), Duration::ZERO);
        let (id, session) = store.create(principal("alice")).unwrap();
        // Back-date last-used so idle() is clearly positive without sleeping.
        session.last_used_ms.store(0, Ordering::Relaxed);
        // created_at is ~now, so idle() ~= 0; force a measurable gap.
        tokio::time::sleep(Duration::from_millis(5)).await;
        store.sweep();
        assert!(store.get(&id).is_none(), "idle session evicted by sweep");
    }

    #[tokio::test]
    async fn ephemeral_is_stable_per_principal() {
        let store = SessionStore::new(engine(), Duration::from_secs(60));
        let a = store.ephemeral_for(principal("alice")).unwrap();
        let a2 = store.ephemeral_for(principal("alice")).unwrap();
        let b = store.ephemeral_for(principal("bob")).unwrap();

        assert!(
            Arc::ptr_eq(&a, &a2),
            "same principal -> same ephemeral session"
        );
        assert!(
            !Arc::ptr_eq(&a, &b),
            "different principal -> different session"
        );
    }

    /// Credential isolation: distinct sessions must own distinct RuntimeEnvs so
    /// vended UC object stores cannot cross principals.
    #[tokio::test]
    async fn sessions_have_isolated_runtimes() {
        let store = SessionStore::new(engine(), Duration::from_secs(60));
        let (_, a) = store.create(principal("alice")).unwrap();
        let (_, b) = store.create(principal("bob")).unwrap();
        assert!(
            !Arc::ptr_eq(&a.runtime_env(), &b.runtime_env()),
            "each session must own its own RuntimeEnv / object-store registry"
        );
    }

    /// The per-query agent/governance context attached via `lakehouse_for_query`
    /// is readable back from the session config (the seam the deferred agent PEP
    /// will use).
    #[tokio::test]
    async fn agent_context_round_trips_through_session_config() {
        use datafusion::catalog::Session as _;

        let (_, session) = SessionStore::new(engine(), Duration::from_secs(60))
            .create(principal("alice"))
            .unwrap();
        let agent = AgentContext {
            agent_id: Some("agent-7".into()),
            task: Some("summarize".into()),
            ..Default::default()
        };
        let lh = session.lakehouse_for_query(LineageContext::default(), Some(agent.clone()));
        let read = lh
            .config()
            .get_extension::<AgentContextExt>()
            .expect("agent context attached");
        assert_eq!(read.0, agent);
    }

    /// Regression guard for the cross-RPC correlation bug: the run id pinned in a
    /// `StoredStatement` at planning time is reused at execution time, so every
    /// OpenLineage event (START at plan, COMPLETE at stream end) carries the same
    /// run id — even though planning and execution build separate per-query
    /// sessions. See docs/adr/0003-per-statement-run-id-correlation.md.
    #[tokio::test]
    async fn pinned_run_id_correlates_start_and_complete() {
        use std::sync::Mutex;

        use async_trait::async_trait;
        use datafusion::arrow::array::Int64Array;
        use datafusion::arrow::datatypes::{DataType, Field, Schema};
        use datafusion::arrow::record_batch::RecordBatch;
        use datafusion::catalog::Session as _;
        use datafusion::datasource::MemTable;
        use datafusion_open_lineage::OpenLineageClient;
        use datafusion_open_lineage::event::{RunEvent, RunEventType};
        use datafusion_open_lineage::transport::{Transport, TransportError};

        #[derive(Debug, Default, Clone)]
        struct Recorder {
            events: Arc<Mutex<Vec<RunEvent>>>,
        }
        #[async_trait]
        impl Transport for Recorder {
            async fn emit(&self, event: &RunEvent) -> Result<(), TransportError> {
                self.events.lock().unwrap().push(event.clone());
                Ok(())
            }
        }

        let recorder = Recorder::default();
        let client = OpenLineageClient::new(Arc::new(recorder.clone()));
        let eng = Engine::new(
            Arc::new(StaticPolicy::new(Decision::Allow)),
            None,
            Some(client),
            OpenLineageConfig::default(),
        );
        let (_, session) = SessionStore::new(eng, Duration::from_secs(60))
            .create(principal("alice"))
            .unwrap();

        // A tiny table to scan.
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
        )
        .unwrap();
        session
            .ctx
            .register_table(
                "t",
                Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
            )
            .unwrap();

        // Pin a run id in a stored statement (as get_flight_info_statement does).
        let run_id = Uuid::now_v7();
        let lineage = LineageContext {
            run_id: Some(run_id),
            sql: Some("SELECT id FROM t".into()),
            ..Default::default()
        };

        // Execute through a per-query session decorated with the pinned context,
        // as do_get_statement does (a separate session clone from planning).
        let lh = session.lakehouse_for_query(lineage, None);
        let plan = lh.create_logical_plan("SELECT id FROM t").await.unwrap();
        let physical = lh.create_physical_plan(&plan).await.unwrap();
        let _ = datafusion::physical_plan::collect(physical, session.ctx.task_ctx())
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(100)).await;
        let events = recorder.events.lock().unwrap();
        assert!(!events.is_empty(), "lineage events emitted");
        for e in events.iter() {
            assert_eq!(
                e.run.run_id, run_id,
                "every event carries the pinned run id"
            );
        }
        assert!(
            events.iter().any(|e| e.event_type == RunEventType::Start),
            "a START event was emitted"
        );
        assert!(
            events
                .iter()
                .any(|e| e.event_type == RunEventType::Complete),
            "a COMPLETE event was emitted"
        );
    }

    /// Register a tiny single-column `MemTable` named `t` on the session.
    fn register_tiny_table(session: &Session) {
        use datafusion::arrow::array::Int64Array;
        use datafusion::arrow::datatypes::{DataType, Field, Schema};
        use datafusion::arrow::record_batch::RecordBatch;
        use datafusion::datasource::MemTable;

        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
        )
        .unwrap();
        session
            .ctx
            .register_table(
                "t",
                Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()),
            )
            .unwrap();
    }

    /// Run `sql` to completion through a per-query session decorated with `lineage`.
    async fn run_query(session: &Session, lineage: LineageContext, sql: &'static str) {
        use datafusion::catalog::Session as _;
        let lh = session.lakehouse_for_query(lineage, None);
        let plan = lh.create_logical_plan(sql).await.unwrap();
        let physical = lh.create_physical_plan(&plan).await.unwrap();
        let _ = datafusion::physical_plan::collect(physical, session.ctx.task_ctx())
            .await
            .unwrap();
    }

    /// C5: re-executing one stored statement mints a fresh run id per execution,
    /// each parented to the planning run — so two executions never share a runId
    /// (which would let one's FAIL clobber the other's COMPLETE). Mirrors what
    /// the server's `do_get_prepared_statement` does via
    /// `crate::lineage::execution_context`.
    #[tokio::test]
    async fn prepared_statement_reexecution_uses_distinct_run_ids() {
        let (eng, recorder) = recording_engine();
        let (_, session) = SessionStore::new(eng, Duration::from_secs(60))
            .create(principal("alice"))
            .unwrap();
        register_tiny_table(&session);

        // The context pinned at prepared-statement creation.
        let planning_run = Uuid::now_v7();
        let stored = LineageContext {
            run_id: Some(planning_run),
            job_name: Some(crate::lineage::job_name_from_sql("SELECT id FROM t")),
            sql: Some("SELECT id FROM t".into()),
            ..Default::default()
        };
        let config = OpenLineageConfig::default();

        // Two executions of the same stored statement.
        run_query(
            &session,
            crate::lineage::execution_context(&stored, &config),
            "SELECT id FROM t",
        )
        .await;
        run_query(
            &session,
            crate::lineage::execution_context(&stored, &config),
            "SELECT id FROM t",
        )
        .await;

        tokio::time::sleep(Duration::from_millis(100)).await;
        let events = recorder.events.lock().unwrap();

        let run_ids: std::collections::HashSet<_> = events.iter().map(|e| e.run.run_id).collect();
        assert_eq!(
            run_ids.len(),
            2,
            "two executions -> two distinct run ids (got {run_ids:?})"
        );
        assert!(
            !run_ids.contains(&planning_run),
            "neither execution reuses the planning run id"
        );
        // Every event parents to the planning run, under one stable job name.
        for e in events.iter() {
            let parent = e.run.facets.parent.as_ref().expect("parent facet present");
            assert_eq!(parent.run.run_id, planning_run.to_string());
            assert_eq!(e.job.name, stored.job_name.clone().unwrap());
        }
    }

    /// ADR 0012: client-forwarded job metadata (documentation/tags facets via
    /// `LineageContext.job_facets`) and the `hydrofoil` provenance run facet
    /// (`run_facets`) ride through planning into every emitted event.
    #[tokio::test]
    async fn client_metadata_facets_land_in_events() {
        let (eng, recorder) = recording_engine();
        let (_, session) = SessionStore::new(eng, Duration::from_secs(60))
            .create(principal("alice"))
            .unwrap();
        register_tiny_table(&session);

        let mut lineage = LineageContext {
            run_id: Some(Uuid::now_v7()),
            job_namespace: Some("demo-pipeline".into()),
            job_name: Some("events_summary".into()),
            sql: Some("SELECT id FROM t".into()),
            ..Default::default()
        };
        lineage.job_facets.insert(
            "documentation".into(),
            serde_json::json!({"description": "Daily rollup."}),
        );
        lineage.run_facets.insert(
            "hydrofoil".into(),
            serde_json::json!({"principal": "User::\"alice\""}),
        );

        run_query(&session, lineage, "SELECT id FROM t").await;

        tokio::time::sleep(Duration::from_millis(100)).await;
        let events = recorder.events.lock().unwrap();
        assert!(!events.is_empty());
        for e in events.iter() {
            assert_eq!(
                e.job.namespace, "demo-pipeline",
                "namespace override applies"
            );
            assert_eq!(e.job.name, "events_summary");
            assert_eq!(
                e.job.facets.extra["documentation"]["description"],
                "Daily rollup."
            );
            assert_eq!(
                e.run.facets.extra["hydrofoil"]["principal"],
                "User::\"alice\""
            );
        }
    }

    /// C9.2: a query that reads and writes nothing (`information_schema`
    /// introspection here) emits no lineage events — the dataset-less job node
    /// would only add noise.
    #[tokio::test]
    async fn dataset_less_query_emits_no_events() {
        let (eng, recorder) = recording_engine();
        let (_, session) = SessionStore::new(eng, Duration::from_secs(60))
            .create(principal("alice"))
            .unwrap();
        register_tiny_table(&session);

        let lineage = LineageContext {
            run_id: Some(Uuid::now_v7()),
            job_name: Some("query-meta".into()),
            sql: Some("SELECT table_name FROM information_schema.tables".into()),
            ..Default::default()
        };
        run_query(
            &session,
            lineage,
            "SELECT table_name FROM information_schema.tables",
        )
        .await;

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            recorder.events.lock().unwrap().is_empty(),
            "information_schema query touches no datasets -> no lineage events"
        );
    }
}
