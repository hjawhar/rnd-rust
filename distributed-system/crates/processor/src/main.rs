use async_nats::HeaderMap;
use common::nats::{inject_trace_context, extract_trace_context};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use std::{net::SocketAddr, sync::Arc, time::Duration};

use async_nats::Message;
use clap::Parser;
use axum::{Router, routing::get};
use common::{
    messages::{
        EnrichmentRequest, EnrichmentResponse, TaskMessage, TaskResult, subjects,
    },
    telemetry::init_telemetry,
};
use chrono::Utc;
use tracing::{error, info, warn};
use uuid::Uuid;
use futures::StreamExt;

#[derive(Parser)]
struct Config {
    /// NATS server URL
    #[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
    nats_url: String,

    /// Address for metrics HTTP server
    #[arg(long, env = "METRICS_ADDR", default_value = "0.0.0.0:9091")]
    metrics_addr: SocketAddr,

    /// Unique processor instance ID (defaults to random UUID)
    #[arg(long, env = "PROCESSOR_ID")]
    processor_id: Option<String>,

    /// Simulated processing delay in milliseconds
    #[arg(long, env = "PROCESSING_DELAY_MS", default_value = "50")]
    processing_delay_ms: u64,

    /// Timeout for enrichment request-reply in milliseconds
    #[arg(long, env = "ENRICH_TIMEOUT_MS", default_value = "2000")]
    enrich_timeout_ms: u64,
}

#[tokio::main]
async fn main() {
    let config = Config::parse();
    let metrics = Arc::new(init_telemetry("processor"));

    let processor_id = config
        .processor_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let nats = common::nats::connect_nats(&config.nats_url, &format!("processor-{processor_id}"))
        .await
        .expect("failed to connect to NATS");

    info!(processor_id = %processor_id, "processor starting");

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

    // Subscribe to tasks.* with queue group for competing consumers
    let mut subscriber = nats
        .queue_subscribe(subjects::TASKS_ALL, subjects::PROCESSOR_QUEUE_GROUP.to_string())
        .await
        .expect("failed to subscribe to task subjects");

    info!("subscribed to {} (queue group: {})", subjects::TASKS_ALL, subjects::PROCESSOR_QUEUE_GROUP);

    while let Some(msg) = subscriber.next().await {
        let nats = nats.clone();
        let processor_id = processor_id.clone();
        let processing_delay = Duration::from_millis(config.processing_delay_ms);
        let enrich_timeout = Duration::from_millis(config.enrich_timeout_ms);

        tokio::spawn(async move {
            if let Err(e) = process_message(msg, &nats, &processor_id, processing_delay, enrich_timeout).await {
                error!(error = %e, "failed to process message");
            }
        });
    }

    warn!("NATS subscription ended, shutting down");
}

async fn process_message(
    msg: Message,
    nats: &async_nats::Client,
    processor_id: &str,
    processing_delay: Duration,
    enrich_timeout: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let start = std::time::Instant::now();

    let task: TaskMessage = serde_json::from_slice(&msg.payload)?;

    // Extract trace context from incoming NATS headers
    if let Some(ref headers) = msg.headers {
        let parent_context = extract_trace_context(headers);
        let _ = tracing::Span::current().set_parent(parent_context);
    }

    info!(
        request_id = %task.request_id,
        task_type = %task.task_type,
        "processing task"
    );

    // Simulate CPU-bound work
    tokio::time::sleep(processing_delay).await;

    // Call enricher via NATS request-reply
    let enrich_req = EnrichmentRequest {
        request_id: task.request_id,
        task_type: task.task_type.clone(),
        data: task.data.clone(),
    };

    let enrich_payload = serde_json::to_vec(&enrich_req)?;

    let mut enrich_headers = HeaderMap::new();
    inject_trace_context(&mut enrich_headers);

    let enrich_response = tokio::time::timeout(
        enrich_timeout,
        nats.request_with_headers(subjects::ENRICH_REQUEST, enrich_headers, enrich_payload.into()),
    )
    .await??;

    let enrichment: EnrichmentResponse = serde_json::from_slice(&enrich_response.payload)?;

    // Build result and publish
    let result = TaskResult {
        request_id: task.request_id,
        task_type: task.task_type.clone(),
        payload: task.data,
        enrichment,
        processor_id: processor_id.to_string(),
        processed_at: Utc::now(),
    };

    let result_payload = serde_json::to_vec(&result)?;
    let result_subject = subjects::result_subject(&task.task_type);

    let mut result_headers = HeaderMap::new();
    inject_trace_context(&mut result_headers);

    nats.publish_with_headers(result_subject.clone(), result_headers, result_payload.into())
        .await?;

    let elapsed = start.elapsed();
    metrics::counter!("nats_messages_received_total", "subject" => "tasks.*").increment(1);
    metrics::histogram!("processing_duration_seconds").record(elapsed.as_secs_f64());
    metrics::counter!("nats_messages_published_total", "subject" => "results.*").increment(1);

    info!(
        request_id = %task.request_id,
        task_type = %task.task_type,
        elapsed_ms = elapsed.as_millis(),
        "task processed and result published"
    );

    Ok(())
}
