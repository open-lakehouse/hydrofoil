use std::sync::Arc;

use arrow_flight::flight_service_server::FlightServiceServer;
use mimalloc::MiMalloc;
use tonic::transport::Server;
use tonic_tracing_opentelemetry::middleware::{filters, server::OtelGrpcLayer};
use unitycatalog_object_store::UnityObjectStoreFactory;

mod catalog;
mod error;
mod execution;
mod lineage;
mod planner;
mod policy;
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
    let mut service = server::FlightSqlServiceImpl::try_new()?;

    // Wire OpenLineage when an endpoint is configured. `OPENLINEAGE_URL` is the
    // base URL of an OpenLineage-compatible service; `OPENLINEAGE_API_KEY` is an
    // optional bearer token. Without `OPENLINEAGE_URL` lineage events are dropped.
    match datafusion_open_lineage::OpenLineageClient::from_env() {
        Ok(client) => {
            if std::env::var("OPENLINEAGE_URL").is_ok() {
                tracing::info!("OpenLineage integration enabled");
            } else {
                tracing::info!(
                    "OpenLineage integration disabled (set OPENLINEAGE_URL to enable)"
                );
            }
            service = service.with_lineage(client);
        }
        Err(e) => tracing::warn!("OpenLineage disabled: {e}"),
    }

    // Wire Unity Catalog when an endpoint is configured. `UC_ENDPOINT` is the
    // Unity Catalog REST base URL (e.g.
    // `http://localhost:8080/api/2.1/unity-catalog/`); `UC_TOKEN` is an
    // optional bearer token (omit for a local unauthenticated OSS server).
    if let Ok(uri) = std::env::var("UC_ENDPOINT") {
        let mut builder = UnityObjectStoreFactory::builder().with_uri(uri);
        match std::env::var("UC_TOKEN") {
            Ok(token) => builder = builder.with_token(token),
            Err(_) => builder = builder.with_allow_unauthenticated(true),
        }
        if let Ok(region) = std::env::var("AWS_REGION") {
            builder = builder.with_aws_region(region);
        }
        let factory = builder.build().await?;
        tracing::info!("Unity Catalog integration enabled");
        service = service.with_unity(Arc::new(factory));
    } else {
        tracing::info!("Unity Catalog integration disabled (set UC_ENDPOINT to enable)");
    }

    tracing::info!("Listening on {addr:?}");
    let svc = FlightServiceServer::new(service);

    Server::builder()
        .layer(OtelGrpcLayer::default().filter(filters::reject_healthcheck))
        .add_service(svc)
        .serve(addr)
        .await?;

    Ok(())
}
