use arrow_flight::flight_service_server::FlightServiceServer;
use mimalloc::MiMalloc;
use tonic::transport::Server;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod catalog;
mod error;
mod execution;
mod server;
mod storage;
mod stream;
mod telemetry;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    telemetry::init_tracer_provider();
    let _guard = telemetry::init_tracing_subscriber();

    let addr = "0.0.0.0:50051".parse()?;
    let service = server::FlightSqlServiceImpl::try_new()?;
    tracing::info!("Listening on {addr:?}");
    let svc = FlightServiceServer::new(service);

    Server::builder().add_service(svc).serve(addr).await?;

    Ok(())
}
