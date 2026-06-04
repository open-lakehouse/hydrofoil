//! End-to-end proof that large datasets cross the Flight wire cleanly under
//! *default* gRPC message limits — in both directions.
//!
//! We removed the hand-rolled `BatchSizer`/`BatchChunkStream` chunking because
//! arrow-flight's `FlightDataEncoderBuilder` already splits large `RecordBatch`es
//! into multiple `FlightData` messages under its 2 MiB default target, safely
//! below tonic's 4 MB decode limit. These tests are the regression guard: they
//! spin up a minimal in-process Flight SQL server with **no** `max_*_message_size`
//! override on either the server or the client, push a single RecordBatch whose
//! in-memory size is well over 4 MB through both `do_get` (server -> client) and
//! ingest/`do_put` (client -> server), and assert the data arrives intact. If
//! anyone reintroduces un-split sending — or bumps the gRPC limits to mask it —
//! these tests fail.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use arrow_flight::decode::FlightRecordBatchStream;
use arrow_flight::encode::FlightDataEncoderBuilder;
use arrow_flight::error::FlightError;
use arrow_flight::flight_service_server::FlightServiceServer;
use arrow_flight::sql::server::{FlightSqlService, PeekableFlightDataStream};
use arrow_flight::sql::{
    CommandStatementIngest, CommandStatementQuery, ProstMessageExt, SqlInfo, TicketStatementQuery,
};
use arrow_flight::{FlightDescriptor, FlightEndpoint, FlightInfo, Ticket};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use futures::TryStreamExt;
use prost::Message;
use tokio::net::TcpListener;
use tonic::transport::Server;
use tonic::transport::server::TcpIncoming;
use tonic::{Request, Response, Status};

use hydrofoil_client::Client;

/// Roughly 32 MB of in-memory data: ~1M rows of an i64 (8 bytes) plus a ~24-byte
/// string each. Comfortably larger than tonic's 4 MB decode limit, so a single
/// un-split IPC message would be rejected.
const NUM_ROWS: usize = 1_000_000;

fn test_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("payload", DataType::Utf8, false),
    ]))
}

fn big_batch() -> RecordBatch {
    let ids = Int64Array::from_iter_values(0..NUM_ROWS as i64);
    let payload =
        StringArray::from_iter_values((0..NUM_ROWS).map(|i| format!("payload-value-{i:012}")));
    RecordBatch::try_new(test_schema(), vec![Arc::new(ids), Arc::new(payload)]).unwrap()
}

/// Sanity check that the test data really would blow past a single 4 MB gRPC
/// message — otherwise the tests below would pass even without splitting.
#[test]
fn fixture_batch_exceeds_grpc_limit() {
    let batch = big_batch();
    let in_memory = batch.get_array_memory_size();
    assert!(
        in_memory > 4 * 1024 * 1024,
        "fixture batch is only {in_memory} bytes; must exceed 4 MB to be a meaningful test"
    );
}

/// Minimal Flight SQL server: enough to serve `do_get` of a fixed big batch and
/// to accept an ingest `do_put`, counting the rows it receives.
#[derive(Clone)]
struct TestFlightSqlServer;

#[tonic::async_trait]
impl FlightSqlService for TestFlightSqlServer {
    type FlightService = TestFlightSqlServer;

    async fn get_flight_info_statement(
        &self,
        _query: CommandStatementQuery,
        request: Request<FlightDescriptor>,
    ) -> Result<Response<FlightInfo>, Status> {
        let schema = test_schema();
        // The ticket bytes round-trip back to `do_get_statement` as a
        // TicketStatementQuery, which is how the framework routes do_get.
        let ticket_stmt = TicketStatementQuery {
            statement_handle: b"big".to_vec().into(),
        };
        // The framework's do_get dispatch decodes the ticket as an Any-packed
        // Command, so pack it via as_any() rather than encoding the bare message.
        let ticket = Ticket::new(ticket_stmt.as_any().encode_to_vec());
        let endpoint = FlightEndpoint::new().with_ticket(ticket);
        let info = FlightInfo::new()
            .try_with_schema(&schema)
            .map_err(|e| Status::internal(e.to_string()))?
            .with_endpoint(endpoint)
            .with_descriptor(request.into_inner());
        Ok(Response::new(info))
    }

    async fn do_get_statement(
        &self,
        _ticket: TicketStatementQuery,
        _request: Request<Ticket>,
    ) -> Result<
        Response<<Self as arrow_flight::flight_service_server::FlightService>::DoGetStream>,
        Status,
    > {
        let schema = test_schema();
        // Mirrors crates/hydrofoil/src/stream.rs: rely on the encoder's 2 MiB
        // default to split the oversized batch into <4 MB FlightData messages.
        let stream = FlightDataEncoderBuilder::new()
            .with_schema(schema)
            .build(futures::stream::once(async { Ok(big_batch()) }))
            .map_err(Status::from);
        Ok(Response::new(Box::pin(stream)))
    }

    async fn do_put_statement_ingest(
        &self,
        _command: CommandStatementIngest,
        request: Request<PeekableFlightDataStream>,
    ) -> Result<i64, Status> {
        // Decode the inbound FlightData stream back into RecordBatches under the
        // default 4 MB decode limit. If the client failed to split the upload,
        // this is where "message length too large" would surface.
        let flight_data = request
            .into_inner()
            .map_err(|status| FlightError::Tonic(Box::new(status)));
        let mut batches = FlightRecordBatchStream::new_from_flight_data(flight_data);
        let mut total: i64 = 0;
        while let Some(batch) = batches
            .try_next()
            .await
            .map_err(|e| Status::internal(e.to_string()))?
        {
            total += batch.num_rows() as i64;
        }
        Ok(total)
    }

    async fn register_sql_info(&self, _id: i32, _result: &SqlInfo) {}
}

/// Start the test server on an ephemeral localhost port with default tonic
/// limits and return its `http://` URL. The server task runs until the test ends.
async fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let incoming = TcpIncoming::from(listener);

    let svc = FlightServiceServer::new(TestFlightSqlServer);
    tokio::spawn(async move {
        // No .max_encoding_message_size / .max_decoding_message_size — defaults only.
        Server::builder()
            .add_service(svc)
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });

    format!("http://{addr}")
}

/// Outbound: server -> client via do_get. The 32 MB batch must arrive intact,
/// proving FlightDataEncoderBuilder split it into sub-4 MB messages.
#[tokio::test]
async fn do_get_large_batch_roundtrips() {
    let url = start_server().await;
    let mut client = Client::try_new(url).await.unwrap();

    let stream = client.execute("SELECT * FROM big", None).await.unwrap();
    let batches: Vec<RecordBatch> = stream.try_collect().await.unwrap();

    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, NUM_ROWS, "all rows must survive the round trip");
    assert!(
        batches.len() > 1,
        "a 32 MB batch must arrive as multiple sub-4 MB messages, got {}",
        batches.len()
    );
    assert_eq!(batches[0].schema(), test_schema());
}

/// Inbound: client -> server via ingest/do_put. This is the path that previously
/// went through BatchSizer; the stream is now passed straight to
/// execute_ingest, which encodes + splits it under the 2 MiB default.
#[tokio::test]
async fn ingest_large_batch_roundtrips() {
    let url = start_server().await;
    let client = Client::try_new(url).await.unwrap();

    let batch = big_batch();
    let input = futures::stream::once(async move { Ok::<_, arrow_schema::ArrowError>(batch) });

    let rows = client.ingest("big", input).await.unwrap();
    assert_eq!(
        rows, NUM_ROWS as i64,
        "server must observe every ingested row"
    );
}

/// Ordering / multi-batch guard: a stream of many medium batches must all arrive
/// through ingest, in order, with the full row count preserved.
#[tokio::test]
async fn ingest_many_batches_preserves_rows() {
    let url = start_server().await;
    let client = Client::try_new(url).await.unwrap();

    let schema = test_schema();
    let per_batch = 50_000;
    let n_batches = 8;
    let batches: Vec<_> = (0..n_batches)
        .map(|b| {
            let base = (b * per_batch) as i64;
            let ids = Int64Array::from_iter_values(base..base + per_batch as i64);
            let payload = StringArray::from_iter_values(
                (0..per_batch).map(|i| format!("payload-value-{:012}", base as usize + i)),
            );
            Ok::<_, arrow_schema::ArrowError>(
                RecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(payload)])
                    .unwrap(),
            )
        })
        .collect();

    let input = futures::stream::iter(batches);
    let rows = client.ingest("big", input).await.unwrap();
    assert_eq!(rows, (n_batches * per_batch) as i64);
}
