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

use super::batch_sizer::BatchSizer;

pub struct IngestBuilder<S>
where
    S: Stream<Item = Result<RecordBatch, ArrowError>> + Send + 'static,
{
    client: FlightSqlServiceClient<Channel>,
    stream: S,
    message: CommandStatementIngest,
    max_batch_size: Option<usize>,
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
            max_batch_size: None,
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

    /// Set the maximum batch size in bytes for RecordBatches sent to the server.
    ///
    /// If not set, the default limit of 3.5MB will be used. This ensures that batches
    /// stay under the 4MB gRPC message limit with encoding overhead.
    ///
    /// Large batches will be automatically split, and small batches will be buffered
    /// and combined to optimize throughput.
    pub fn with_max_batch_size(mut self, max_batch_size: usize) -> Self {
        self.max_batch_size = Some(max_batch_size);
        self
    }
}

impl<S> IntoFuture for IngestBuilder<S>
where
    S: Stream<Item = Result<RecordBatch, ArrowError>> + Send + 'static,
{
    type Output = Result<i64, FlightError>;
    type IntoFuture = BoxFuture<'static, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        let mut client = self.client;
        let message = self.message;
        let stream = self.stream;
        let max_batch_size = self.max_batch_size;

        Box::pin(async move {
            // Wrap the stream with BatchSizer to ensure batches stay within size limits
            let sized_stream = if let Some(size) = max_batch_size {
                BatchSizer::with_max_size(stream, size)
            } else {
                BatchSizer::new(stream)
            };

            client
                .execute_ingest(message, sized_stream.map_err(FlightError::from))
                .await
        })
    }
}
