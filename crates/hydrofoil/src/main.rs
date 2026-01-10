use arrow_flight::flight_service_server::FlightServiceServer;
use mimalloc::MiMalloc;
use tonic::transport::Server;
use tonic_tracing_opentelemetry::middleware::{filters, server::OtelGrpcLayer};

mod catalog;
mod error;
mod execution;
mod external_tables;
mod planner;
mod server;
mod session;
mod stream;
mod telemetry;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // telemetry::init_tracer_provider();
    let _guard = telemetry::init_tracing_subscriber();

    let addr = "0.0.0.0:50051".parse()?;
    let service = server::FlightSqlServiceImpl::try_new()?;
    tracing::info!("Listening on {addr:?}");
    let svc = FlightServiceServer::new(service);

    Server::builder()
        .layer(OtelGrpcLayer::default().filter(filters::reject_healthcheck))
        .add_service(svc)
        .serve(addr)
        .await?;

    Ok(())
}
