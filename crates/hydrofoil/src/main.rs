use std::sync::Arc;
use std::time::Duration;

use arrow_flight::flight_service_server::FlightServiceServer;
use mimalloc::MiMalloc;
use tonic::transport::Server;
use tonic_tracing_opentelemetry::middleware::{filters, server::OtelGrpcLayer};
use unitycatalog_object_store::UnityObjectStoreFactory;

mod agent;
mod catalog;
mod config;
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

    // Config file path: first positional arg, else the HYDROFOIL_CONFIG env var
    // (handled inside Config::load). With neither, run on defaults + HYDROFOIL__*
    // env overrides.
    let config_path = std::env::args().nth(1);
    let cfg = config::Config::load(config_path.as_ref())?;

    let addr = format!("{}:{}", cfg.host, cfg.port).parse()?;
    let mut service = server::FlightSqlServiceImpl::try_new()?;

    // Wire OpenLineage when a URL is configured. Without `lineage.url` the
    // session emits to a no-op client and lineage events are dropped.
    let lineage = build_lineage_client(&cfg.lineage)?;
    if cfg.lineage.url.as_deref().is_some_and(|u| !u.is_empty()) {
        // The default job namespace is read from `OPENLINEAGE_NAMESPACE` deep in
        // the request path (OpenLineageConfig::default), so bridge the config
        // value into the environment before any request is served.
        if let Some(ns) = cfg.lineage.namespace.as_deref().filter(|n| !n.is_empty()) {
            // SAFETY: single-threaded startup, before the server accepts requests.
            unsafe { std::env::set_var("OPENLINEAGE_NAMESPACE", ns) };
        }
        tracing::info!("OpenLineage integration enabled");
    } else {
        tracing::info!("OpenLineage integration disabled (set lineage.url to enable)");
    }
    service = service.with_lineage(lineage);

    // Wire Cedar policy enforcement when a policy reference is configured.
    // `policy.oci_ref` is an OCI reference to a Cedar policy image (e.g.
    // `localhost:10100/hydrofoil/plan-policy:latest`). Without it, the server
    // runs with the allow-all default (an open, ungoverned server).
    match cfg.policy.oci_ref.as_deref() {
        Some(reference) if !reference.is_empty() => {
            let policy = policy::CedarPolicy::from_oci(reference).await?;
            tracing::info!("Cedar policy enforcement enabled (ref: {reference})");
            service = service.with_policy(Arc::new(policy));
        }
        _ => {
            tracing::info!("Cedar policy enforcement disabled (set policy.oci_ref to enable)")
        }
    }

    // Wire Unity Catalog when an endpoint is configured. `unity.endpoint` is the
    // Unity Catalog REST base URL (e.g.
    // `http://localhost:8080/api/2.1/unity-catalog/`); `unity.token` is an
    // optional bearer token (omit for a local unauthenticated OSS server).
    match cfg.unity.endpoint.as_deref() {
        Some(uri) if !uri.is_empty() => {
            let mut builder = UnityObjectStoreFactory::builder().with_uri(uri.to_string());
            match cfg.unity.token.as_deref().filter(|t| !t.is_empty()) {
                Some(token) => builder = builder.with_token(token.to_string()),
                None => builder = builder.with_allow_unauthenticated(true),
            }
            if let Some(region) = cfg.unity.region.as_deref().filter(|r| !r.is_empty()) {
                builder = builder.with_aws_region(region.to_string());
            }
            let factory = builder.build().await?;
            tracing::info!("Unity Catalog integration enabled");
            service = service.with_unity(Arc::new(factory));
        }
        _ => {
            tracing::info!("Unity Catalog integration disabled (set unity.endpoint to enable)");
        }
    }

    // Finalize the configured components into the engine + session store and
    // start the background session sweeper.
    let service = service.build(Duration::from_secs(cfg.session_ttl_secs));

    tracing::info!("Listening on {addr:?}");
    let svc = FlightServiceServer::new(service);

    Server::builder()
        .layer(OtelGrpcLayer::default().filter(filters::reject_healthcheck))
        .add_service(svc)
        .serve(addr)
        .await?;

    Ok(())
}

/// Build an OpenLineage client from config. With a non-empty `lineage.url` an
/// HTTP transport is built (the endpoint defaults to `/api/v1/lineage`, the
/// `api_key` is sent as a bearer token); otherwise a no-op client that drops
/// events.
fn build_lineage_client(
    cfg: &config::LineageConfig,
) -> Result<datafusion_open_lineage::OpenLineageClient, Box<dyn std::error::Error>> {
    use datafusion_open_lineage::{CloudClientTransport, OpenLineageClient};

    let Some(url) = cfg.url.as_deref().filter(|u| !u.is_empty()) else {
        return Ok(OpenLineageClient::noop());
    };

    let endpoint = cfg.endpoint.as_deref().unwrap_or("/api/v1/lineage");
    let full = format!("{}{}", url.trim_end_matches('/'), endpoint);
    let endpoint_url = url::Url::parse(&full)?;

    let transport = match cfg.api_key.as_deref().filter(|k| !k.is_empty()) {
        Some(token) => CloudClientTransport::with_token(endpoint_url, token),
        None => CloudClientTransport::unauthenticated(endpoint_url),
    };
    Ok(OpenLineageClient::new(Arc::new(transport)))
}
