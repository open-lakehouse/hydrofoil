use arrow_flight::sql::client::FlightSqlServiceClient;
use arrow_flight::sql::{DoPutUpdateResult, ProstMessageExt};
use arrow_flight::{FlightData, FlightDescriptor};
use arrow_flight::{FlightInfo, flight_service_client::FlightServiceClient};
use arrow_schema::{ArrowError, Fields, Schema};
use bytes::Bytes;
use futures::future::BoxFuture;
use hydrofoil_common::conversion::field_to_connect;
use hydrofoil_common::{
    AddFeatureSupport, CreateDeltaTable, CreateDeltaTableMode, DeltaCommand, DeltaCommandType,
    VacuumTable,
};
use itertools::Itertools as _;
use prost::Message as _;
use tonic::IntoRequest as _;
use tonic::transport::{Channel, Endpoint};

use crate::error::{Result, decode_error_to_arrow_error, status_to_arrow_error};

mod error;

#[derive(Debug, Clone)]
pub struct Client {
    client: FlightSqlServiceClient<Channel>,
}

impl Client {
    pub async fn try_new<D>(endpoint: D) -> Result<Self>
    where
        D: TryInto<Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        let endpoint = Endpoint::new(endpoint)?;
        let channel = endpoint.connect().await?;
        let inner = FlightServiceClient::new(channel);
        let client = FlightSqlServiceClient::new_from_inner(inner);
        Ok(Self { client })
    }

    pub async fn handshake(&mut self) -> Result<()> {
        let result = self.client.handshake("user", "password").await?;
        println!("Handshake result: {:?}", result);
        Ok(())
    }

    /// Execute a query on the server.
    pub async fn execute(
        &mut self,
        query: impl ToString,
        transaction_id: impl Into<Option<Bytes>>,
    ) -> Result<FlightInfo, ArrowError> {
        self.client
            .execute(query.to_string(), transaction_id.into())
            .await
    }

    pub fn create_delta_table(&self) -> CreateDeltaTableBuilder {
        CreateDeltaTableBuilder::new(self.client.clone())
    }
}

pub struct CreateDeltaTableBuilder {
    client: FlightSqlServiceClient<Channel>,
    message: CreateDeltaTable,
}

impl CreateDeltaTableBuilder {
    fn new(client: FlightSqlServiceClient<Channel>) -> Self {
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
    type Output = Result<()>;
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

#[cfg(test)]
mod tests {
    use arrow_schema::{DataType, Field};

    use super::*;

    #[tokio::test]
    async fn it_works() {
        let mut client = Client::try_new("http://localhost:50051").await.unwrap();
        let schema = Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("value", DataType::Utf8View, false),
        ]);
        let result = client
            .create_delta_table()
            .with_location("s3://open-lakehouse/test_table/")
            .with_schema(&schema)
            .unwrap()
            .await
            .unwrap();
    }
}
