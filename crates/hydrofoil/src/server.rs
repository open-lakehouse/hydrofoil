use std::pin::Pin;
use std::sync::Arc;

use arrow::array::{ArrayRef, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::ipc::writer::IpcWriteOptions;
use arrow::record_batch::RecordBatch;
use arrow_flight::encode::FlightDataEncoderBuilder;
use arrow_flight::flight_descriptor::DescriptorType;
use arrow_flight::flight_service_server::FlightService;
use arrow_flight::sql::server::{FlightSqlService, PeekableFlightDataStream};
use arrow_flight::sql::{
    ActionClosePreparedStatementRequest, ActionCreatePreparedStatementRequest,
    ActionCreatePreparedStatementResult, Any, CommandGetCatalogs, CommandGetDbSchemas,
    CommandGetSqlInfo, CommandGetTables, CommandGetXdbcTypeInfo, CommandPreparedStatementQuery,
    CommandPreparedStatementUpdate, CommandStatementIngest, CommandStatementQuery,
    CommandStatementUpdate, DoPutUpdateResult, ProstMessageExt, SqlInfo, TicketStatementQuery,
};
use arrow_flight::{
    Action, FlightDescriptor, FlightEndpoint, FlightInfo, HandshakeRequest, HandshakeResponse,
    IpcMessage, PutResult, SchemaAsIpc, Ticket,
};
use dashmap::DashMap;
use datafusion::error::DataFusionError;
use datafusion::execution::SessionStateBuilder;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::{SQLOptions, SessionConfig, SessionContext};
use datafusion_tracing::{
    InstrumentationOptions, instrument_with_info_spans, pretty_format_compact_batch,
};
use deltalake_core::delta_datafusion::engine::AsObjectStoreUrl as _;
use deltalake_core::kernel::engine::arrow_conversion::TryIntoKernel;
use deltalake_core::protocol::SaveMode;
use deltalake_core::{DeltaTableBuilder, StructField};
use futures::{Stream, TryStreamExt};
use hydrofoil_common::conversion::{ConversionOptions, column_to_arrow};
use hydrofoil_common::{CreateDeltaTableMode, DeltaCommand, DeltaCommandType};
use itertools::Itertools as _;
use prost::Message;
use tonic::metadata::MetadataValue;
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, dispatcher, info, instrument};
use uuid::Uuid;

use crate::execution::CpuRuntime;
use crate::storage::update_session;
use crate::stream::FlightDataReceiverStreamBuilder;

mod metadata;

macro_rules! status {
    ($desc:expr, $err:expr) => {
        Status::internal(format!("{}: {} at {}:{}", $desc, $err, file!(), line!()))
    };
}

pub struct FlightSqlServiceImpl {
    pub(crate) contexts: Arc<DashMap<String, Arc<SessionContext>>>,
    pub(crate) statements: Arc<DashMap<String, LogicalPlan>>,
    pub(crate) results: Arc<DashMap<String, Vec<RecordBatch>>>,

    cpu_runtime: CpuRuntime,
}

impl FlightSqlServiceImpl {
    pub fn try_new() -> Result<Self, DataFusionError> {
        Ok(Self {
            contexts: Arc::new(DashMap::new()),
            statements: Arc::new(DashMap::new()),
            results: Arc::new(DashMap::new()),
            cpu_runtime: CpuRuntime::try_new()?,
        })
    }

    fn create_ctx(&self) -> Result<String, Status> {
        let uuid = Uuid::new_v4().hyphenated().to_string();
        let session_config = SessionConfig::from_env()
            .map_err(|e| Status::internal(format!("Error building plan: {e}")))?
            .with_information_schema(true);
        let ctx = Arc::new(SessionContext::new_with_config(session_config));

        self.contexts.insert(uuid.clone(), ctx);

        Ok(uuid)
    }

    #[allow(clippy::result_large_err)]
    fn get_ctx<T>(&self, req: &Request<T>) -> Result<Arc<SessionContext>, Status> {
        // get the token from the authorization header on Request
        // let auth = req
        //     .metadata()
        //     .get("authorization")
        //     .ok_or_else(|| Status::internal("No authorization header!"))?;
        // let str = auth
        //     .to_str()
        //     .map_err(|e| Status::internal(format!("Error parsing header: {e}")))?;
        // let authorization = str.to_string();
        // let bearer = "Bearer ";
        // if !authorization.starts_with(bearer) {
        //     Err(Status::internal("Invalid auth header!"))?;
        // }
        // let auth = authorization[bearer.len()..].to_string();

        // if let Some(context) = self.contexts.get(&auth) {
        //     Ok(context.clone())
        // } else {
        //     let context = self.create_ctx()?;
        //     Err(Status::internal(format!(
        //         "Context handle not found: {auth}"
        //     )))?
        // }

        if let Some(ctx) = self.contexts.get("key") {
            Ok(ctx.value().clone())
        } else {
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

            let session_config = SessionConfig::from_env()
                .map_err(|e| Status::internal(format!("Error building plan: {e}")))?
                .with_information_schema(true);
            let session_state = SessionStateBuilder::new_with_default_features()
                .with_config(session_config)
                .with_physical_optimizer_rule(instrument_rule)
                .build();
            let ctx = Arc::new(SessionContext::new_with_state(session_state));
            update_session(&ctx.state()).map_err(|e| status!("Failed to update session", e))?;
            self.contexts.insert("key".to_string(), ctx.clone());
            Ok(ctx)
        }
    }

    #[allow(clippy::result_large_err)]
    fn get_plan(&self, handle: &str) -> Result<LogicalPlan, Status> {
        if let Some(plan) = self.statements.get(handle) {
            Ok(plan.clone())
        } else {
            Err(Status::internal(format!("Plan handle not found: {handle}")))?
        }
    }

    async fn tables(&self, ctx: Arc<SessionContext>) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("catalog_name", DataType::Utf8, true),
            Field::new("db_schema_name", DataType::Utf8, true),
            Field::new("table_name", DataType::Utf8, false),
            Field::new("table_type", DataType::Utf8, false),
        ]));

        let mut catalogs = vec![];
        let mut schemas = vec![];
        let mut names = vec![];
        let mut types = vec![];
        for catalog in ctx.catalog_names() {
            let catalog_provider = ctx.catalog(&catalog).unwrap();
            for schema in catalog_provider.schema_names() {
                let schema_provider = catalog_provider.schema(&schema).unwrap();
                for table in schema_provider.table_names() {
                    let table_provider = schema_provider.table(&table).await.unwrap().unwrap();
                    catalogs.push(catalog.clone());
                    schemas.push(schema.clone());
                    names.push(table.clone());
                    types.push(table_provider.table_type().to_string())
                }
            }
        }

        RecordBatch::try_new(
            schema,
            [catalogs, schemas, names, types]
                .into_iter()
                .map(|i| Arc::new(StringArray::from(i)) as ArrayRef)
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    #[allow(clippy::result_large_err)]
    fn remove_plan(&self, handle: &str) -> Result<(), Status> {
        self.statements.remove(&handle.to_string());
        Ok(())
    }

    #[allow(clippy::result_large_err)]
    fn remove_result(&self, handle: &str) -> Result<(), Status> {
        self.results.remove(&handle.to_string());
        Ok(())
    }
}

#[tonic::async_trait]
impl FlightSqlService for FlightSqlServiceImpl {
    type FlightService = FlightSqlServiceImpl;

    async fn do_handshake(
        &self,
        _request: Request<Streaming<HandshakeRequest>>,
    ) -> Result<
        Response<Pin<Box<dyn Stream<Item = Result<HandshakeResponse, Status>> + Send>>>,
        Status,
    > {
        info!("do_handshake");
        // no authentication actually takes place here
        // see Ballista implementation for example of basic auth
        // in this case, we simply accept the connection and create a new SessionContext
        // the SessionContext will be re-used within this same connection/session
        let token = self.create_ctx()?;

        let result = HandshakeResponse {
            protocol_version: 0,
            payload: token.as_bytes().to_vec().into(),
        };
        let result = Ok(result);
        let output = futures::stream::iter(vec![result]);
        let str = format!("Bearer {token}");
        let mut resp: Response<Pin<Box<dyn Stream<Item = Result<_, _>> + Send>>> =
            Response::new(Box::pin(output));
        let md = MetadataValue::try_from(str)
            .map_err(|_| Status::invalid_argument("authorization not parsable"))?;
        resp.metadata_mut().insert("authorization", md);
        Ok(resp)
    }

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
    #[instrument(skip(self, request))]
    async fn get_flight_info_statement(
        &self,
        query: CommandStatementQuery,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let ctx = self.get_ctx(&request)?;
        let plan = ctx
            .state()
            .create_logical_plan(&query.query)
            .await
            .map_err(|e| Status::internal(format!("Error building plan: {e}")))?;
        let options = SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_dml(false);
        options
            .verify_plan(&plan)
            .map_err(|e| Status::internal(format!("{e:?}")))?;

        let plan_uuid = Uuid::new_v4().hyphenated().to_string();
        self.statements.insert(plan_uuid.clone(), plan.clone());

        let fetch = FetchResults {
            handle: plan_uuid.to_string(),
        };
        let buf = fetch.as_any().encode_to_vec().into();
        let ticket = Ticket { ticket: buf };

        let info = FlightInfo::new()
            .try_with_schema(plan.schema().as_arrow())
            .expect("encoding failed")
            .with_endpoint(FlightEndpoint::new().with_ticket(ticket))
            .with_descriptor(FlightDescriptor {
                r#type: DescriptorType::Cmd.into(),
                cmd: Default::default(),
                path: vec![],
            });

        Ok(Response::new(info))
    }

    async fn get_flight_info_prepared_statement(
        &self,
        cmd: CommandPreparedStatementQuery,
        _request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        debug!("get_flight_info_prepared_statement");

        let handle = std::str::from_utf8(&cmd.prepared_statement_handle)
            .map_err(|e| status!("Unable to parse uuid", e))?;

        let plan = self.get_plan(handle)?;

        let fetch = FetchResults {
            handle: handle.to_string(),
        };
        let buf = fetch.as_any().encode_to_vec().into();
        let ticket = Ticket { ticket: buf };

        let info = FlightInfo::new()
            .try_with_schema(plan.schema().as_arrow())
            .expect("encoding failed")
            .with_endpoint(FlightEndpoint::new().with_ticket(ticket))
            .with_descriptor(FlightDescriptor {
                r#type: DescriptorType::Cmd.into(),
                cmd: Default::default(),
                path: vec![],
            });

        Ok(Response::new(info))
    }

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

    async fn do_get_statement(
        &self,
        _ticket: TicketStatementQuery,
        _request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        Err(Status::unimplemented(
            "do_get_statement has no default implementation",
        ))
    }

    async fn do_get_prepared_statement(
        &self,
        _query: CommandPreparedStatementQuery,
        _request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        Err(Status::unimplemented(
            "do_get_prepared_statement has no default implementation",
        ))
    }

    #[instrument(skip(self, request))]
    async fn do_get_fallback(
        &self,
        request: Request<Ticket>,
        message: Any,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        if !message.is::<FetchResults>() {
            Err(Status::unimplemented(format!(
                "do_get: The defined request is invalid: {}",
                message.type_url
            )))?
        }

        let fr: FetchResults = message
            .unpack()
            .map_err(|e| Status::internal(format!("{e:?}")))?
            .ok_or_else(|| Status::internal("Expected FetchResults but got None!"))?;

        let handle = fr.handle;
        info!("getting results for {handle}");

        let ctx = self.get_ctx(&request)?;

        // retrieve the plan and verify it is safe to execute
        let plan = self.get_plan(handle.as_str())?;
        let options = SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_dml(false);
        options
            .verify_plan(&plan)
            .map_err(|e| Status::internal(format!("{e:?}")))?;

        let mut builder = FlightDataReceiverStreamBuilder::new(100);
        builder.execute_logical_plan(Arc::new(ctx.state()), plan, self.cpu_runtime.handle());
        let stream = builder.build().map_err(Status::from);

        Ok(Response::new(Box::pin(stream)))
    }

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

    async fn do_put_statement_ingest(
        &self,
        _ticket: CommandStatementIngest,
        _request: Request<PeekableFlightDataStream>,
    ) -> Result<i64, Status> {
        Err(Status::unimplemented(
            "do_put_statement_ingest not implemented",
        ))
    }

    #[instrument(skip(self, request))]
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

        let Some(command_type) = &command.command_type else {
            return Err(Status::internal("DeltaCommand has no command_type"));
        };

        let ctx = self.get_ctx(&request)?;

        match command_type {
            DeltaCommandType::CreateDeltaTable(create) => {
                info!("Creating Delta table: {:?}", create);
                let table_url = url::Url::parse(create.location.as_ref().unwrap()).unwrap();
                let store_url = table_url.as_object_store_url();
                let root_storage = ctx
                    .runtime_env()
                    .object_store(&store_url)
                    .map_err(|e| status!("failed to get object store", e))?
                    .clone();
                let table = DeltaTableBuilder::from_url(table_url.clone())
                    .map_err(|e| status!("failed to get object store", e))?
                    .with_storage_backend(root_storage, table_url)
                    .build()
                    .map_err(|e| status!("failed to get object store", e))?;

                let mut builder = table.create();
                // if let Some(location) = &create.location {
                //     builder = builder.with_location(location.clone());
                // }
                if let Some(table_name) = &create.table_name {
                    builder = builder.with_table_name(table_name.clone());
                }
                if let Some(comment) = &create.comment {
                    builder = builder.with_comment(comment.clone());
                }
                if !create.partitioning_columns.is_empty() {
                    builder = builder.with_partition_columns(&create.partitioning_columns);
                }
                let mode = match create.mode() {
                    CreateDeltaTableMode::Create => SaveMode::ErrorIfExists,
                    CreateDeltaTableMode::Replace | CreateDeltaTableMode::CreateOrReplace => {
                        SaveMode::Overwrite
                    }
                    CreateDeltaTableMode::CreateIfNotExists => SaveMode::Ignore,
                    _ => {
                        return Err(Status::invalid_argument(format!(
                            "Invalid CreateMode: {}",
                            create.mode
                        )));
                    }
                };
                builder = builder.with_save_mode(mode);

                let fields: Vec<StructField> = create
                    .columns
                    .iter()
                    .map(|c| {
                        column_to_arrow(c, &ConversionOptions::default())
                            .and_then(|f| (&f).try_into_kernel())
                    })
                    .try_collect()
                    .map_err(|e| {
                        Status::internal(format!("Error converting columns to Arrow: {e}"))
                    })?;
                builder = builder.with_columns(fields);

                let dispatch = dispatcher::get_default(|d| d.clone());
                let span = tracing::Span::current();

                let handle = self.cpu_runtime.handle().spawn(async move {
                    dispatcher::with_default(&dispatch, || async {
                        let _enter = span.enter();
                        info!("running delta table creation task");
                        builder.await
                    })
                    .await
                });
                let _table = handle
                    .await
                    .map_err(|e| Status::internal(format!("{e}")))?
                    .map_err(|e| Status::internal(format!("{e}")))?;
            }
            _ => {
                return Err(Status::unimplemented(format!(
                    "DeltaCommandType {:?} not implemented",
                    command_type
                )));
            }
        }

        let result = DoPutUpdateResult { record_count: -1 };
        let output = futures::stream::iter(vec![Ok(PutResult {
            app_metadata: result.encode_to_vec().into(),
        })]);
        Ok(Response::new(Box::pin(output)))
    }

    async fn do_action_create_prepared_statement(
        &self,
        query: ActionCreatePreparedStatementRequest,
        request: Request<Action>,
    ) -> Result<ActionCreatePreparedStatementResult, Status> {
        debug!("do_action_create_prepared_statement");

        let user_query = query.query.as_str();

        let ctx = self.get_ctx(&request)?;

        let plan = ctx
            .state()
            .create_logical_plan(user_query)
            .await
            .map_err(|e| Status::internal(format!("Error building plan: {e}")))?;

        // let plan = ctx
        //     .sql(user_query)
        //     .await
        //     .and_then(|df| df.into_optimized_plan())
        //     .map_err(|e| Status::internal(format!("Error building plan: {e}")))?;

        // store a copy of the plan,  it will be used for execution
        let plan_uuid = Uuid::new_v4().hyphenated().to_string();
        self.statements.insert(plan_uuid.clone(), plan.clone());

        let plan_schema = plan.schema();

        let arrow_schema = plan_schema.as_arrow();
        let message = SchemaAsIpc::new(arrow_schema, &IpcWriteOptions::default())
            .try_into()
            .map_err(|e| status!("Unable to serialize schema", e))?;
        let IpcMessage(schema_bytes) = message;

        let res = ActionCreatePreparedStatementResult {
            prepared_statement_handle: plan_uuid.into(),
            dataset_schema: schema_bytes,
            parameter_schema: Default::default(),
        };
        Ok(res)
    }

    async fn do_action_close_prepared_statement(
        &self,
        handle: ActionClosePreparedStatementRequest,
        _request: Request<Action>,
    ) -> Result<(), Status> {
        let handle = std::str::from_utf8(&handle.prepared_statement_handle);
        if let Ok(handle) = handle {
            info!("do_action_close_prepared_statement: removing plan and results for {handle}");
            let _ = self.remove_plan(handle);
            let _ = self.remove_result(handle);
        }
        Ok(())
    }

    async fn register_sql_info(&self, _id: i32, _result: &SqlInfo) {}
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FetchResults {
    #[prost(string, tag = "1")]
    pub handle: ::prost::alloc::string::String,
}

impl ProstMessageExt for FetchResults {
    fn type_url() -> &'static str {
        "type.googleapis.com/datafusion.example.com.sql.FetchResults"
    }

    fn as_any(&self) -> Any {
        Any {
            type_url: FetchResults::type_url().to_string(),
            value: ::prost::Message::encode_to_vec(self).into(),
        }
    }
}
