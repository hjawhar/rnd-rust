use vm_data::models::streams::StreamType;
use vm_nats::subjects;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use reqwest::StatusCode;
use serde_json::json;
use crate::models::state::AppState;
use crate::routes::middleware::Claims;
use crate::routes::helpers::{audit_log, nats_rpc};

#[derive(Debug, Deserialize)]
pub struct LockProjectPayload {
    /// Unix timestamp (seconds) for scheduled lock. If omitted, locks immediately.
    pub lock_at: Option<u64>,
}

/// Generic chain-aware project operation handler.
/// Looks up the project to determine its network, then routes to the correct worker.
pub(super) async fn chain_aware_project_op(
    state: &Arc<AppState>,
    user_id: i32,
    project_id: i32,
    make_type: fn(i32) -> StreamType,
    sol_subject: &str,
    evm_subject: &str,
    success_msg: &str,
    fail_msg: &str,
    audit_action: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    let (network, locked) = match state.get_db().get_project_network(user_id, project_id).await.ok().flatten() {
        Some(r) => r,
        None => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found" })));
        }
    };

    if locked {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "Project is locked" })));
    }

    let st = match nats_rpc(
        &state.nats_client, &network,
        sol_subject, evm_subject,
        user_id, make_type(project_id),
        Duration::from_secs(15),
    ).await {
        Ok(st) => st,
        Err((status, body)) => return (status, body),
    };

    let is_success = matches!(
        st,
        StreamType::ResponseStartTask(Some(true))
            | StreamType::ResponseStopTask(Some(true))
            | StreamType::ResponseCollectNative(Some(true))
            | StreamType::ResponseCollectTokens(Some(true))
            | StreamType::ResponseDisperseNative(Some(true))
            | StreamType::ResponseDisperseTokens(Some(true))
    );
    let is_not_found = matches!(
        st,
        StreamType::ResponseStartTask(None)
            | StreamType::ResponseStopTask(None)
            | StreamType::ResponseCollectNative(None)
            | StreamType::ResponseCollectTokens(None)
            | StreamType::ResponseDisperseNative(None)
            | StreamType::ResponseDisperseTokens(None)
    );
    if is_success {
        audit_log(state, user_id, Some(project_id), audit_action, None);
        (StatusCode::OK, Json(json!({ "message": success_msg })))
    } else if is_not_found {
        (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found" })))
    } else {
        (StatusCode::BAD_REQUEST, Json(json!({ "error": fail_msg })))
    }
}

pub async fn start_task_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    chain_aware_project_op(
        &state, claims.id, id,
        StreamType::RequestStartTask,
        subjects::rpc::sol::PROJECT_START_TASK, subjects::rpc::evm::PROJECT_START_TASK,
        "Successfully started task", "Failed to start task",
        "task.start",
    ).await
}

pub async fn stop_task_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    chain_aware_project_op(
        &state, claims.id, id,
        StreamType::RequestStopTask,
        subjects::rpc::sol::PROJECT_STOP_TASK, subjects::rpc::evm::PROJECT_STOP_TASK,
        "Successfully stopped task", "Failed to stop task",
        "task.stop",
    ).await
}

pub async fn lock_project_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    body: Option<Json<LockProjectPayload>>,
) -> impl IntoResponse {
    let lock_at = body.and_then(|b| b.lock_at);

    // If lock_at is set, schedule a future lock instead of locking immediately
    if let Some(lock_at_secs) = lock_at {
        let lock_time = std::time::UNIX_EPOCH + Duration::from_secs(lock_at_secs);
        if lock_time <= std::time::SystemTime::now() {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": "lock_at must be in the future" })));
        }
        match state.get_db().set_project_lock_at(id, lock_time).await {
            Ok(true) => {
                audit_log(&state, claims.id, Some(id), "project.lock.scheduled", Some(json!({ "lock_at": lock_at_secs })));
                return (StatusCode::OK, Json(json!({ "message": "Project lock scheduled" })));
            }
            Ok(false) => return (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found" }))),
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "Failed to schedule lock" }))),
        }
    }

    // Immediate lock
    let was_running = match state.get_db().get_project(claims.id, id).await.ok().flatten() {
        Some(p) => p.status == "running",
        None => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found" })));
        }
    };

    
    match state.get_db().set_project_locked(id, true).await {
        Ok(Some((network, _))) => {
            if was_running {
                let _ = nats_rpc(
                    &state.nats_client, &network,
                    subjects::rpc::sol::PROJECT_STOP_TASK, subjects::rpc::evm::PROJECT_STOP_TASK,
                    claims.id, StreamType::RequestStopTask(id),
                    Duration::from_secs(15),
                ).await;
            }
            audit_log(&state, claims.id, Some(id), "project.lock", None);
            (StatusCode::OK, Json(json!({ "message": "Project locked" })))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found" }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "Failed to lock project" }))),
    }
}

pub async fn unlock_project_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    match state.get_db().set_project_locked(id, false).await {
        Ok(Some(_)) => {
            audit_log(&state, claims.id, Some(id), "project.unlock", None);
            (StatusCode::OK, Json(json!({ "message": "Project unlocked" })))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found" }))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "Failed to unlock project" }))),
    }
}
