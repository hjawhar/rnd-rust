use std::{str::FromStr, sync::Arc};

use alloy::{primitives::Address, signers::Signature};
use axum::{
    extract::{ConnectInfo, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use jsonwebtoken::{encode, Algorithm, Header};
use serde_json::json;
use std::net::SocketAddr;
use uuid::Uuid;

use crate::{
    models::state::AppState,
    models::{
        claims::{AuthError, Claims},
        user::{LoginNoncePayload, LoginPayload, LoginResponse, NewUser},
    },
    utils::constants::KEYS,
};

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            AuthError::WrongCredentials => (StatusCode::UNAUTHORIZED, "Wrong credentials"),
            AuthError::MissingCredentials => (StatusCode::BAD_REQUEST, "Missing credentials"),
            AuthError::TokenCreation => (StatusCode::INTERNAL_SERVER_ERROR, "Token creation error"),
            AuthError::InvalidToken => (StatusCode::BAD_REQUEST, "Invalid token"),
        };
        let body = Json(json!({
            "error": error_message,
        }));
        (status, body).into_response()
    }
}

pub async fn login_user_nonce(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<LoginNoncePayload>,
) -> impl IntoResponse {
    let user_address = match Address::from_str(&payload.address) {
        Ok(addy) => addy.to_string(),
        Err(_) => {
            let body = Json(json!({
                "error": "Error public key format",
            }));
            return (StatusCode::BAD_REQUEST, body).into_response();
        }
    };

    let users_data = state.get_db().get_user_by_address(&user_address).await;
    let id = Uuid::new_v4();

    if users_data.len() == 0 {
        let body = Json(json!({
            "error": "Please contact the administrator to access the app"
        }));
        return (StatusCode::UNAUTHORIZED, body).into_response();
    } else {
        let _res = state.get_db().update_user_nonce(&user_address, id).await;
        let body = Json(json!({
            "nonce": id.to_string(),
            "address":user_address.clone()
        }));
        return (StatusCode::CREATED, body).into_response();
    }
}

pub async fn login_user(
    State(state): State<Arc<AppState>>,
    ConnectInfo(_addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<LoginPayload>,
) -> impl IntoResponse {
    // let ip = addr.ip().clone();
    // let remote_ip = Some(&ip);
    let user_address = match Address::from_str(&payload.address) {
        Ok(addy) => addy.to_string(),
        Err(_) => {
            let body = Json(json!({
                "error": "Error public key format",
            }));
            return (StatusCode::BAD_REQUEST, body).into_response();
        }
    };

    // if res.is_ok() {
    let users_data = state.get_db().get_user_by_address(&user_address).await;
    if users_data.len() == 0 {
        let body = Json(json!({
            "error": "User not found"
        }));
        return (StatusCode::NOT_FOUND, body).into_response();
    }

    let current_user = &users_data[0];

    if !current_user.whitelisted {
        let body = Json(json!({
            "error": "Please contact the administrator to access the app"
        }));
        return (StatusCode::BAD_REQUEST, body).into_response();
    }

    let signed_nonce = &payload.signed_nonce;
    let signature = Signature::from_str(signed_nonce);
    if signature.is_err() {
        let body = Json(json!({
            "error": "Failed to check for signed nonce"
        }));
        return (StatusCode::BAD_REQUEST, body).into_response();
    }
    let recovered_result = signature.unwrap().recover_address_from_msg(payload.nonce);
    if recovered_result.is_err() {
        let body = Json(json!({
            "error": "Failed to recover nonce owner"
        }));
        return (StatusCode::BAD_REQUEST, body).into_response();
    }
    let recovered_address = recovered_result.unwrap();
    if recovered_address.ne(&Address::from_str(&payload.address).unwrap()) {
        let body = Json(json!({
            "error": "Recovered address do not match current address"
        }));
        return (StatusCode::BAD_REQUEST, body).into_response();
    }

    let id = Uuid::new_v4();
    let _res = state.get_db().update_user_nonce(&user_address, id).await;
    let claims = Claims {
        exp: 2000000000,
        sub: "sts_app".to_string(),
        company: "sts_app".to_string(),
        id: current_user.id,
        address: current_user.address.clone(),
        whitelisted: current_user.whitelisted,
        group_id: current_user.group_id,
    };

    let token = encode(&Header::new(Algorithm::EdDSA), &claims, &KEYS.encoding);

    match token {
        Ok(jwt) => {
            let login_respose = LoginResponse {
                address: current_user.address.clone(),
                jwt,
                success: true,
            };

            return (StatusCode::OK, Json(json!(login_respose))).into_response();
        }
        Err(_err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to generate token",
            )
                .into_response();
        }
    }
}

pub async fn add_user_manually(state: &Arc<AppState>, address: &str, group_id: i32) {
    let user_address = match Address::from_str(address) {
        Ok(addy) => addy.to_string(),
        Err(_) => {
            return;
        }
    };

    let users_data = state.get_db().get_user_by_address(&user_address).await;
    let id = Uuid::new_v4();

    if users_data.len() == 0 {
        let new_user = NewUser {
            address: user_address.clone(),
            nonce: id.clone().to_string(),
            whitelisted: true,
            group_id,
        };

        state.get_db().add_user(&new_user).await;
    }
}
