use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{Extension, Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;

use crate::models::state::AppState;

#[derive(Clone, Serialize)]
pub struct HealthInfo {
    pub status: &'static str,
    pub checks: HealthChecks,
}

#[derive(Clone, Serialize)]
pub struct HealthChecks {
    pub db: bool,
    pub nats: bool,
    pub redis: bool,
}

#[derive(Clone, Serialize)]
pub struct WorkerGroup {
    pub active: usize,
    pub instances: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct Workers {
    pub sol: WorkerGroup,
    pub evm: WorkerGroup,
    pub relay: WorkerGroup,
}

#[derive(Clone, Serialize)]
pub struct SystemStatusData {
    pub health: HealthInfo,
    pub workers: Workers,
    pub ws_clients: usize,
}

fn compute_status(checks: &HealthChecks) -> &'static str {
    let passed = checks.db as u8 + checks.nats as u8 + checks.redis as u8;
    match passed {
        3 => "healthy",
        0 => "unhealthy",
        _ => "degraded",
    }
}

async fn run_health_checks(state: &AppState) -> HealthChecks {
    let db = state.db.check_connection().await.is_ok();

    let nats = tokio::time::timeout(
        Duration::from_secs(3),
        state.nats_client.flush(),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false);

    let redis = vm_redis::ping_redis().await.unwrap_or(false);

    HealthChecks { db, nats, redis }
}

/// Count active workers from a Redis hash, filtering out entries older than 45s.
async fn count_active_workers(hash_key: &str) -> WorkerGroup {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let entries = vm_redis::hgetall_redis(hash_key)
        .await
        .unwrap_or_default();

    let cutoff = now.saturating_sub(45);
    let instances: Vec<String> = entries
        .into_iter()
        .filter(|(_id, ts)| ts.parse::<u64>().unwrap_or(0) > cutoff)
        .map(|(id, _ts)| id)
        .collect();

    WorkerGroup {
        active: instances.len(),
        instances,
    }
}

/// Compute the full system status snapshot (reused by HTTP endpoint + WS broadcast).
pub async fn compute_system_status(state: &AppState) -> SystemStatusData {
    let checks = run_health_checks(state).await;
    let status = compute_status(&checks);

    let (sol, evm, relay) = tokio::join!(
        count_active_workers("workers:sol"),
        count_active_workers("workers:evm"),
        count_active_workers("workers:relay"),
    );

    let ws_clients = state.ws_clients.client_count().await;

    SystemStatusData {
        health: HealthInfo { status, checks },
        workers: Workers { sol, evm, relay },
        ws_clients,
    }
}

#[derive(Serialize)]
pub struct PingResponse {
    pub version: &'static str,
}

/// GET /ping — unauthenticated, returns api version
pub async fn ping() -> impl IntoResponse {
    Json(PingResponse {
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// GET /health — unauthenticated liveness probe
pub async fn health_check(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    let checks = run_health_checks(&state).await;
    let status = compute_status(&checks);

    let code = match status {
        "healthy" => StatusCode::OK,
        "degraded" => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };

    (code, Json(HealthInfo { status, checks }))
}

/// GET /system/status — authenticated, full status for frontend
pub async fn system_status(
    Extension(state): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    Json(compute_system_status(&state).await)
}
