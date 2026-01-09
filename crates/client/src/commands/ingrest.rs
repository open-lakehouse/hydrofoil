use arrow_array::RecordBatch;
use arrow_flight::{
    error::FlightError,
    sql::{
        CommandStatementIngest, TableDefinitionOptions, TableExistsOption, TableNotExistOption,
        client::FlightSqlServiceClient,
    },
};
use arrow_schema::ArrowError;
use bytes::Bytes;
use futures::{Stream, TryStreamExt, future::BoxFuture};
use tonic::transport::Channel;

pub struct IngestBuilder<S>
where
    S: Stream<Item = Result<RecordBatch, ArrowError>> + Send + 'static,
{
    client: FlightSqlServiceClient<Channel>,
    stream: S,
    message: CommandStatementIngest,
}

impl<S> IngestBuilder<S>
where
    S: Stream<Item = Result<RecordBatch, ArrowError>> + Send + 'static,
{
    pub(crate) fn new(client: FlightSqlServiceClient<Channel>, stream: S) -> Self {
        Self {
            client,
            stream,
            message: CommandStatementIngest::default(),
        }
    }

    pub fn with_table_name(mut self, name: impl ToString) -> Self {
        self.message.table = name.to_string();
        self
    }

    pub fn with_schema_name(mut self, name: impl ToString) -> Self {
        self.message.schema = Some(name.to_string());
        self
    }

    pub fn with_if_exists(mut self, if_exists: TableExistsOption) -> Self {
        if let Some(options) = self.message.table_definition_options.as_mut() {
            options.if_exists = if_exists as i32;
        } else {
            self.message.table_definition_options = Some(TableDefinitionOptions {
                if_exists: if_exists as i32,
                ..Default::default()
            });
        }
        self
    }

    pub fn with_if_not_exist(mut self, if_not_exist: TableNotExistOption) -> Self {
        if let Some(options) = self.message.table_definition_options.as_mut() {
            options.if_not_exist = if_not_exist as i32;
        } else {
            self.message.table_definition_options = Some(TableDefinitionOptions {
                if_not_exist: if_not_exist as i32,
                ..Default::default()
            });
        }
        self
    }

    pub fn with_catalog_name(mut self, name: impl ToString) -> Self {
        self.message.catalog = Some(name.to_string());
        self
    }

    pub fn with_transaction_id(mut self, transaction_id: impl Into<Bytes>) -> Self {
        self.message.transaction_id = Some(transaction_id.into());
        self
    }
}

impl<S> IntoFuture for IngestBuilder<S>
where
    S: Stream<Item = Result<RecordBatch, ArrowError>> + Send + 'static,
{
    type Output = Result<i64, ArrowError>;
    type IntoFuture = BoxFuture<'static, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        let mut client = self.client;
        let message = self.message;
        let stream = self.stream;

        Box::pin(async move {
            client
                .execute_ingest(message, stream.map_err(FlightError::from))
                .await
        })
    }
}
