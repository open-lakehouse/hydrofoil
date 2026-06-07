use std::sync::Arc;

use tracing_subscriber::EnvFilter;

use table_service::config::{Config, SinkKind, WriterConfig};
use table_service::http::{self, AppState};
use table_service::writer::buffered::{BufferedWriter, BufferedWriterConfig};
use table_service::writer::delta::DeltaWriter;
use table_service::writer::iceberg::IcebergSink;
use table_service::writer::sink::TableSink;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cfg = Config::from_env();
    let sinks = build_sinks(&cfg).await;

    let writer = BufferedWriter::spawn(sinks, writer_config(&cfg.writer));
    let app = http::router(AppState {
        writer: writer.handle(),
    });

    let addr = format!("0.0.0.0:{}", cfg.port);
    tracing::info!("table-service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();

    // The server has stopped accepting requests and dropped its handler state
    // (and the writer handle inside it), so the channel can now close. Drain
    // any buffered events before exiting.
    tracing::info!("draining buffered writer");
    writer.shutdown().await;
}

fn writer_config(cfg: &WriterConfig) -> BufferedWriterConfig {
    BufferedWriterConfig {
        buffer_size: cfg.buffer_size,
        flush_interval: std::time::Duration::from_millis(cfg.flush_interval_ms),
        channel_capacity: cfg.channel_capacity,
    }
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

async fn build_sinks(cfg: &Config) -> Vec<Arc<dyn TableSink>> {
    let mut sinks: Vec<Arc<dyn TableSink>> = Vec::with_capacity(cfg.sinks.len());
    for kind in &cfg.sinks {
        match kind {
            SinkKind::Delta => {
                tracing::info!("registering delta sink at {}", cfg.delta.table_path);
                sinks.push(Arc::new(DeltaWriter::new(cfg)));
            }
            SinkKind::Iceberg => {
                let ic = cfg
                    .iceberg
                    .as_ref()
                    .expect("iceberg sink requires ICEBERG_* config");
                tracing::info!(
                    "registering iceberg sink: catalog={} warehouse={} table={}.{}",
                    ic.catalog_uri,
                    ic.warehouse,
                    ic.namespace,
                    ic.table,
                );
                let sink = IcebergSink::from_config(ic)
                    .await
                    .expect("failed to initialize iceberg sink");
                sinks.push(Arc::new(sink));
            }
        }
    }
    sinks
}
