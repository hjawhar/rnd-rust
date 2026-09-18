use common::nats::extract_trace_context;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use std::{net::SocketAddr, sync::Arc, time::Duration};

use clap::Parser;
use axum::{Router, routing::get};
use common::{
    messages::{EnrichmentRequest, EnrichmentResponse, GeoPoint, subjects},
    telemetry::init_telemetry,
};
use chrono::Utc;
use tracing::{error, info, warn};
use futures::StreamExt;

#[derive(Parser)]
struct Config {
    /// NATS server URL
    #[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
    nats_url: String,

    /// Address for metrics HTTP server
    #[arg(long, env = "METRICS_ADDR", default_value = "0.0.0.0:9090")]
    metrics_addr: SocketAddr,

    /// Simulated enrichment latency in milliseconds (0 = no delay)
    #[arg(long, env = "ENRICH_DELAY_MS", default_value = "10")]
    enrich_delay_ms: u64,
}

#[tokio::main]
async fn main() {
    let config = Config::parse();
    let metrics = Arc::new(init_telemetry("enricher"));

    // Spawn lightweight metrics HTTP server
    {
        let metrics = metrics.clone();
        let addr = config.metrics_addr;
        tokio::spawn(async move {
            let app = Router::new()
                .route("/metrics", get(move || {
                    let m = metrics.clone();
                    async move { m.render() }
                }));
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            info!(addr = %addr, "metrics server listening");
            axum::serve(listener, app).await.unwrap();
        });
    }

    let nats = common::nats::connect_nats(&config.nats_url, "enricher")
        .await
        .expect("failed to connect to NATS");

    info!("enricher starting, subscribing to {}", subjects::ENRICH_REQUEST);

    let mut subscriber = nats
        .subscribe(subjects::ENRICH_REQUEST)
        .await
        .expect("failed to subscribe to enrich subject");

    while let Some(msg) = subscriber.next().await {
        let reply = match msg.reply.as_ref() {
            Some(r) => r.clone(),
            None => {
                warn!("received enrich request without reply subject, skipping");
                continue;
            }
        };

        let nats = nats.clone();
        let delay = Duration::from_millis(config.enrich_delay_ms);

        tokio::spawn(async move {
            if let Err(e) = handle_enrich_request(&msg.payload, msg.headers.as_ref(), &nats, &reply, delay).await {
                error!(error = %e, "failed to handle enrich request");
            }
        });
    }

    warn!("NATS subscription ended, shutting down");
}

async fn handle_enrich_request(
    payload: &[u8],
    headers: Option<&async_nats::HeaderMap>,
    nats: &async_nats::Client,
    reply: &async_nats::Subject,
    delay: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let request: EnrichmentRequest = serde_json::from_slice(payload)?;

    if let Some(headers) = headers {
        let parent_context = extract_trace_context(headers);
        let _ = tracing::Span::current().set_parent(parent_context);
    }

    info!(request_id = %request.request_id, "enriching request");

    // Simulate external API latency
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }

    // Generate simulated enrichment data.
    // Uses request_id bytes for deterministic-ish but varied output.
    let id_bytes = request.request_id.as_bytes();
    let lat = (id_bytes[0] as f64 / 255.0) * 180.0 - 90.0;
    let lon = (id_bytes[1] as f64 / 255.0) * 360.0 - 180.0;

    let tags = vec![
        format!("type:{}", request.task_type),
        "source:enricher".to_string(),
        format!("priority:{}", if id_bytes[2] > 128 { "high" } else { "low" }),
    ];

    let response = EnrichmentResponse {
        request_id: request.request_id,
        tags,
        geo: GeoPoint { lat, lon },
        enriched_at: Utc::now(),
    };

    let response_payload = serde_json::to_vec(&response)?;
    nats.publish(reply.clone(), response_payload.into()).await?;

    metrics::counter!("nats_messages_received_total", "subject" => "enrich.request").increment(1);

    info!(request_id = %request.request_id, "enrichment response sent");

    Ok(())
}
