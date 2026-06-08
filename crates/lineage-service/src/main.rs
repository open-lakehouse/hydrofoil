use std::sync::Arc;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

use lineage_service::config::{Config, SinkKind, WriterConfig};
use lineage_service::http::{self, AppState};
use lineage_service::writer::buffered::{BufferedWriter, BufferedWriterConfig};
use lineage_service::writer::delta::DeltaWriter;
use lineage_service::writer::sink::TableSink;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // Config file path: first positional arg, else the LINEAGE_CONFIG env var
    // (handled inside Config::load). With neither, run on defaults + LINEAGE__*
    // env overrides.
    let config_path = std::env::args().nth(1);
    let cfg = Config::load(config_path.as_ref()).context("invalid configuration")?;
    let sinks = build_sinks(&cfg).await?;

    let writer = BufferedWriter::spawn(sinks, writer_config(&cfg.writer));
    let app = http::router(AppState {
        writer: writer.handle(),
    });

    let addr = format!("0.0.0.0:{}", cfg.port);
    tracing::info!("lineage-service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    // The server has stopped accepting requests and dropped its handler state
    // (and the writer handle inside it), so the channel can now close. Drain
    // any buffered events before exiting.
    tracing::info!("draining buffered writer");
    writer.shutdown().await;
    Ok(())
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

async fn build_sinks(cfg: &Config) -> anyhow::Result<Vec<Arc<dyn TableSink>>> {
    let mut sinks: Vec<Arc<dyn TableSink>> = Vec::with_capacity(cfg.sinks.len());
    for kind in &cfg.sinks {
        match kind {
            SinkKind::Delta => {
                tracing::info!("registering delta sink at {}", cfg.delta.table_path);
                sinks.push(Arc::new(DeltaWriter::new(cfg)));
            }
            #[cfg(feature = "iceberg")]
            SinkKind::Iceberg => {
                use lineage_service::writer::iceberg::IcebergSink;
                let ic = cfg
                    .iceberg
                    .as_ref()
                    .context("iceberg sink requires ICEBERG_* config")?;
                tracing::info!(
                    "registering iceberg sink: catalog={} warehouse={} table={}.{}",
                    ic.catalog_uri,
                    ic.warehouse,
                    ic.namespace,
                    ic.table,
                );
                let sink = IcebergSink::from_config(ic)
                    .await
                    .context("failed to initialize iceberg sink")?;
                sinks.push(Arc::new(sink));
            }
            // `SinkKind::Iceberg` can only be produced when the `iceberg`
            // feature is enabled (config parsing rejects it otherwise), so this
            // arm is unreachable in a default build — it exists only to keep the
            // match exhaustive over the always-present enum variant.
            #[cfg(not(feature = "iceberg"))]
            SinkKind::Iceberg => unreachable!(
                "iceberg sink selected without the `iceberg` feature; config parsing should have rejected this"
            ),
        }
    }
    Ok(sinks)
}
