use std::sync::Arc;

use arrow_flight::flight_service_server::FlightServiceServer;
use mimalloc::MiMalloc;
use tonic::transport::Server;
use tonic_tracing_opentelemetry::middleware::{filters, server::OtelGrpcLayer};
use unitycatalog_object_store::UnityObjectStoreFactory;

mod agent;
mod catalog;
mod engine;
mod error;
mod execution;
mod identity;
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
                tracing::info!("OpenLineage integration disabled (set OPENLINEAGE_URL to enable)");
            }
            service = service.with_lineage(client);
        }
        Err(e) => tracing::warn!("OpenLineage disabled: {e}"),
    }

    // Wire Cedar policy enforcement when a policy reference is configured.
    // `HYDROFOIL_POLICY_REF` is an OCI reference to a Cedar policy image (e.g.
    // `localhost:10100/hydrofoil/plan-policy:latest`). Without it, the server
    // runs with the allow-all default (an open, ungoverned server).
    match std::env::var("HYDROFOIL_POLICY_REF") {
        Ok(reference) if !reference.is_empty() => {
            let policy = policy::CedarPolicy::from_oci(&reference).await?;
            tracing::info!("Cedar policy enforcement enabled (ref: {reference})");
            service = service.with_policy(Arc::new(policy));
        }
        _ => {
            tracing::info!("Cedar policy enforcement disabled (set HYDROFOIL_POLICY_REF to enable)")
        }
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

    // Finalize the configured components into the engine + session store and
    // start the background session sweeper.
    let service = service.build();

    tracing::info!("Listening on {addr:?}");
    let svc = FlightServiceServer::new(service);

    Server::builder()
        .layer(OtelGrpcLayer::default().filter(filters::reject_healthcheck))
        .add_service(svc)
        .serve(addr)
        .await?;

    Ok(())
}
