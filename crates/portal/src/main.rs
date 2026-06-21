//! Portal server binary: serves the Tags + Files ConnectRPC services over axum.

use std::sync::Arc;

use anyhow::Context;
use tracing_subscriber::EnvFilter;
use unitycatalog_object_store::UnityObjectStoreFactory;

use portal::service::AppState;
use portal::store::{FileStore, MemoryStore, TagStore, UnityVolumeStore};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let port: u16 = std::env::var("PORTAL_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    // Tags are always served from the in-memory store for now.
    let tags: Arc<dyn TagStore> = Arc::new(MemoryStore::new());

    // The files backend is Unity Catalog volumes when `UNITY_ENDPOINT` is set;
    // otherwise fall back to the in-memory store so the service still runs
    // end-to-end with no external dependencies.
    let files: Arc<dyn FileStore> = match unity_files_backend().await? {
        Some(store) => {
            tracing::info!("files backed by Unity Catalog volumes");
            store
        }
        None => {
            tracing::info!("files backed by in-memory store (set UNITY_ENDPOINT to use volumes)");
            Arc::new(MemoryStore::new())
        }
    };

    let state = AppState::new(files, tags);
    let connect = state.register_all(connectrpc::Router::new());

    let app = axum::Router::new()
        .route("/health", axum::routing::get(|| async { "OK" }))
        .fallback_service(connect.into_axum_service());

    let addr = format!("0.0.0.0:{port}");
    tracing::info!("portal listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
    Ok(())
}

/// Build a Unity Catalog volume-backed [`FileStore`] from the environment.
///
/// Returns `Ok(None)` when `UNITY_ENDPOINT` is unset/empty (the caller falls
/// back to the in-memory store). The endpoint must be the Unity Catalog REST
/// base URL (e.g. `https://<host>/api/2.1/unity-catalog/`); use `https://` — an
/// `http://` endpoint 301-redirects and drops the bearer token. `UNITY_TOKEN`
/// is an optional bearer token (omit for a local unauthenticated OSS server);
/// `UNITY_REGION` (falling back to `AWS_REGION`) overrides the AWS region for
/// vended credentials.
async fn unity_files_backend() -> anyhow::Result<Option<Arc<dyn FileStore>>> {
    let endpoint = match std::env::var("UNITY_ENDPOINT")
        .ok()
        .filter(|e| !e.is_empty())
    {
        Some(e) => e,
        None => return Ok(None),
    };

    let mut builder = UnityObjectStoreFactory::builder()
        .with_uri(endpoint)
        .with_io_runtime(tokio::runtime::Handle::current());
    match std::env::var("UNITY_TOKEN").ok().filter(|t| !t.is_empty()) {
        Some(token) => builder = builder.with_token(token),
        None => builder = builder.with_allow_unauthenticated(true),
    }
    if let Some(region) = std::env::var("UNITY_REGION").ok().filter(|r| !r.is_empty()) {
        builder = builder.with_aws_region(region);
    }

    let factory = builder
        .build()
        .await
        .context("failed to build Unity Catalog object-store factory")?;
    Ok(Some(Arc::new(UnityVolumeStore::new(Arc::new(factory)))))
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
