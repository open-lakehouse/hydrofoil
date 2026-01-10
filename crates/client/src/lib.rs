use arrow_array::RecordBatch;
use arrow_flight::Ticket;
use arrow_flight::decode::FlightRecordBatchStream;
use arrow_flight::sql::client::{FlightSqlServiceClient, PreparedStatement};
use arrow_flight::{FlightInfo, flight_service_client::FlightServiceClient};
use arrow_schema::ArrowError;
use bytes::Bytes;
use datafusion_common::TableReference;
use futures::Stream;
use tonic::IntoRequest;
use tonic::transport::{Channel, Endpoint};

use crate::commands::{CreateDeltaTableBuilder, IngestBuilder};
use crate::error::Result;

pub use arrow_flight::sql::{TableExistsOption, TableNotExistOption};

mod commands;
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

    pub async fn prepare(
        &mut self,
        query: impl ToString,
        transaction_id: impl Into<Option<Bytes>>,
    ) -> Result<PreparedStatement<Channel>, ArrowError> {
        self.client
            .prepare(query.to_string(), transaction_id.into())
            .await
    }

    pub async fn do_get(
        &mut self,
        ticket: impl IntoRequest<Ticket>,
    ) -> Result<FlightRecordBatchStream, ArrowError> {
        self.client.do_get(ticket).await
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

    pub fn ingest<S>(&self, table: impl Into<TableReference>, stream: S) -> IngestBuilder<S>
    where
        S: Stream<Item = Result<RecordBatch, ArrowError>> + Send + 'static,
    {
        let table = table.into();
        let mut builder =
            IngestBuilder::new(self.client.clone(), stream).with_table_name(table.table());
        if let Some(schema) = table.schema() {
            builder = builder.with_schema_name(schema);
        }
        if let Some(catalog) = table.catalog() {
            builder = builder.with_catalog_name(catalog);
        }
        builder
    }

    pub fn create_delta_table(&self) -> CreateDeltaTableBuilder {
        CreateDeltaTableBuilder::new(self.client.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::{Int64Array, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use futures::TryStreamExt as _;

    use super::*;

    #[tokio::test]
    async fn it_works_too() {
        let mut client = Client::try_new("http://localhost:50051").await.unwrap();

        let mut stmt = client
            .prepare("SELECT 1, 2.0, 'Hello, world!'", None)
            .await
            .unwrap();

        let flight_info = stmt.execute().await.unwrap();

        let ticket = flight_info.endpoint[0].ticket.as_ref().unwrap().clone();
        let flight_data = client.do_get(ticket).await.unwrap();
        let batches: Vec<_> = flight_data.try_collect().await.unwrap();

        println!("{batches:?}");
    }

    #[tokio::test]
    async fn it_works() {
        let client = Client::try_new("http://localhost:50051").await.unwrap();

        let schema = Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("value", DataType::Utf8, false),
        ]);

        let _result = client
            .create_delta_table()
            .with_location("s3://open-lakehouse/test_table/")
            .with_table_name("test_table")
            .with_schema(&schema)
            .unwrap()
            .await
            .unwrap();

        let data = vec![
            RecordBatch::try_new(
                Arc::new(schema.clone()),
                vec![
                    Arc::new(Int64Array::from(vec![1, 2, 3])),
                    Arc::new(StringArray::from(vec!["a", "b", "c"])),
                ],
            )
            .unwrap(),
        ];

        client
            .ingest(
                "test_table",
                futures::stream::iter(data.into_iter().map(Ok)),
            )
            .await
            .unwrap();

        // assert!(result.is_ok());
    }
}
