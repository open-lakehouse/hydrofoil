//! Portal server binary: serves the Tags + Files ConnectRPC services over axum.

use std::sync::Arc;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

use portal::service::AppState;
use portal::store::MemoryStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let port: u16 = std::env::var("PORTAL_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let state = AppState::new(Arc::new(MemoryStore::new()));
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
