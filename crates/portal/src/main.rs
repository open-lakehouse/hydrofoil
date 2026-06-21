//! Portal server binary: serves the Tags + Files ConnectRPC services over axum.

use std::sync::Arc;

use anyhow::Context;
use tracing_subscriber::EnvFilter;
use unitycatalog_object_store::UnityObjectStoreFactory;

use portal::config::{Config, FilesBackend};
use portal::service::AppState;
use portal::store::{FileStore, MemoryStore, TagStore, UnityVolumeStore};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Config file path: first positional arg, else the PORTAL_CONFIG env var
    // (handled inside Config::load). With neither, run on defaults + PORTAL__*
    // env overrides.
    let config_path = std::env::args().nth(1);
    let cfg = Config::load(config_path.as_ref()).context("invalid configuration")?;

    // Tags are always served from the in-memory store for now.
    let tags: Arc<dyn TagStore> = Arc::new(MemoryStore::new());

    // The files backend is Unity Catalog volumes when configured (or when
    // UNITY_ENDPOINT is set); otherwise fall back to the in-memory store so the
    // service still runs end-to-end with no external dependencies.
    let files: Arc<dyn FileStore> = match cfg.files_backend()? {
        FilesBackend::Unity {
            endpoint,
            token,
            region,
        } => {
            tracing::info!("files backed by Unity Catalog volumes");
            unity_files_backend(endpoint, token, region).await?
        }
        FilesBackend::Memory => {
            tracing::info!(
                "files backed by in-memory store (set files.backend = \"unity\" / UNITY_ENDPOINT to use volumes)"
            );
            Arc::new(MemoryStore::new())
        }
    };

    let state = AppState::new(files, tags);
    let connect = state.register_all(connectrpc::Router::new());

    let app = axum::Router::new()
        .route("/health", axum::routing::get(|| async { "OK" }))
        .fallback_service(connect.into_axum_service());

    let addr = format!("0.0.0.0:{}", cfg.port);
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

/// Build a Unity Catalog volume-backed [`FileStore`] from the resolved config.
///
/// The endpoint must be the Unity Catalog REST base URL (e.g.
/// `https://<host>/api/2.1/unity-catalog/`); use `https://` when a token is set
/// — an `http://` endpoint 301-redirects and drops the bearer token. `token` is
/// optional (omit for a local unauthenticated OSS server); `region` overrides
/// the AWS region for vended credentials.
async fn unity_files_backend(
    endpoint: String,
    token: Option<String>,
    region: Option<String>,
) -> anyhow::Result<Arc<dyn FileStore>> {
    let mut builder = UnityObjectStoreFactory::builder()
        .with_uri(endpoint)
        .with_io_runtime(tokio::runtime::Handle::current());
    match token.filter(|t| !t.is_empty()) {
        Some(token) => builder = builder.with_token(token),
        None => builder = builder.with_allow_unauthenticated(true),
    }
    if let Some(region) = region.filter(|r| !r.is_empty()) {
        builder = builder.with_aws_region(region);
    }

    let factory = builder
        .build()
        .await
        .context("failed to build Unity Catalog object-store factory")?;
    Ok(Arc::new(UnityVolumeStore::new(Arc::new(factory))))
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
