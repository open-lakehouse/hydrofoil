use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use arrow::array::AsArray;
use arrow::datatypes::UInt64Type;
use arrow::ipc::writer::IpcWriteOptions;
use arrow_flight::sql::{
    ActionClosePreparedStatementRequest, ActionCreatePreparedStatementRequest,
    ActionCreatePreparedStatementResult, Any, CommandGetCatalogs, CommandGetDbSchemas,
    CommandGetSqlInfo, CommandGetTables, CommandGetXdbcTypeInfo, CommandPreparedStatementQuery,
    CommandPreparedStatementUpdate, CommandStatementIngest, CommandStatementQuery,
    CommandStatementUpdate, DoPutUpdateResult, ProstMessageExt, SqlInfo, TicketStatementQuery,
    server::{FlightSqlService, PeekableFlightDataStream},
};
use arrow_flight::{
    Action, FlightDescriptor, FlightEndpoint, FlightInfo, HandshakeRequest, HandshakeResponse,
    IpcMessage, PutResult, SchemaAsIpc, Ticket, encode::FlightDataEncoderBuilder,
    flight_descriptor::DescriptorType, flight_service_server::FlightService,
};
use bytes::Bytes;
use cedar_oci::Decision;
use datafusion::error::DataFusionError;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SQLOptions;
use datafusion_open_lineage::OpenLineageClient;
use datafusion_open_lineage::config::OpenLineageConfig;
use futures::{Stream, TryStreamExt};
use hydrofoil_common::DeltaCommand;
use prost::Message;
use tonic::metadata::{MetadataMap, MetadataValue};
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, info, instrument};
use uuid::Uuid;

use unitycatalog_object_store::UnityObjectStoreFactory;

use crate::engine::{Engine, Session, SessionStore, StoredStatement};
use crate::stream::FlightDataReceiverStreamBuilder;
use crate::{execution::CpuRuntime, policy::Policy};
use crate::{
    planner::{DeltaPlanner, FlightPlanner},
    policy::StaticPolicy,
};

mod metadata;

/// gRPC metadata key carrying the Flight SQL session id (a slash-safe,
/// header-friendly alternative to the `authorization: Bearer` channel ADBC
/// uses). See `docs/adr/0002-flight-sql-session-identity.md`.
const SESSION_ID_HEADER: &str = "x-session-id";

/// Idle TTL for the placeholder session store created by [`FlightSqlServiceImpl::try_new`],
/// before [`FlightSqlServiceImpl::build`] replaces it with the configured TTL.
const DEFAULT_SESSION_TTL_SECS: u64 = 1800;

macro_rules! status {
    ($desc:expr, $err:expr) => {
        Status::internal(format!("{}: {} at {}:{}", $desc, $err, file!(), line!()))
    };
}

/// Resolve the Flight SQL session id from request metadata, checking (in order)
/// the `x-session-id` header, the `cookie` header (`session_id=…`), and the
/// `authorization: Bearer <id>` header that ADBC echoes from the handshake
/// response.
fn session_id_from_metadata(meta: &MetadataMap) -> Option<String> {
    if let Some(id) = meta.get(SESSION_ID_HEADER).and_then(|v| v.to_str().ok()) {
        return Some(id.to_string());
    }
    if let Some(cookie) = meta.get("cookie").and_then(|v| v.to_str().ok()) {
        for part in cookie.split(';') {
            if let Some(val) = part.trim().strip_prefix("session_id=") {
                return Some(val.to_string());
            }
        }
    }
    if let Some(token) = meta
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|auth| auth.strip_prefix("Bearer "))
    {
        return Some(token.trim().to_string());
    }
    None
}

pub struct FlightSqlServiceImpl {
    /// The session store (sessions own their statements). Built by [`Self::build`].
    sessions: Arc<SessionStore>,
    executor: CpuRuntime,

    // Builder-stage inputs, assembled into the `Engine` by [`Self::build`].
    policy: Arc<dyn Policy>,
    unity_factory: Option<Arc<UnityObjectStoreFactory>>,
    lineage: Option<OpenLineageClient>,
    /// Static OpenLineage config (job namespace, producer, engine identity),
    /// built once at startup from the hydrofoil config and threaded through the
    /// engine + the per-request lineage context. Default until [`Self::with_lineage`].
    lineage_config: OpenLineageConfig,
    identity: Option<Arc<dyn datafusion_cedar::IdentityProvider>>,
}

impl FlightSqlServiceImpl {
    pub fn try_new() -> Result<Self, DataFusionError> {
        let policy: Arc<dyn Policy> = Arc::new(StaticPolicy::new(Decision::Allow));
        // A placeholder store; replaced by `build()` once the optional
        // components (lineage/policy/unity) are configured.
        let engine = Engine::new(policy.clone(), None, None, OpenLineageConfig::default());
        let ttl = Duration::from_secs(DEFAULT_SESSION_TTL_SECS);
        Ok(Self {
            sessions: SessionStore::new(engine, ttl),
            executor: CpuRuntime::try_new()?,
            policy,
            unity_factory: None,
            lineage: None,
            lineage_config: OpenLineageConfig::default(),
            identity: Some(crate::identity::default_identity_provider()),
        })
    }

    /// Attach an OpenLineage client (and its static config) so sessions emit
    /// lineage events around query planning. The config carries the job
    /// namespace, producer, and engine identity built once at startup.
    pub fn with_lineage(mut self, lineage: OpenLineageClient, config: OpenLineageConfig) -> Self {
        self.lineage = Some(lineage);
        self.lineage_config = config;
        self
    }

    /// Set the policy for the server.
    pub fn with_policy(mut self, policy: Arc<dyn Policy>) -> Self {
        self.policy = policy;
        self
    }

    /// Attach a Unity Catalog object store factory so that sessions resolve
    /// qualified table references against Unity Catalog with vended credentials.
    pub fn with_unity(mut self, factory: Arc<UnityObjectStoreFactory>) -> Self {
        self.unity_factory = Some(factory);
        self
    }

    /// Set the identity provider (the principal/identity PIP) used to enrich
    /// principals with attributes + group membership at session creation,
    /// overriding the [`default_identity_provider`](crate::identity::default_identity_provider).
    /// A deployment uses this to swap in a real IdP/directory backend.
    // The default provider is wired in `try_new`; this is the public override
    // hook, not yet exercised by the env-driven `main.rs` wiring.
    #[allow(dead_code)]
    pub fn with_identity_provider(
        mut self,
        identity: Arc<dyn datafusion_cedar::IdentityProvider>,
    ) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Finalize the configured components into an [`Engine`] + [`SessionStore`]
    /// and start the background session sweeper, using `ttl` as the idle session
    /// timeout. Call once before serving.
    pub fn build(mut self, ttl: Duration) -> Self {
        let mut engine = Engine::new(
            self.policy.clone(),
            self.unity_factory.clone(),
            self.lineage.clone(),
            self.lineage_config.clone(),
        );
        if let Some(identity) = self.identity.clone() {
            engine = engine.with_identity_provider(identity);
        }
        let sessions = SessionStore::new(engine, ttl);
        sessions.spawn_sweeper();
        self.sessions = sessions;
        self
    }

    /// Resolve the session for a request: by session id (handshake / cookie /
    /// Bearer) when present, otherwise the principal's stable ephemeral session
    /// (so no-handshake clients keep working — the two RPCs of one query still
    /// share a statement store). See `docs/adr/0002-flight-sql-session-identity.md`.
    #[allow(clippy::result_large_err)]
    async fn resolve_session<T>(&self, req: &Request<T>) -> Result<Arc<Session>, Status> {
        if let Some(session) =
            session_id_from_metadata(req.metadata()).and_then(|id| self.sessions.get(&id))
        {
            return Ok(session);
        }
        // No session id, or a known-shaped but unknown/expired one: fall through
        // to the principal's ephemeral session rather than hard-failing.
        let principal = crate::identity::principal_from_metadata(req.metadata())?;
        self.session_for_principal(principal).await
    }

    /// Resolve the stable ephemeral session for an already-parsed principal,
    /// enriching it (attributes + group membership) first. Shared by the gRPC
    /// no-session-id fallback ([`Self::resolve_session`]) and the HTTP query
    /// surface ([`crate::http`]), which always resolves by principal.
    ///
    /// Enrichment is fail-closed: a provider error fails the request rather than
    /// proceeding with an un-enriched (under/over-authorized) principal. The
    /// engine cache makes the enrich cheap on the reused ephemeral session.
    #[allow(clippy::result_large_err)]
    pub(crate) async fn session_for_principal(
        &self,
        principal: datafusion_cedar::PrincipalIdentity,
    ) -> Result<Arc<Session>, Status> {
        let principal = self
            .sessions
            .enrich(principal)
            .await
            .map_err(|e| status!("Failed to resolve identity", e))?;
        self.sessions
            .ephemeral_for(principal)
            .map_err(|e| status!("Failed to resolve session", e))
    }

    /// The CPU runtime used to build logical plans off the async I/O runtime.
    /// Exposed so the HTTP query surface ([`crate::http`]) shares the same
    /// planner path as the Flight statement RPC.
    pub(crate) fn executor(&self) -> &CpuRuntime {
        &self.executor
    }

    /// Build the per-request lineage context: parent-run facet, job-metadata
    /// facets (description/tags/owners), and namespace override parsed from
    /// metadata; the pinned `run_id`; the derived job name; the statement's SQL
    /// text; and the governance provenance (principal + agent context) as the
    /// custom `hydrofoil` run facet. The job namespace falls back to the
    /// engine's `OpenLineageConfig` at emit time when no header overrides it;
    /// the job *name* is derived per statement so distinct queries are distinct
    /// Marquez jobs — see [`crate::lineage::job_name_from_metadata`] and the
    /// `crate::lineage` module docs for the full header reference.
    fn lineage_context<T>(
        &self,
        req: &Request<T>,
        run_id: Uuid,
        sql: Option<&str>,
        principal: &datafusion_cedar::PrincipalIdentity,
        agent: Option<&crate::agent::AgentContext>,
    ) -> datafusion_open_lineage::context::LineageContext {
        let mut ctx = crate::lineage::context_from_metadata(req.metadata(), &self.lineage_config);
        ctx.run_id = Some(run_id);
        ctx.job_name = sql.map(|s| crate::lineage::job_name_from_metadata(req.metadata(), s));
        ctx.sql = sql.map(str::to_string);
        if let Some(facet) =
            crate::lineage::hydrofoil_run_facet(Some(principal), agent, &self.lineage_config)
        {
            ctx.run_facets.insert("hydrofoil".to_string(), facet);
        }
        ctx
    }

    /// Emit a standalone FAIL event for a query that errored during *logical*
    /// planning (parse / name resolution), before any physical plan — and thus
    /// any `OpenLineageExec` — exists. Without this such failures are invisible
    /// to lineage (the planner only emits inside `create_physical_plan`). The
    /// FAIL carries the same run/job identity the START would have used; inputs
    /// and outputs are unknown (the plan never resolved), so only the SQL +
    /// error facets are populated. No-op when lineage is not wired.
    fn emit_planning_failure(
        &self,
        lineage: &datafusion_open_lineage::context::LineageContext,
        error: &str,
    ) {
        let Some(client) = self.lineage.as_ref() else {
            return;
        };
        let run_id = lineage.run_id.unwrap_or_else(Uuid::now_v7);
        let query = datafusion_open_lineage::QueryLineage {
            sql: lineage.sql.clone(),
            ..Default::default()
        };
        client.emit(datafusion_open_lineage::builder::fail_event(
            run_id,
            &query,
            lineage,
            &self.lineage_config,
            error,
        ));
    }

    fn do_get_handle(
        &self,
        session: Arc<dyn datafusion::catalog::Session>,
        plan: LogicalPlan,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        // Reject DataFusion-native DDL/DML (e.g. `CREATE TABLE`, `INSERT`).
        // Unity Catalog DDL (`CREATE`/`DROP CATALOG`/`SCHEMA`) is planned as an
        // `Extension` node, which `verify_plan` does not classify as DDL — that
        // is intentional, not a bypass: such DDL is authorized by the Cedar gate
        // in `LakehouseSession::create_physical_plan` (the node lowers to a real
        // `create_catalog`/… action, default-deny if no policy permits it).
        let options = SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_dml(false);

        options
            .verify_plan(&plan)
            .map_err(|e| Status::internal(format!("{e:?}")))?;

        let mut builder = FlightDataReceiverStreamBuilder::new(100);
        builder.execute_logical_plan(session, plan, self.executor.handle());
        let stream = builder.build().map_err(Status::from);

        Ok(Response::new(Box::pin(stream)))
    }
}

#[tonic::async_trait]
impl FlightSqlService for FlightSqlServiceImpl {
    type FlightService = FlightSqlServiceImpl;

    /// Establish a session: mint a session id, return it to the client both as
    /// the handshake payload and as `authorization: Bearer <id>` + `x-session-id`
    /// response metadata (ADBC echoes the Bearer on subsequent RPCs). See
    /// `docs/adr/0002-flight-sql-session-identity.md`.
    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn do_handshake(
        &self,
        request: Request<Streaming<HandshakeRequest>>,
    ) -> Result<
        Response<Pin<Box<dyn Stream<Item = Result<HandshakeResponse, Status>> + Send>>>,
        Status,
    > {
        // Enrich the authenticated principal (attributes + group membership)
        // from the identity PIP before binding it into the session. Fail-closed:
        // a provider error fails the handshake rather than proceeding with an
        // un-enriched (under/over-authorized) principal.
        let principal = crate::identity::principal_from_metadata(request.metadata())?;
        let principal = self
            .sessions
            .enrich(principal)
            .await
            .map_err(|e| status!("Failed to resolve identity", e))?;
        let (session_id, _session) = self
            .sessions
            .create(principal)
            .map_err(|e| status!("Failed to create session", e))?;

        let payload = Bytes::from(session_id.clone());
        let output = futures::stream::once(async move {
            Ok(HandshakeResponse {
                protocol_version: 0,
                payload,
            })
        });
        let mut response: Response<Pin<Box<dyn Stream<Item = _> + Send>>> =
            Response::new(Box::pin(output));
        let bearer = format!("Bearer {session_id}");
        response.metadata_mut().insert(
            "authorization",
            MetadataValue::try_from(bearer).map_err(|e| status!("Invalid token", e))?,
        );
        response.metadata_mut().insert(
            SESSION_ID_HEADER,
            MetadataValue::try_from(session_id).map_err(|e| status!("Invalid session id", e))?,
        );
        Ok(response)
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn get_flight_info_catalogs(
        &self,
        query: CommandGetCatalogs,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let flight_descriptor = request.into_inner();
        let ticket = Ticket {
            ticket: query.as_any().encode_to_vec().into(),
        };
        let endpoint = FlightEndpoint::new().with_ticket(ticket);

        let flight_info = FlightInfo::new()
            .try_with_schema(&query.into_builder().schema())
            .map_err(|e| status!("Unable to encode schema", e))?
            .with_endpoint(endpoint)
            .with_descriptor(flight_descriptor);

        Ok(tonic::Response::new(flight_info))
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn get_flight_info_schemas(
        &self,
        query: CommandGetDbSchemas,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let flight_descriptor = request.into_inner();
        let ticket = Ticket {
            ticket: query.as_any().encode_to_vec().into(),
        };
        let endpoint = FlightEndpoint::new().with_ticket(ticket);

        let flight_info = FlightInfo::new()
            .try_with_schema(&query.into_builder().schema())
            .map_err(|e| status!("Unable to encode schema", e))?
            .with_endpoint(endpoint)
            .with_descriptor(flight_descriptor);

        Ok(tonic::Response::new(flight_info))
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn get_flight_info_tables(
        &self,
        query: CommandGetTables,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let flight_descriptor = request.into_inner();
        let ticket = Ticket {
            ticket: query.as_any().encode_to_vec().into(),
        };
        let endpoint = FlightEndpoint::new().with_ticket(ticket);

        let flight_info = FlightInfo::new()
            .try_with_schema(&query.into_builder().schema())
            .map_err(|e| status!("Unable to encode schema", e))?
            .with_endpoint(endpoint)
            .with_descriptor(flight_descriptor);

        Ok(tonic::Response::new(flight_info))
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn get_flight_info_sql_info(
        &self,
        query: CommandGetSqlInfo,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let flight_descriptor = request.into_inner();
        let ticket = Ticket::new(query.as_any().encode_to_vec());
        let endpoint = FlightEndpoint::new().with_ticket(ticket);

        let flight_info = FlightInfo::new()
            .try_with_schema(
                query
                    .into_builder(&metadata::INSTANCE_SQL_DATA)
                    .schema()
                    .as_ref(),
            )
            .map_err(|e| status!("Unable to encode schema", e))?
            .with_endpoint(endpoint)
            .with_descriptor(flight_descriptor);

        Ok(tonic::Response::new(flight_info))
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn get_flight_info_xdbc_type_info(
        &self,
        query: CommandGetXdbcTypeInfo,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let flight_descriptor = request.into_inner();
        let ticket = Ticket::new(query.as_any().encode_to_vec());
        let endpoint = FlightEndpoint::new().with_ticket(ticket);

        let flight_info = FlightInfo::new()
            .try_with_schema(
                query
                    .into_builder(&metadata::INSTANCE_XBDC_DATA)
                    .schema()
                    .as_ref(),
            )
            .map_err(|e| status!("Unable to encode schema", e))?
            .with_endpoint(endpoint)
            .with_descriptor(flight_descriptor);

        Ok(tonic::Response::new(flight_info))
    }

    /// Get a FlightInfo for executing a SQL query.
    #[instrument(
        skip_all,
        level = "info",
        fields(
            query = query.query.as_str(),
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn get_flight_info_statement(
        &self,
        query: CommandStatementQuery,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let session = self.resolve_session(&request).await?;

        // Mint the lineage run id now and snapshot the lineage context from this
        // (planning) RPC. Planning runs through a session decorated with that
        // context so the OpenLineage START event carries the pinned run id; the
        // later do_get reuses the same snapshot so COMPLETE/FAIL correlate. See
        // docs/adr/0003-per-statement-run-id-correlation.md.
        let run_id = Uuid::now_v7();
        let agent = crate::agent::agent_context_from_metadata(request.metadata());
        let lineage = self.lineage_context(
            &request,
            run_id,
            Some(&query.query),
            session.principal(),
            agent.as_ref(),
        );
        let lh = session.lakehouse_for_query(lineage.clone(), agent);

        let plan = match self
            .executor
            .create_logical_plan(lh, query.query.clone())
            .await
        {
            Ok(plan) => plan,
            Err(e) => {
                // Logical planning (parse / resolution) failed: no physical plan
                // is built, so the OpenLineageExec path never runs. Emit a FAIL
                // here under the same run/job identity so the failure is visible.
                self.emit_planning_failure(&lineage, &e.to_string());
                return Err(Status::internal(format!("Error building plan: {e}")));
            }
        };

        // See `do_get_handle`: this blocks DataFusion-native DDL/DML; Unity
        // Catalog DDL rides through as an `Extension` node and is authorized by
        // the Cedar gate in `create_physical_plan`, not here.
        let options = SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_dml(false);
        options
            .verify_plan(&plan)
            .map_err(|e| Status::internal(format!("{e:?}")))?;

        let plan_id = Uuid::now_v7();
        session.statements().insert(
            plan_id,
            StoredStatement {
                plan: plan.clone(),
                lineage,
                created_at: std::time::Instant::now(),
            },
        );

        let ticket = TicketStatementQuery {
            statement_handle: Bytes::copy_from_slice(plan_id.as_bytes()),
        };
        let ticket = Ticket {
            ticket: ticket.as_any().encode_to_vec().into(),
        };

        let info = FlightInfo::new()
            .try_with_schema(plan.schema().as_arrow())
            .expect("encoding failed")
            .with_endpoint(FlightEndpoint::new().with_ticket(ticket))
            .with_descriptor(FlightDescriptor {
                r#type: DescriptorType::Cmd.into(),
                cmd: query.as_any().encode_to_vec().into(),
                path: vec![],
            });

        Ok(Response::new(info))
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn get_flight_info_prepared_statement(
        &self,
        cmd: CommandPreparedStatementQuery,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let handle = Uuid::from_slice(&cmd.prepared_statement_handle)
            .map_err(|e| status!("Invalid handle", e))?;
        let session = self.resolve_session(&request).await?;
        let plan = session
            .statements()
            .get(&handle)
            .map(|s| s.plan.clone())
            .ok_or_else(|| Status::internal(format!("Plan handle not found: {handle}")))?;

        let ticket = CommandPreparedStatementQuery {
            prepared_statement_handle: cmd.prepared_statement_handle.clone(),
        };
        let ticket = Ticket {
            ticket: ticket.as_any().encode_to_vec().into(),
        };

        let info = FlightInfo::new()
            .try_with_schema(plan.schema().as_arrow())
            .expect("encoding failed")
            .with_endpoint(FlightEndpoint::new().with_ticket(ticket))
            .with_descriptor(FlightDescriptor {
                r#type: DescriptorType::Cmd.into(),
                cmd: cmd.as_any().encode_to_vec().into(),
                path: vec![],
            });

        Ok(Response::new(info))
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn do_get_sql_info(
        &self,
        query: CommandGetSqlInfo,
        _request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let builder = query.into_builder(&metadata::INSTANCE_SQL_DATA);
        let schema = builder.schema();
        let batch = builder.build();
        let stream = FlightDataEncoderBuilder::new()
            .with_schema(schema)
            .build(futures::stream::once(async { batch }))
            .map_err(Status::from);
        Ok(Response::new(Box::pin(stream)))
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn do_get_xdbc_type_info(
        &self,
        query: CommandGetXdbcTypeInfo,
        _request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        // create a builder with pre-defined Xdbc data:
        let builder = query.into_builder(&metadata::INSTANCE_XBDC_DATA);
        let schema = builder.schema();
        let batch = builder.build();
        let stream = FlightDataEncoderBuilder::new()
            .with_schema(schema)
            .build(futures::stream::once(async { batch }))
            .map_err(Status::from);
        Ok(Response::new(Box::pin(stream)))
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn do_get_statement(
        &self,
        ticket: TicketStatementQuery,
        request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let handle =
            Uuid::from_slice(&ticket.statement_handle).map_err(|e| status!("Invalid handle", e))?;
        let session = self.resolve_session(&request).await?;
        let stored = session
            .statements()
            .get(&handle)
            .map(|s| s.clone())
            .ok_or_else(|| Status::internal(format!("Plan handle not found: {handle}")))?;

        // Mint a fresh run id for this execution, with the planning run folded
        // in as the parent facet (see crate::lineage::execution_context and ADR
        // 0003): START and COMPLETE/FAIL emitted by the OpenLineageExec share
        // this execution's run id, while the parent chain ties it back to the
        // statement it was planned from. Layer this request's agent context on top.
        let mut lineage = crate::lineage::execution_context(&stored.lineage, &self.lineage_config);
        let agent = crate::agent::agent_context_from_metadata(request.metadata());
        // This request may carry its own agent context (it can differ from
        // planning's); refresh the provenance facet then — otherwise the
        // planning-time snapshot carried by `execution_context` stands.
        if agent.is_some()
            && let Some(facet) = crate::lineage::hydrofoil_run_facet(
                Some(session.principal()),
                agent.as_ref(),
                &self.lineage_config,
            )
        {
            lineage.run_facets.insert("hydrofoil".to_string(), facet);
        }
        let lh = session.lakehouse_for_query(lineage, agent);
        let result = self.do_get_handle(Arc::new(lh), stored.plan.clone());
        session.statements().remove(&handle);
        result
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn do_get_prepared_statement(
        &self,
        query: CommandPreparedStatementQuery,
        request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let handle = Uuid::from_slice(&query.prepared_statement_handle)
            .map_err(|e| status!("Invalid handle", e))?;
        let session = self.resolve_session(&request).await?;
        let stored = session
            .statements()
            .get(&handle)
            .map(|s| s.clone())
            .ok_or_else(|| Status::internal(format!("Plan handle not found: {handle}")))?;
        // Prepared statements are removed on ClosePreparedStatement, not here, so
        // one handle may be executed many times. Mint a fresh run id per
        // execution (parented to the planning run) so re-executions don't share
        // a runId and clobber one another's terminal event — see
        // crate::lineage::execution_context and ADR 0003.
        let mut lineage = crate::lineage::execution_context(&stored.lineage, &self.lineage_config);
        let agent = crate::agent::agent_context_from_metadata(request.metadata());
        // Per-execution agent context wins over the creation-time snapshot (one
        // prepared handle may serve many distinct agent tasks).
        if agent.is_some()
            && let Some(facet) = crate::lineage::hydrofoil_run_facet(
                Some(session.principal()),
                agent.as_ref(),
                &self.lineage_config,
            )
        {
            lineage.run_facets.insert("hydrofoil".to_string(), facet);
        }
        let lh = session.lakehouse_for_query(lineage, agent);
        self.do_get_handle(Arc::new(lh), stored.plan.clone())
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            message_type_url = message.type_url.as_str(),
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn do_get_fallback(
        &self,
        _request: Request<Ticket>,
        message: Any,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        Err(Status::unimplemented(format!(
            "do_get_fallback: {}",
            message.type_url
        )))
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn do_put_statement_update(
        &self,
        _handle: CommandStatementUpdate,
        _request: Request<PeekableFlightDataStream>,
    ) -> Result<i64, Status> {
        debug!("do_put_statement_update");
        // statements like "CREATE TABLE.." or "SET datafusion.nnn.." call this function
        // and we are required to return some row count here
        Ok(-1)
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn do_put_prepared_statement_update(
        &self,
        _handle: CommandPreparedStatementUpdate,
        _request: Request<PeekableFlightDataStream>,
    ) -> Result<i64, Status> {
        info!("do_put_prepared_statement_update");
        // statements like "CREATE TABLE.." or "SET datafusion.nnn.." call this function
        // and we are required to return some row count here
        Ok(-1)
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            table = ticket.table,
            schema = ticket.schema,
            catalog = ticket.catalog,
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL,
        )
    )]
    async fn do_put_statement_ingest(
        &self,
        ticket: CommandStatementIngest,
        request: Request<PeekableFlightDataStream>,
    ) -> Result<i64, Status> {
        let session = self.resolve_session(&request).await?;
        let ctx = Arc::new(session.ctx());
        let planner = FlightPlanner::new();

        let ctx_inner = ctx.clone();
        let plan = self
            .executor
            .spawn(async move {
                planner
                    .plan_ingest(ctx_inner.ctx(), &ticket, request.into_inner())
                    .await
            })
            .await
            .map_err(|e| status!("Failed to spawn ingest command", e))?
            .map_err(|e| status!("Failed to spawn ingest command", e))?;

        let batches = self
            .executor
            .execute_logical_plan(ctx, plan)
            .await
            .map_err(|e| status!("Failed to execute ingest command", e))?;

        if batches.is_empty() {
            Ok(0)
        } else {
            let row_count = batches[0].column(0).as_primitive::<UInt64Type>().value(0);
            Ok(row_count as i64)
        }
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            message_type_url = message.type_url.as_str(),
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn do_put_fallback(
        &self,
        request: Request<PeekableFlightDataStream>,
        message: Any,
    ) -> Result<Response<<Self as FlightService>::DoPutStream>, Status> {
        if !message.is::<DeltaCommand>() {
            return Err(Status::unimplemented(format!(
                "do_put: The defined request is invalid: {}",
                message.type_url
            )));
        }

        let command: DeltaCommand = message
            .unpack()
            .map_err(|e| Status::internal(format!("{e:?}")))?
            .ok_or_else(|| Status::internal("Expected DeltaCommand but got None!"))?;

        let session = self.resolve_session(&request).await?;
        let ctx = Arc::new(session.ctx());
        let planner = DeltaPlanner::new();
        let plan = planner
            .plan_delta_connect(&ctx.session(), &command)
            .map_err(|e| Status::internal(format!("Error planning delta command: {e}")))?;

        let _batches = self
            .executor
            .execute_logical_plan(ctx, plan)
            .await
            .map_err(|e| status!("Failed to execute ingest command", e))?;

        let result = DoPutUpdateResult { record_count: -1 };
        let output = futures::stream::iter(vec![Ok(PutResult {
            app_metadata: result.encode_to_vec().into(),
        })]);

        Ok(Response::new(Box::pin(output)))
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            query = query.query.as_str(),
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn do_action_create_prepared_statement(
        &self,
        query: ActionCreatePreparedStatementRequest,
        request: Request<Action>,
    ) -> Result<ActionCreatePreparedStatementResult, Status> {
        let session = self.resolve_session(&request).await?;

        let plan_id = Uuid::now_v7();
        let run_id = Uuid::now_v7();
        let agent = crate::agent::agent_context_from_metadata(request.metadata());
        let lineage = self.lineage_context(
            &request,
            run_id,
            Some(&query.query),
            session.principal(),
            agent.as_ref(),
        );
        let lh = session.lakehouse_for_query(lineage.clone(), agent);

        let plan = match self.executor.create_logical_plan(lh, query.query).await {
            Ok(plan) => plan,
            Err(e) => {
                self.emit_planning_failure(&lineage, &e.to_string());
                return Err(status!("Error building plan", e));
            }
        };

        session.statements().insert(
            plan_id,
            StoredStatement {
                plan: plan.clone(),
                lineage,
                created_at: std::time::Instant::now(),
            },
        );
        let message = SchemaAsIpc::new(plan.schema().as_arrow(), &IpcWriteOptions::default())
            .try_into()
            .map_err(|e| status!("Error encoding schema", e))?;
        let IpcMessage(schema_bytes) = message;

        Ok(ActionCreatePreparedStatementResult {
            prepared_statement_handle: Bytes::copy_from_slice(plan_id.as_bytes()),
            dataset_schema: schema_bytes,
            parameter_schema: Default::default(),
        })
    }

    #[instrument(
        skip_all,
        level = "info",
        fields(
            {crate::telemetry::mlflow::FIELD_SPAN_TYPE} = crate::telemetry::mlflow::SPAN_TYPE_WORKFLOW,
            {crate::telemetry::mlflow::FIELD_ZONE} = crate::telemetry::mlflow::ZONE_HYDROFOIL
        )
    )]
    async fn do_action_close_prepared_statement(
        &self,
        handle: ActionClosePreparedStatementRequest,
        request: Request<Action>,
    ) -> Result<(), Status> {
        if let Ok(handle) = Uuid::from_slice(&handle.prepared_statement_handle)
            && let Ok(session) = self.resolve_session(&request).await
        {
            session.statements().remove(&handle);
        }
        Ok(())
    }

    async fn register_sql_info(&self, _id: i32, _result: &SqlInfo) {}
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use datafusion_open_lineage::OpenLineageClient;
    use datafusion_open_lineage::context::LineageContext;
    use datafusion_open_lineage::event::{RunEvent, RunEventType};
    use datafusion_open_lineage::transport::{Transport, TransportError};
    use uuid::Uuid;

    use super::*;

    #[derive(Debug, Default, Clone)]
    struct Recorder {
        events: Arc<Mutex<Vec<RunEvent>>>,
    }

    #[async_trait::async_trait]
    impl Transport for Recorder {
        async fn emit(&self, event: &RunEvent) -> std::result::Result<(), TransportError> {
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
    }

    /// C9.1: a query that fails during *logical* planning (before any physical
    /// plan / OpenLineageExec exists) still surfaces to lineage as a single FAIL
    /// carrying the statement's run/job identity and the error message.
    #[tokio::test]
    async fn logical_planning_failure_emits_fail_event() {
        let recorder = Recorder::default();
        let client = OpenLineageClient::new(Arc::new(recorder.clone()));
        let service = FlightSqlServiceImpl::try_new()
            .unwrap()
            .with_lineage(client, OpenLineageConfig::default());

        let run_id = Uuid::now_v7();
        let lineage = LineageContext {
            run_id: Some(run_id),
            job_name: Some("query-bad".into()),
            sql: Some("SELECT * FROM nonexistent".into()),
            ..Default::default()
        };

        service.emit_planning_failure(&lineage, "table 'nonexistent' not found");

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let events = recorder.events.lock().unwrap();
        assert_eq!(events.len(), 1, "exactly one FAIL event");
        let e = &events[0];
        assert_eq!(e.event_type, RunEventType::Fail);
        assert_eq!(e.run.run_id, run_id, "FAIL carries the statement run id");
        assert_eq!(e.job.name, "query-bad", "FAIL carries the derived job name");
        let err = e
            .run
            .facets
            .error_message
            .as_ref()
            .expect("error facet present");
        assert!(err.message.contains("nonexistent"));
        // No plan resolved, so no input/output datasets are reported.
        assert!(e.inputs.is_empty() && e.outputs.is_empty());
    }

    /// Without a lineage client wired, the planning-failure path is a no-op (it
    /// must not panic).
    #[tokio::test]
    async fn planning_failure_without_lineage_is_noop() {
        let service = FlightSqlServiceImpl::try_new().unwrap();
        let lineage = LineageContext {
            run_id: Some(Uuid::now_v7()),
            ..Default::default()
        };
        service.emit_planning_failure(&lineage, "boom");
    }
}
