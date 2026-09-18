use async_nats::HeaderMap;
use common::nats::inject_trace_context;
use std::{net::SocketAddr, sync::Arc};

use axum::{Json, Router, extract::State, http::StatusCode, routing::{get, post}};
use clap::Parser;
use common::messages::{TaskAccepted, TaskMessage, TaskRequest, subjects};
use common::telemetry::{MetricsHandle, init_telemetry};
use chrono::Utc;
use tracing::info;
use uuid::Uuid;

#[derive(Parser)]
struct Config {
    /// Address to bind the HTTP server
    #[arg(long, env = "GATEWAY_ADDR", default_value = "0.0.0.0:3000")]
    addr: SocketAddr,

    /// NATS server URL
    #[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
    nats_url: String,
}

struct AppState {
    nats: async_nats::Client,
    metrics: MetricsHandle,
}

#[tokio::main]
async fn main() {
    let config = Config::parse();
    let metrics = init_telemetry("gateway");

    let nats = common::nats::connect_nats(&config.nats_url, "gateway")
        .await
        .expect("failed to connect to NATS");

    let state = Arc::new(AppState { nats, metrics });

    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics_handler))
        .route("/tasks", post(submit_task))
        .with_state(state);

    info!(addr = %config.addr, "gateway listening");
    let listener = tokio::net::TcpListener::bind(config.addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> String {
    state.metrics.render()
}

async fn submit_task(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TaskRequest>,
) -> Result<(StatusCode, Json<TaskAccepted>), (StatusCode, String)> {
    let start = std::time::Instant::now();
    let request_id = Uuid::new_v4();

    let result = async {
        let message = TaskMessage {
            request_id,
            task_type: request.task_type.clone(),
            data: request.data,
            submitted_at: Utc::now(),
        };

        let payload = serde_json::to_vec(&message)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let subject = subjects::task_subject(&request.task_type);

        let mut headers = HeaderMap::new();
        inject_trace_context(&mut headers);

        state
            .nats
            .publish_with_headers(subject.clone(), headers, payload.into())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        tracing::info!(
            request_id = %request_id,
            task_type = %request.task_type,
            subject = %subject,
            "task published to NATS"
        );

        Ok((
            StatusCode::ACCEPTED,
            Json(TaskAccepted {
                request_id,
                status: "accepted",
            }),
        ))
    }
    .await;

    // Record metrics on both success and error paths
    let status = match &result {
        Ok((code, _)) => code.as_str(),
        Err((code, _)) => code.as_str(),
    };
    metrics::counter!("http_requests_total", "method" => "POST", "path" => "/tasks", "status" => status.to_string()).increment(1);
    metrics::histogram!("http_request_duration_seconds", "method" => "POST", "path" => "/tasks").record(start.elapsed().as_secs_f64());

    result
}
