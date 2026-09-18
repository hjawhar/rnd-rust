use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json,
    extract::{Path, Query, State},
    response::IntoResponse,
};
use vm_data::models::streams::{StreamType, TokenInfoPayload};
use vm_nats::subjects;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::json;

use crate::{models::state::AppState, routes::middleware::Claims};
use super::is_evm_network;

#[derive(Deserialize)]
pub struct TokenInfoQuery {
    #[serde(default = "default_network")]
    pub network: String,
}

fn default_network() -> String {
    "solana".to_string()
}

pub async fn get_token_info_req(
    _claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
    Query(query): Query<TokenInfoQuery>,
) -> impl IntoResponse {
    tracing::info!("[API] Requesting token info for: {} (network: {})", token, query.network);

    let (subject, payload_bytes) = if is_evm_network(&query.network) {
        let request = StreamType::RequestTokenInfo(TokenInfoPayload {
            token: token.clone(),
            network: Some(query.network.clone()),
        });
        (
            subjects::rpc::evm::TOKEN_INFO,
            serde_json::to_vec(&request).unwrap_or_default(),
        )
    } else {
        let request = StreamType::RequestTokenInfo(TokenInfoPayload {
            token: token.clone(),
            network: None,
        });
        (
            subjects::rpc::sol::TOKEN_INFO,
            serde_json::to_vec(&request).unwrap_or_default(),
        )
    };

    let response = vm_nats::request_with_timeout(
        &state.nats_client,
        subject,
        payload_bytes,
        Duration::from_secs(120),
    ).await;

    match response {
        Ok(msg) => {
            tracing::debug!("[API] Token info response: {} bytes", msg.payload.len());
            if let Ok(stream_type) = serde_json::from_slice::<StreamType>(&msg.payload) {
                let data = match stream_type {
                    StreamType::ResponseTokenInfo(pools) => json!(pools),
                    _ => json!(null),
                };
                let body = Json(json!({ "data": data }));
                return (StatusCode::OK, body);
            }

            tracing::warn!("[API] Failed to deserialize token info response");
            let body = Json(json!({
                "error": "Failed to deserialize response"
            }));
            (StatusCode::INTERNAL_SERVER_ERROR, body)
        }
        Err(e) => {
            tracing::error!("[API] Token info request failed: {:?}", e);
            let body = Json(json!({
                "error": format!("Request failed: {}", e)
            }));
            (StatusCode::GATEWAY_TIMEOUT, body)
        }
    }
}
