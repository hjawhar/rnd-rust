use std::sync::Arc;

use vm_data::models::audit_log::NewAuditLog;

use crate::models::state::AppState;

pub fn audit_log(
    state: &Arc<AppState>,
    user_id: i32,
    project_id: Option<i32>,
    action: &str,
    details: Option<serde_json::Value>,
) {
    let db = state.get_db().clone();
    let log = NewAuditLog {
        user_id,
        project_id,
        action: action.to_string(),
        details,
    };
    tokio::spawn(async move {
        if let Err(e) = db.insert_audit_log(&log).await {
            tracing::warn!("Failed to insert audit log: {}", e);
        }
    });
}


use axum::Json;
use vm_data::models::streams::{StreamInfo, StreamType};
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::time::Duration;

use super::is_evm_network;

/// Send a typed NATS RPC request to the correct chain worker.
///
/// Handles: subject selection by network, serialization, request_with_timeout,
/// deserialization. Returns the deserialized `StreamType` on success, or an
/// HTTP error tuple on failure.
pub async fn nats_rpc(
    nats_client: &async_nats::Client,
    network: &str,
    sol_subject: &str,
    evm_subject: &str,
    user_id: i32,
    stream_type: StreamType,
    timeout: Duration,
) -> Result<StreamType, (StatusCode, Json<Value>)> {
    let subject = if is_evm_network(network) {
        evm_subject
    } else {
        sol_subject
    };

    let serialized = serde_json::to_vec(&StreamInfo {
        user_id,
        stream_type,
    })
    .unwrap_or_default();

    let response = vm_nats::request_with_timeout(
        nats_client,
        subject,
        serialized,
        timeout,
    )
    .await;

    match response {
        Ok(msg) => {
            serde_json::from_slice::<StreamType>(&msg.payload)
                .map_err(|_| (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "Failed to deserialize response" })),
                ))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Request failed: {}", e) })),
        )),
    }
}