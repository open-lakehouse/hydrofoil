use arrow_flight::{
    FlightData, FlightDescriptor,
    sql::{DoPutUpdateResult, ProstMessageExt, client::FlightSqlServiceClient},
};
use arrow_schema::{ArrowError, Fields, Schema};
use futures::future::BoxFuture;
use hydrofoil_common::{
    CreateDeltaTable, CreateDeltaTableMode, DeltaCommand, DeltaCommandType,
    conversion::field_to_connect,
};
use itertools::Itertools as _;
use prost::Message as _;
use tonic::{IntoRequest as _, transport::Channel};

use crate::error::{decode_error_to_arrow_error, status_to_arrow_error};

pub struct CreateDeltaTableBuilder {
    client: FlightSqlServiceClient<Channel>,
    message: CreateDeltaTable,
}

impl CreateDeltaTableBuilder {
    pub(crate) fn new(client: FlightSqlServiceClient<Channel>) -> Self {
        let mut message = CreateDeltaTable::default();
        message.set_mode(CreateDeltaTableMode::Create);
        Self { client, message }
    }

    pub fn with_mode(mut self, mode: CreateDeltaTableMode) -> Self {
        self.message.set_mode(mode);
        self
    }

    pub fn with_location<S: Into<String>>(mut self, path: S) -> Self {
        self.message.location = Some(path.into());
        self
    }

    pub fn with_table_name<S: Into<String>>(mut self, name: S) -> Self {
        self.message.table_name = Some(name.into());
        self
    }

    pub fn with_comment<S: Into<String>>(mut self, comment: S) -> Self {
        self.message.comment = Some(comment.into());
        self
    }

    pub fn with_partition_columns<S: Into<String>>(
        mut self,
        columns: impl IntoIterator<Item = S>,
    ) -> Self {
        self.message.partitioning_columns = columns.into_iter().map(|s| s.into()).collect();
        self
    }

    pub fn with_clustering_columns<S: Into<String>>(
        mut self,
        columns: impl IntoIterator<Item = S>,
    ) -> Self {
        self.message.clustering_columns = columns.into_iter().map(|s| s.into()).collect();
        self
    }

    pub fn with_columns(mut self, fields: impl Into<Fields>) -> Result<Self, ArrowError> {
        self.message.columns = fields
            .into()
            .iter()
            .map(|f| field_to_connect(f))
            .try_collect()?;
        Ok(self)
    }

    pub fn with_schema(mut self, schema: &Schema) -> Result<Self, ArrowError> {
        self.message.columns = schema
            .fields()
            .iter()
            .map(|f| field_to_connect(f))
            .try_collect()?;
        Ok(self)
    }
}

impl IntoFuture for CreateDeltaTableBuilder {
    type Output = Result<(), ArrowError>;
    type IntoFuture = BoxFuture<'static, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        let mut client = self.client;
        let message = self.message;

        Box::pin(async move {
            let command = DeltaCommand {
                command_type: Some(DeltaCommandType::CreateDeltaTable(message)),
            };
            let descriptor = FlightDescriptor::new_cmd(command.as_any().encode_to_vec());
            // let req = self.client.set_request_headers(
            //     stream::iter(vec![FlightData {
            //         flight_descriptor: Some(descriptor),
            //         ..Default::default()
            //     }])
            //     .into_request(),
            // )?;
            let req = futures::stream::iter(vec![FlightData {
                flight_descriptor: Some(descriptor),
                ..Default::default()
            }])
            .into_request();

            let mut result = client.do_put(req).await?;
            let result = result
                .message()
                .await
                .map_err(status_to_arrow_error)?
                .unwrap();
            let _result = DoPutUpdateResult::decode(&*result.app_metadata)
                .map_err(decode_error_to_arrow_error)?;

            Ok(())
        })
    }
}
