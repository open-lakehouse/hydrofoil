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
use datafusion_open_lineage::OpenLineageClient;
use datafusion::error::DataFusionError;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::SQLOptions;
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

/// Default idle TTL for sessions when `HYDROFOIL_SESSION_TTL_SECS` is unset.
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
}

impl FlightSqlServiceImpl {
    pub fn try_new() -> Result<Self, DataFusionError> {
        let policy: Arc<dyn Policy> = Arc::new(StaticPolicy::new(Decision::Allow));
        // A placeholder store; replaced by `build()` once the optional
        // components (lineage/policy/unity) are configured.
        let engine = Engine::new(policy.clone(), None, None);
        let ttl = Duration::from_secs(DEFAULT_SESSION_TTL_SECS);
        Ok(Self {
            sessions: SessionStore::new(engine, ttl),
            executor: CpuRuntime::try_new()?,
            policy,
            unity_factory: None,
            lineage: None,
        })
    }

    /// Attach an OpenLineage client so sessions emit lineage events around
    /// query planning.
    pub fn with_lineage(mut self, lineage: OpenLineageClient) -> Self {
        self.lineage = Some(lineage);
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

    /// Finalize the configured components into an [`Engine`] + [`SessionStore`]
    /// and start the background session sweeper. Call once before serving.
    pub fn build(mut self) -> Self {
        let ttl = std::env::var("HYDROFOIL_SESSION_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(DEFAULT_SESSION_TTL_SECS));
        let engine = Engine::new(
            self.policy.clone(),
            self.unity_factory.clone(),
            self.lineage.clone(),
        );
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
    fn resolve_session<T>(&self, req: &Request<T>) -> Result<Arc<Session>, Status> {
        if let Some(session) =
            session_id_from_metadata(req.metadata()).and_then(|id| self.sessions.get(&id))
        {
            return Ok(session);
        }
        // No session id, or a known-shaped but unknown/expired one: fall through
        // to the principal's ephemeral session rather than hard-failing.
        let principal = crate::identity::principal_from_metadata(req.metadata())?;
        self.sessions
            .ephemeral_for(principal)
            .map_err(|e| status!("Failed to resolve session", e))
    }

    /// Build the per-request lineage context: parent-run facet parsed from
    /// metadata, the pinned `run_id`, and the statement's SQL text.
    fn lineage_context<T>(
        &self,
        req: &Request<T>,
        run_id: Uuid,
        sql: Option<String>,
    ) -> datafusion_open_lineage::context::LineageContext {
        let mut ctx = crate::lineage::context_from_metadata(
            req.metadata(),
            &datafusion_open_lineage::config::OpenLineageConfig::default(),
        );
        ctx.run_id = Some(run_id);
        ctx.sql = sql;
        ctx
    }

    fn do_get_handle(
        &self,
        session: Arc<dyn datafusion::catalog::Session>,
        plan: LogicalPlan,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
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
    #[instrument(skip_all, level = "info")]
    async fn do_handshake(
        &self,
        request: Request<Streaming<HandshakeRequest>>,
    ) -> Result<
        Response<Pin<Box<dyn Stream<Item = Result<HandshakeResponse, Status>> + Send>>>,
        Status,
    > {
        let principal = crate::identity::principal_from_metadata(request.metadata())?;
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

    #[instrument(skip_all, level = "info")]
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

    #[instrument(skip_all, level = "info")]
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

    #[instrument(skip_all, level = "info")]
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

    #[instrument(skip_all, level = "info")]
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

    #[instrument(skip_all, level = "info")]
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
    #[instrument(skip_all, level = "info", fields(query = query.query.as_str()))]
    async fn get_flight_info_statement(
        &self,
        query: CommandStatementQuery,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let session = self.resolve_session(&request)?;

        // Mint the lineage run id now and snapshot the lineage context from this
        // (planning) RPC. Planning runs through a session decorated with that
        // context so the OpenLineage START event carries the pinned run id; the
        // later do_get reuses the same snapshot so COMPLETE/FAIL correlate. See
        // docs/adr/0003-per-statement-run-id-correlation.md.
        let run_id = Uuid::now_v7();
        let lineage = self.lineage_context(&request, run_id, Some(query.query.clone()));
        let agent = crate::agent::agent_context_from_metadata(request.metadata());
        let lh = session.lakehouse_for_query(lineage.clone(), agent);

        let plan = self
            .executor
            .create_logical_plan(lh, query.query.clone())
            .await
            .map_err(|e| Status::internal(format!("Error building plan: {e}")))?;

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

    #[instrument(skip_all, level = "info")]
    async fn get_flight_info_prepared_statement(
        &self,
        cmd: CommandPreparedStatementQuery,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let handle = Uuid::from_slice(&cmd.prepared_statement_handle)
            .map_err(|e| status!("Invalid handle", e))?;
        let session = self.resolve_session(&request)?;
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

    #[instrument(skip_all, level = "info")]
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

    #[instrument(skip_all, level = "info")]
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

    #[instrument(skip_all, level = "info")]
    async fn do_get_statement(
        &self,
        ticket: TicketStatementQuery,
        request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let handle =
            Uuid::from_slice(&ticket.statement_handle).map_err(|e| status!("Invalid handle", e))?;
        let session = self.resolve_session(&request)?;
        let stored = session
            .statements()
            .get(&handle)
            .map(|s| s.clone())
            .ok_or_else(|| Status::internal(format!("Plan handle not found: {handle}")))?;

        // Reuse the run_id + lineage snapshot from planning so START and
        // COMPLETE/FAIL share one runId; layer this request's agent context on top.
        let agent = crate::agent::agent_context_from_metadata(request.metadata());
        let lh = session.lakehouse_for_query(stored.lineage.clone(), agent);
        let result = self.do_get_handle(Arc::new(lh), stored.plan.clone());
        session.statements().remove(&handle);
        result
    }

    #[instrument(skip_all, level = "info")]
    async fn do_get_prepared_statement(
        &self,
        query: CommandPreparedStatementQuery,
        request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let handle = Uuid::from_slice(&query.prepared_statement_handle)
            .map_err(|e| status!("Invalid handle", e))?;
        let session = self.resolve_session(&request)?;
        let stored = session
            .statements()
            .get(&handle)
            .map(|s| s.clone())
            .ok_or_else(|| Status::internal(format!("Plan handle not found: {handle}")))?;
        // Prepared statements are removed on ClosePreparedStatement, not here.
        let agent = crate::agent::agent_context_from_metadata(request.metadata());
        let lh = session.lakehouse_for_query(stored.lineage.clone(), agent);
        self.do_get_handle(Arc::new(lh), stored.plan.clone())
    }

    #[instrument(skip_all, level = "info", fields(message_type_url = message.type_url.as_str()))]
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

    #[instrument(skip_all, level = "info")]
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

    #[instrument(skip_all, level = "info")]
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
        )
    )]
    async fn do_put_statement_ingest(
        &self,
        ticket: CommandStatementIngest,
        request: Request<PeekableFlightDataStream>,
    ) -> Result<i64, Status> {
        let session = self.resolve_session(&request)?;
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

    #[instrument(skip_all, level = "info", fields(message_type_url = message.type_url.as_str()))]
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

        let session = self.resolve_session(&request)?;
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

    #[instrument(skip_all, level = "info", fields(query = query.query.as_str()))]
    async fn do_action_create_prepared_statement(
        &self,
        query: ActionCreatePreparedStatementRequest,
        request: Request<Action>,
    ) -> Result<ActionCreatePreparedStatementResult, Status> {
        let session = self.resolve_session(&request)?;

        let plan_id = Uuid::now_v7();
        let run_id = Uuid::now_v7();
        let lineage = self.lineage_context(&request, run_id, Some(query.query.clone()));
        let agent = crate::agent::agent_context_from_metadata(request.metadata());
        let lh = session.lakehouse_for_query(lineage.clone(), agent);

        let plan = self
            .executor
            .create_logical_plan(lh, query.query)
            .await
            .map_err(|e| status!("Error building plan", e))?;

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

    #[instrument(skip_all, level = "info")]
    async fn do_action_close_prepared_statement(
        &self,
        handle: ActionClosePreparedStatementRequest,
        request: Request<Action>,
    ) -> Result<(), Status> {
        if let Ok(handle) = Uuid::from_slice(&handle.prepared_statement_handle)
            && let Ok(session) = self.resolve_session(&request)
        {
            session.statements().remove(&handle);
        }
        Ok(())
    }

    async fn register_sql_info(&self, _id: i32, _result: &SqlInfo) {}
}
