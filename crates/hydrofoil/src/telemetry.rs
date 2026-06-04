use std::collections::HashMap;
use std::time::Duration;

use opentelemetry::{global, trace::TracerProvider};
use opentelemetry_otlp::{WithExportConfig as _, WithHttpConfig as _};
use opentelemetry_sdk::{
    Resource,
    metrics::{MeterProviderBuilder, PeriodicReader, SdkMeterProvider},
    trace::{Sampler, SdkTracerProvider},
};
use tracing::{Level, level_filters::LevelFilter};
use tracing_opentelemetry::MetricsLayer;
use tracing_subscriber::{
    Layer as _, fmt::writer::MakeWriterExt as _, layer::SubscriberExt as _,
    util::SubscriberInitExt as _,
};

/// Default OTLP/HTTP traces endpoint when `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`
/// is unset. Points at the MLflow tracking server's OTLP ingestion path, served
/// at root behind its dedicated Envoy listener (see
/// `environments/docker/envoy/envoy.yaml`). MLflow accepts OTLP **over HTTP
/// only** — no gRPC — at `/v1/traces`, and requires the experiment-id header.
const DEFAULT_TRACES_ENDPOINT: &str = "http://localhost:10120/v1/traces";

fn resource() -> Resource {
    Resource::builder().with_service_name("hydrofoil").build()
}

/// Initialize an OTLP/HTTP metrics exporter — but only when an endpoint is
/// explicitly configured via `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT`.
///
/// The default trace sink (MLflow) has **no** metrics endpoint, so metrics are
/// opt-in: point this at a metrics-capable OTLP collector to enable them. When
/// unset, no meter provider is created (and no `MetricsLayer` is added).
fn init_meter_provider() -> Option<SdkMeterProvider> {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT").ok()?;

    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .with_temporality(opentelemetry_sdk::metrics::Temporality::default())
        .build()
        .expect("Failed to create metric exporter");

    let reader = PeriodicReader::builder(exporter)
        .with_interval(Duration::from_secs(30))
        .build();

    let meter_provider = MeterProviderBuilder::default()
        .with_resource(resource())
        .with_reader(reader)
        .build();

    global::set_meter_provider(meter_provider.clone());

    Some(meter_provider)
}

// Initialize tracing-subscriber and return OtelGuard for opentelemetry-related termination processing
pub(crate) fn init_tracing_subscriber() -> OtelGuard {
    let meter_provider = init_meter_provider();

    // OTLP/HTTP span exporter -> MLflow. The endpoint is the FULL path: the 0.31
    // builder uses a programmatic endpoint verbatim (it does NOT append
    // `/v1/traces`), so the caller supplies the complete URL.
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
        .unwrap_or_else(|_| DEFAULT_TRACES_ENDPOINT.to_string());

    // MLflow routes OTLP traces to an experiment via this header (mandatory).
    // Experiment "0" (Default) always exists.
    let mut headers = HashMap::new();
    let experiment_id = std::env::var("MLFLOW_EXPERIMENT_ID").unwrap_or_else(|_| "0".to_string());
    headers.insert("x-mlflow-experiment-id".to_string(), experiment_id);

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .with_headers(headers)
        .with_timeout(Duration::from_secs(10))
        .build()
        .expect("Failed to create span exporter");

    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource())
        .with_sampler(Sampler::AlwaysOn)
        .build();
    global::set_tracer_provider(tracer_provider.clone());

    let tracer = tracer_provider.tracer("hydrofoil");

    let telemetry_layer = tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_location(false)
        .with_filter(LevelFilter::INFO);

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_writer(std::io::stdout.with_max_level(Level::INFO));

    let registry = tracing_subscriber::registry()
        .with(telemetry_layer)
        .with(fmt_layer);

    // The metrics layer is only added when a meter provider was configured.
    match &meter_provider {
        Some(mp) => registry.with(MetricsLayer::new(mp.clone())).init(),
        None => registry.init(),
    }

    OtelGuard {
        tracer_provider,
        meter_provider,
    }
}

pub(crate) struct OtelGuard {
    tracer_provider: SdkTracerProvider,
    meter_provider: Option<SdkMeterProvider>,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Err(err) = self.tracer_provider.shutdown() {
            eprintln!("{err:?}");
        }
        if let Some(meter_provider) = &self.meter_provider
            && let Err(err) = meter_provider.shutdown()
        {
            eprintln!("{err:?}");
        }
    }
}
