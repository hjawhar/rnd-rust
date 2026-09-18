use metrics_exporter_prometheus::PrometheusBuilder;
use opentelemetry::global;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub struct MetricsHandle {
    handle: metrics_exporter_prometheus::PrometheusHandle,
}

impl MetricsHandle {
    pub fn render(&self) -> String {
        self.handle.render()
    }
}

/// Initialize tracing with optional OTLP export and Prometheus metrics.
///
/// Set `OTEL_EXPORTER_OTLP_ENDPOINT` to enable trace export (e.g., `http://localhost:4317`).
/// If not set, traces are only logged locally.
pub fn init_telemetry(service_name: &str) -> MetricsHandle {
    let prometheus_handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus recorder");

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .json();

    let otlp_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    if let Some(endpoint) = otlp_endpoint {
        let exporter = SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&endpoint)
            .build()
            .expect("failed to create OTLP exporter");

        let resource = Resource::builder()
            .with_service_name(service_name.to_string())
            .build();

        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(resource)
            .build();

        let tracer = provider.tracer(service_name.to_string());

        // Register the W3C TraceContext propagator globally.
        // Without this, inject/extract_trace_context are no-ops.
        global::set_text_map_propagator(TraceContextPropagator::new());

        let otel_layer = OpenTelemetryLayer::new(tracer);

        registry.with(otel_layer).init();

        tracing::info!(service = service_name, endpoint = endpoint, "OTLP tracing enabled");
    } else {
        registry.init();
        tracing::info!(service = service_name, "telemetry initialized (no OTLP export)");
    }

    MetricsHandle {
        handle: prometheus_handle,
    }
}
