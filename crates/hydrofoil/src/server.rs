use std::sync::Arc;

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
    Action, FlightDescriptor, FlightEndpoint, FlightInfo, IpcMessage, PutResult, SchemaAsIpc,
    Ticket, encode::FlightDataEncoderBuilder, flight_descriptor::DescriptorType,
    flight_service_server::FlightService,
};
use bytes::Bytes;
use dashmap::DashMap;
use datafusion::error::DataFusionError;
use datafusion::logical_expr::LogicalPlan;
use datafusion::prelude::{SQLOptions, SessionContext};
use futures::TryStreamExt;
use hydrofoil_common::DeltaCommand;
use prost::Message;
use tonic::{Request, Response, Status};
use tracing::{debug, info, instrument};
use uuid::Uuid;

use crate::execution::CpuRuntime;
use crate::planner::{DeltaPlanner, FlightPlanner};
use crate::session::create_session;
use crate::stream::FlightDataReceiverStreamBuilder;

mod metadata;

macro_rules! status {
    ($desc:expr, $err:expr) => {
        Status::internal(format!("{}: {} at {}:{}", $desc, $err, file!(), line!()))
    };
}

pub struct FlightSqlServiceImpl {
    pub(crate) contexts: Arc<DashMap<String, Arc<SessionContext>>>,
    pub(crate) statements: Arc<DashMap<Uuid, LogicalPlan>>,

    executor: CpuRuntime,
}

impl FlightSqlServiceImpl {
    pub fn try_new() -> Result<Self, DataFusionError> {
        Ok(Self {
            contexts: Arc::new(DashMap::new()),
            statements: Arc::new(DashMap::new()),
            executor: CpuRuntime::try_new()?,
        })
    }

    #[allow(clippy::result_large_err)]
    fn get_ctx<T>(&self, _req: &Request<T>) -> Result<Arc<SessionContext>, Status> {
        if let Some(ctx) = self.contexts.get("key") {
            Ok(ctx.value().clone())
        } else {
            let session_id = Uuid::new_v4();
            let ctx = Arc::new(
                create_session(session_id).map_err(|e| status!("Failed to create session", e))?,
            );
            self.contexts.insert("key".to_string(), ctx.clone());
            Ok(ctx)
        }
    }

    #[allow(clippy::result_large_err)]
    fn get_plan(&self, handle: &Uuid) -> Result<LogicalPlan, Status> {
        if let Some(plan) = self.statements.get(handle) {
            Ok(plan.clone())
        } else {
            Err(Status::internal(format!("Plan handle not found: {handle}")))?
        }
    }

    #[allow(clippy::result_large_err)]
    fn remove_plan(&self, handle: &Uuid) -> Result<(), Status> {
        self.statements.remove(handle);
        Ok(())
    }
}

#[tonic::async_trait]
impl FlightSqlService for FlightSqlServiceImpl {
    type FlightService = FlightSqlServiceImpl;

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
        let ctx = self.get_ctx(&request)?;

        let plan = self
            .executor
            .create_logical_plan(ctx, query.query.clone())
            .await
            .map_err(|e| Status::internal(format!("Error building plan: {e}")))?;

        let options = SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_dml(false);
        options
            .verify_plan(&plan)
            .map_err(|e| Status::internal(format!("{e:?}")))?;

        let plan_id = Uuid::now_v7();
        self.statements.insert(plan_id.clone(), plan.clone());

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
        _request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let handle = Uuid::from_slice(&cmd.prepared_statement_handle)
            .map_err(|e| status!("Invalid handle", e))?;
        let plan = self.get_plan(&handle)?;

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
        _ticket: TicketStatementQuery,
        _request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        Err(Status::unimplemented(
            "do_get_statement has no default implementation",
        ))
    }

    #[instrument(skip_all, level = "info")]
    async fn do_get_prepared_statement(
        &self,
        query: CommandPreparedStatementQuery,
        request: Request<Ticket>,
    ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
        let options = SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_dml(false);
        let handle = Uuid::from_slice(&query.prepared_statement_handle)
            .map_err(|e| status!("Invalid handle", e))?;

        let plan = self.get_plan(&handle)?;
        options
            .verify_plan(&plan)
            .map_err(|e| Status::internal(format!("{e:?}")))?;

        let ctx = self.get_ctx(&request)?;
        let mut builder = FlightDataReceiverStreamBuilder::new(100);
        builder.execute_logical_plan(Arc::new(ctx.state()), plan, self.executor.handle());
        let stream = builder.build().map_err(Status::from);

        Ok(Response::new(Box::pin(stream)))
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
        let ctx = self.get_ctx(&request)?;
        let planner = FlightPlanner::new();

        let ctx_inner = ctx.clone();
        let plan = self
            .executor
            .spawn(async move {
                planner
                    .plan_ingest(&ctx_inner, &ticket, request.into_inner())
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

        let ctx = self.get_ctx(&request)?;
        let planner = DeltaPlanner::new();
        let plan = planner
            .plan_delta_connect(&ctx.state(), &command)
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
        let ctx = self.get_ctx(&request)?;

        let plan_id = Uuid::now_v7();
        let plan = self
            .executor
            .create_logical_plan(ctx, query.query)
            .await
            .map_err(|e| status!("Error building plan", e))?;

        self.statements.insert(plan_id.clone(), plan.clone());
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
        _request: Request<Action>,
    ) -> Result<(), Status> {
        if let Ok(handle) = Uuid::from_slice(&handle.prepared_statement_handle) {
            let _ = self.remove_plan(&handle);
        }
        Ok(())
    }

    async fn register_sql_info(&self, _id: i32, _result: &SqlInfo) {}
}
