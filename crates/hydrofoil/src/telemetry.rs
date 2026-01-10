use std::time::Duration;

use opentelemetry::{global, trace::TracerProvider};
use opentelemetry_otlp::WithExportConfig as _;
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

fn resource() -> Resource {
    Resource::builder().with_service_name("hydrofoil").build()
}

pub(crate) fn init_meter_provider() -> SdkMeterProvider {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_temporality(opentelemetry_sdk::metrics::Temporality::default())
        .build()
        .unwrap();

    let reader = PeriodicReader::builder(exporter)
        .with_interval(std::time::Duration::from_secs(30))
        .build();

    // For debugging in development
    // let stdout_reader =
    //     PeriodicReader::builder(opentelemetry_stdout::MetricExporter::default()).build();

    let meter_provider = MeterProviderBuilder::default()
        .with_resource(resource())
        .with_reader(reader)
        // with_reader(stdout_reader)
        .build();

    global::set_meter_provider(meter_provider.clone());

    meter_provider
}

// Initialize tracing-subscriber and return OtelGuard for opentelemetry-related termination processing
pub(crate) fn init_tracing_subscriber() -> OtelGuard {
    let meter_provider = init_meter_provider();
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint("http://localhost:4317") // Endpoint for OTLP collector.
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

    tracing_subscriber::registry()
        .with(telemetry_layer)
        .with(fmt_layer)
        .with(MetricsLayer::new(meter_provider.clone()))
        .init();

    OtelGuard {
        tracer_provider,
        meter_provider,
    }
}

pub(crate) struct OtelGuard {
    tracer_provider: SdkTracerProvider,
    meter_provider: SdkMeterProvider,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Err(err) = self.tracer_provider.shutdown() {
            eprintln!("{err:?}");
        }
        if let Err(err) = self.meter_provider.shutdown() {
            eprintln!("{err:?}");
        }
    }
}
