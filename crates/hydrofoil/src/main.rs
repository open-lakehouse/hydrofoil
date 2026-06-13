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
mod http;
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
    // session emits to a no-op client and lineage events are dropped. The
    // OpenLineageConfig (job namespace, producer, engine identity) is built once
    // here from the hydrofoil TOML config and threaded through the engine —
    // nothing in the request path re-reads it from the environment.
    let lineage = build_lineage_client(&cfg.lineage)?;
    let lineage_config = build_lineage_config(&cfg.lineage);
    if cfg.lineage.url.as_deref().is_some_and(|u| !u.is_empty()) {
        tracing::info!("OpenLineage integration enabled");
    } else {
        tracing::info!("OpenLineage integration disabled (set lineage.url to enable)");
    }
    // Keep a handle so the shutdown path can drain queued terminal events before
    // the process exits (clones share one background drain task).
    let lineage_shutdown = lineage.clone();
    service = service.with_lineage(lineage, lineage_config);

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
            let mut builder = UnityObjectStoreFactory::builder()
                .with_uri(uri.to_string())
                .with_io_runtime(tokio::runtime::Handle::current());
            match cfg.unity.token.as_deref().filter(|t| !t.is_empty()) {
                Some(token) => builder = builder.with_token(token.to_string()),
                None => builder = builder.with_allow_unauthenticated(true),
            }
            if let Some(region) = cfg.unity.region.as_deref().filter(|r| !r.is_empty()) {
                builder = builder.with_aws_region(region.to_string());
            }
            let factory = builder.build().await?;
            tracing::info!("Unity Catalog integration enabled");
            // The shared factory carries the server-wide `unity.token` (the
            // fallback). Pass the endpoint/region too so the engine can build a
            // per-user factory from a request-supplied UC token, forwarding the
            // caller's identity to UC (per-user permissions).
            service = service.with_unity(
                Arc::new(factory),
                Some(uri.to_string()),
                cfg.unity.region.clone().filter(|r| !r.is_empty()),
            );
        }
        _ => {
            tracing::info!("Unity Catalog integration disabled (set unity.endpoint to enable)");
        }
    }

    // Finalize the configured components into the engine + session store and
    // start the background session sweeper. Shared (as an `Arc`) between the
    // Flight SQL gRPC server and the HTTP query surface so both speak to the
    // same engine, sessions, and UC/Cedar/lineage wiring.
    let service = Arc::new(service.build(Duration::from_secs(cfg.session_ttl_secs)));

    // Flight SQL over gRPC (ADBC, arrow-flight clients). Shut down gracefully on
    // Ctrl-C so the lineage drain below runs on a clean exit.
    let svc = FlightServiceServer::from_arc(service.clone());
    let grpc = Server::builder()
        .layer(OtelGrpcLayer::default().filter(filters::reject_healthcheck))
        .add_service(svc)
        .serve_with_shutdown(addr, shutdown_signal());

    // Catalog-native HTTP query surface (`POST /query`), replacing the
    // UC-quickstart query sidecar. Runs on its own port, same host. Optional:
    // with `http_enabled = false` only the Flight SQL gRPC server runs.
    if !cfg.http_enabled {
        tracing::info!("Flight SQL listening on {addr:?}; HTTP query surface disabled");
        grpc.await?;
    } else {
        let http_addr: std::net::SocketAddr = format!("{}:{}", cfg.host, cfg.http_port).parse()?;
        let router = http::router(http::AppState {
            service: service.clone(),
            query_default_limit: cfg.query_default_limit,
            query_max_limit: cfg.query_max_limit,
        });
        let listener = tokio::net::TcpListener::bind(http_addr).await?;

        tracing::info!("Flight SQL listening on {addr:?}; HTTP query surface on {http_addr:?}");

        // Run both servers concurrently; if either exits (error or clean), bring
        // the process down so a supervisor restarts it rather than serving
        // half-up.
        tokio::select! {
            res = grpc => res?,
            res = axum::serve(listener, router)
                .with_graceful_shutdown(shutdown_signal()) => res?,
        }
    }

    // Flush queued OpenLineage events before exit. `shutdown()` awaits the drain
    // task, which only ends once every client clone's sender is dropped — but the
    // detached session sweeper keeps an engine (and thus a client clone) alive for
    // the process lifetime, so the await would otherwise never return. Bound it
    // with a timeout: the drain task delivers queued events continuously, so a
    // short grace window is enough to flush them even though the task itself
    // won't terminate. A no-op client's drain ends immediately and returns early.
    let drain = tokio::time::timeout(Duration::from_secs(5), lineage_shutdown.shutdown()).await;
    if drain.is_err() {
        tracing::debug!("OpenLineage drain timed out; queued events flushed best-effort");
    }

    Ok(())
}

/// Resolve when the process receives a shutdown signal (Ctrl-C / SIGINT). Used
/// to stop the servers gracefully so the OpenLineage drain runs on exit.
async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::warn!(error = %err, "failed to install Ctrl-C handler");
    }
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

/// Build the static [`OpenLineageConfig`] from hydrofoil's config: the default
/// job namespace comes from `lineage.namespace` (falling back to the crate
/// default), everything else (producer, engine identity, adapter version) takes
/// the crate defaults. This replaces the former `OPENLINEAGE_NAMESPACE` env
/// bridge — the value is built once and threaded through the request path.
fn build_lineage_config(
    cfg: &config::LineageConfig,
) -> datafusion_open_lineage::config::OpenLineageConfig {
    let mut ol = datafusion_open_lineage::config::OpenLineageConfig::default();
    if let Some(ns) = cfg.namespace.as_deref().filter(|n| !n.is_empty()) {
        ol.job_namespace = ns.to_string();
    }
    ol
}
