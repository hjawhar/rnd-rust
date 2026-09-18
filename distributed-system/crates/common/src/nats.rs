use async_nats::{Client, HeaderMap};
use opentelemetry::propagation::{Extractor, Injector};
use opentelemetry::global;
use tracing::info;
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub async fn connect_nats(url: &str, service_name: &str) -> Result<Client, async_nats::Error> {
    let client = async_nats::ConnectOptions::new()
        .name(service_name)
        .connect(url)
        .await?;

    info!(service = service_name, url = url, "connected to NATS");
    Ok(client)
}

/// Inject the current tracing span's context into NATS headers.
/// Call this before publishing a message to propagate the trace.
pub fn inject_trace_context(headers: &mut HeaderMap) {
    let context = tracing::Span::current().context();
    global::get_text_map_propagator(|p| {
        p.inject_context(&context, &mut NatsHeaderInjector(headers));
    });
}

/// Extract trace context from NATS headers.
/// Call this at the start of a message handler.
pub fn extract_trace_context(headers: &HeaderMap) -> opentelemetry::Context {
    global::get_text_map_propagator(|p| {
        p.extract(&NatsHeaderExtractor(headers))
    })
}

struct NatsHeaderInjector<'a>(&'a mut HeaderMap);

impl Injector for NatsHeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key, value.as_str());
    }
}

struct NatsHeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for NatsHeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|v| v.as_str())
    }

    fn keys(&self) -> Vec<&str> {
        // W3C TraceContext propagator uses `get()` with known field names
        // ("traceparent", "tracestate") — it does not rely on `keys()`.
        // We return the well-known propagation headers to satisfy the trait.
        vec!["traceparent", "tracestate"]
    }
}
