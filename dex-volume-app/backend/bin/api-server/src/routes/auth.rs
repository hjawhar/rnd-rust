use std::{sync::Arc, time::Duration, time::SystemTime};

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::StatusCode,
    response::IntoResponse,
};
use vm_data::models::streams::{StreamType, VerifySignaturePayload};
use vm_data::models::user::{
    LoginNoncePayload, LoginPayload, LoginResponse, NewUser, RefreshResponse, RefreshTokenPayload,
};
use vm_data::utils::constants::KEYS;
use vm_data::utils::helpers::validate_evm_address;
use vm_nats::subjects;
use jsonwebtoken::{Algorithm, Header, encode};
use rand::Rng;
use serde_json::json;
use std::net::SocketAddr;
use uuid::Uuid;

use crate::{models::state::AppState, routes::middleware::Claims};

/// Returns a UNIX timestamp 24 hours from now.
fn jwt_expiry() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 24 * 60 * 60
}

/// Generates a 32-byte random hex string for use as a refresh token.
fn generate_refresh_token() -> String {
    let mut buf = [0u8; 32];
    rand::rng().fill(&mut buf);
    hex::encode(buf)
}

/// Returns a SystemTime 30 days from now for refresh token expiry.
fn refresh_token_expiry() -> SystemTime {
    SystemTime::now() + Duration::from_secs(30 * 24 * 60 * 60)
}

/// Validate a Solana address by checking it's valid base58 and exactly 32 bytes.
pub fn validate_solana_address(address: &str) -> Option<String> {
    let bytes = bs58::decode(address).into_vec().ok()?;
    if bytes.len() == 32 {
        Some(address.to_string())
    } else {
        None
    }
}


/// Detect chain and validate address. Returns (validated_address, chain).
/// chain is "solana" for Solana addresses, "evm" for EVM addresses.
pub fn validate_address(address: &str) -> Option<(String, &'static str)> {
    if address.starts_with("0x") {
        validate_evm_address(address).map(|a| (a, "evm"))
    } else {
        validate_solana_address(address).map(|a| (a, "solana"))
    }
}

pub async fn login_user_nonce(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<LoginNoncePayload>,
) -> impl IntoResponse {
    let user_address = match validate_address(&payload.address) {
        Some((addr, _chain)) => addr,
        None => {
            let body = Json(json!({
                "error": "Error public key format",
            }));
            return (StatusCode::BAD_REQUEST, body).into_response();
        }
    };

    let user = state.get_db().get_user_by_address(&user_address).await.unwrap_or(None);
    let id = Uuid::new_v4();

    if user.is_none() {
        let body = Json(json!({
            "error": "Please contact an administrator to access the app"
        }));
        (StatusCode::UNAUTHORIZED, body).into_response()
    } else {
        let _res = state.get_db().update_user_nonce(&user_address, id).await;
        let body = Json(json!({
            "nonce": id.to_string(),
            "address":user_address.clone()
        }));
        (StatusCode::CREATED, body).into_response()
    }
}

pub async fn login_user(
    State(state): State<Arc<AppState>>,
    ConnectInfo(_addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<LoginPayload>,
) -> impl IntoResponse {
    let (user_address, chain) = match validate_address(&payload.address) {
        Some(result) => result,
        None => {
            let body = Json(json!({
                "error": "Error public key format",
            }));
            return (StatusCode::BAD_REQUEST, body).into_response();
        }
    };

    let current_user = match state.get_db().get_user_by_address(&user_address).await.unwrap_or(None) {
        Some(user) => user,
        None => {
            let body = Json(json!({
                "error": "User not found"
            }));
            return (StatusCode::NOT_FOUND, body).into_response();
        }
    };

    if !current_user.whitelisted {
        let body = Json(json!({
            "error": "Please contact an administrator to access the app"
        }));
        return (StatusCode::BAD_REQUEST, body).into_response();
    }

    // Verify signature via chain-appropriate worker NATS RPC
    let verify_payload = VerifySignaturePayload {
        user_pubkey: user_address.clone(),
        message: payload.nonce,
        signature: payload.signed_nonce,
    };
    let verify_subject = if chain == "evm" {
        subjects::rpc::evm::VERIFY_SIGNATURE
    } else {
        subjects::rpc::sol::VERIFY_SIGNATURE
    };
    let request_type = StreamType::RequestVerifySignature(verify_payload);
    let response_match: fn(StreamType) -> Option<bool> =
        |st| if let StreamType::ResponseVerifySignature(v) = st { v } else { None };
    let request_bytes = match serde_json::to_vec(&request_type) {
        Ok(b) => b,
        Err(_) => {
            let body = Json(json!({
                "error": "Failed to serialize verify request"
            }));
            return (StatusCode::INTERNAL_SERVER_ERROR, body).into_response();
        }
    };
    let verify_response = vm_nats::request_with_timeout(
        &state.nats_client,
        verify_subject,
        request_bytes,
        Duration::from_secs(15),
    )
    .await;
    let verified = match verify_response {
        Ok(msg) => match serde_json::from_slice::<StreamType>(&msg.payload) {
            Ok(st) => response_match(st).unwrap_or(false),
            _ => false,
        },
        Err(err) => {
            let body = Json(json!({
                "error": format!("Failed to verify nonce {:#?}", err)
            }));
            return (StatusCode::BAD_REQUEST, body).into_response();
        }
    };
    if !verified {
        let body = Json(json!({
            "error": format!("Failed to verify ownership")
        }));
        return (StatusCode::BAD_REQUEST, body).into_response();
    }

    let id = Uuid::new_v4();
    let session_id = Uuid::new_v4().to_string();
    let _res = state.get_db().update_user_nonce(&user_address, id).await;
    let claims = Claims {
        exp: jwt_expiry(),
        sub: "vm_app".to_string(),
        company: "vm_app".to_string(),
        id: current_user.id,
        address: current_user.address.clone(),
        whitelisted: current_user.whitelisted,
        group_id: current_user.group_id,
        session_id: Some(session_id.clone()),
    };

    let token = encode(&Header::new(Algorithm::EdDSA), &claims, &KEYS.encoding);

    match token {
        Ok(jwt) => {
            let refresh = generate_refresh_token();
            let expires = refresh_token_expiry();
            if let Err(e) = state
                .get_db()
                .set_user_refresh_token(current_user.id, &refresh, expires)
                .await
            {
                tracing::error!("Failed to store refresh token: {}", e);
            }
            if let Err(e) = state
                .get_db()
                .set_user_session_id(current_user.id, &session_id)
                .await
            {
                tracing::error!("Failed to store session ID: {}", e);
            }

            let login_response = LoginResponse {
                address: current_user.address.clone(),
                jwt,
                refresh_token: refresh,
                success: true,
            };

            (StatusCode::OK, Json(json!(login_response))).into_response()
        }
        Err(_err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to generate token",
        )
            .into_response(),
    }
}

pub async fn refresh_token_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RefreshTokenPayload>,
) -> impl IntoResponse {
    let user = match state
        .get_db()
        .get_user_by_refresh_token(&payload.refresh_token)
        .await
    {
        Ok(Some(u)) => u,
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Invalid refresh token" })),
            )
                .into_response();
        }
    };

    // Check expiry
    if let Some(expires_at) = user.refresh_token_expires_at {
        if SystemTime::now() > expires_at {
            let _ = state.get_db().clear_user_refresh_token(user.id).await;
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Refresh token expired" })),
            )
                .into_response();
        }
    } else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "Invalid refresh token" })),
        )
            .into_response();
    }

    if !user.whitelisted {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "User not whitelisted" })),
        )
            .into_response();
    }

    // Issue new JWT — carry forward the same session_id
    let claims = Claims {
        exp: jwt_expiry(),
        sub: "vm_app".to_string(),
        company: "vm_app".to_string(),
        id: user.id,
        address: user.address.clone(),
        whitelisted: user.whitelisted,
        group_id: user.group_id,
        session_id: user.session_id.clone(),
    };

    let token = match encode(&Header::new(Algorithm::EdDSA), &claims, &KEYS.encoding) {
        Ok(t) => t,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Failed to generate token" })),
            )
                .into_response();
        }
    };

    // Rotate refresh token
    let new_refresh = generate_refresh_token();
    let expires = refresh_token_expiry();
    if let Err(e) = state
        .get_db()
        .set_user_refresh_token(user.id, &new_refresh, expires)
        .await
    {
        tracing::error!("Failed to rotate refresh token: {}", e);
    }

    let response = RefreshResponse {
        jwt: token,
        refresh_token: new_refresh,
    };
    (StatusCode::OK, Json(json!(response))).into_response()
}

pub async fn logout_handler(
    State(state): State<Arc<AppState>>,
    claims: Claims,
) -> impl IntoResponse {
    if let Err(e) = state.get_db().clear_user_refresh_token(claims.id).await {
        tracing::error!("Failed to clear refresh token: {}", e);
    }
    // Invalidate cached session so the logout takes effect immediately
    let _ = vm_redis::del_redis(&format!("session:{}", claims.id)).await;
    (StatusCode::OK, Json(json!({ "success": true }))).into_response()
}

pub async fn add_user_manually(state: &Arc<AppState>, address: &str, group_id: i32) {
    let user_address = match validate_address(address) {
        Some((addr, _chain)) => addr,
        None => {
            return;
        }
    };

    let user = state.get_db().get_user_by_address(&user_address).await.unwrap_or(None);
    let id = Uuid::new_v4();

    if user.is_none() {
        let new_user = NewUser {
            address: user_address.clone(),
            nonce: id.clone().to_string(),
            whitelisted: true,
            group_id,
            date_added: SystemTime::now(),
        };

        let _ = state.get_db().add_user(&new_user).await;
    }
}
