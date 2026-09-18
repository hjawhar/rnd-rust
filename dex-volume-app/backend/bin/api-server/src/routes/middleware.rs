use std::sync::Arc;

use axum::{
    Extension, Json, RequestPartsExt,
    extract::{FromRequestParts, Request},
    http::{StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use vm_data::utils::constants::KEYS;
use jsonwebtoken::{Algorithm, Validation, decode};
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::models::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub exp: u64,
    pub sub: String,
    pub company: String,
    pub id: i32,
    pub address: String,
    pub group_id: i32,
    pub whitelisted: bool,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuthBody {
    pub access_token: String,
    pub token_type: String,
}

#[derive(Debug)]
pub enum AuthError {
    WrongCredentials,
    MissingCredentials,
    TokenCreation,
    InvalidToken,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            AuthError::WrongCredentials => (StatusCode::UNAUTHORIZED, "Wrong credentials"),
            AuthError::MissingCredentials => (StatusCode::BAD_REQUEST, "Missing credentials"),
            AuthError::TokenCreation => (StatusCode::INTERNAL_SERVER_ERROR, "Token creation error"),
            AuthError::InvalidToken => (StatusCode::UNAUTHORIZED, "Invalid token"),
        };
        let body = Json(json!({
            "error": error_message,
        }));
        (status, body).into_response()
    }
}

impl<S> FromRequestParts<S> for Claims
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Extract the token from the authorization header
        let TypedHeader(Authorization(bearer)) = parts
            .extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| AuthError::InvalidToken)?;
        // Decode the user data
        let validation = Validation::new(Algorithm::EdDSA);

        let token_data = decode::<Claims>(bearer.token(), &KEYS.decoding, &validation)
            .map_err(|_| AuthError::InvalidToken)?;

        Ok(token_data.claims)
    }
}

pub struct UserIp(pub String);
impl<S> FromRequestParts<S> for UserIp
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let real_ip = parts
            .headers
            .get("x-real-ip")
            .or_else(|| parts.headers.get("x-forwarded-for"))
            .and_then(|header| header.to_str().ok())
            .and_then(|header_str| header_str.split(',').next().map(|s| s.trim()));

        let real_ip = match real_ip {
            Some(real_ip) => real_ip,
            None => {
                let body = Json(json!({
                    "error": "Failed to fetch client ip address"
                }));
                return Err((StatusCode::FORBIDDEN, body).into_response());
            }
        };

        Ok(UserIp(real_ip.to_string()))
    }
}

pub fn get_claims(token: &str) -> Result<Claims, AuthError> {
    let validation = Validation::new(Algorithm::EdDSA);
    let token_data = decode::<Claims>(token, &KEYS.decoding, &validation)
        .map_err(|_| AuthError::InvalidToken)?;
    Ok(token_data.claims)
}

pub fn token_is_valid(token: &str) -> bool {
    let validation = Validation::new(Algorithm::EdDSA);
    let token_data = decode::<Claims>(token, &KEYS.decoding, &validation);
    token_data.is_ok()
}

/// Session TTL in Redis (seconds). A compromised session can remain valid
/// for at most this duration after being invalidated in the DB.
const SESSION_CACHE_TTL: u64 = 60;

async fn validate_session(claims: &Claims, state: &AppState) -> Result<(), StatusCode> {
    let session_id = claims.session_id.as_deref().ok_or(StatusCode::UNAUTHORIZED)?;
    let cache_key = format!("session:{}", claims.id);

    // Try Redis cache first
    if let Ok(Some(cached)) = vm_redis::get_redis_data::<String>(cache_key.clone()).await {
        return if cached == session_id {
            Ok(())
        } else {
            Err(StatusCode::UNAUTHORIZED)
        };
    }

    // Cache miss — fall back to DB
    let db_session = state
        .get_db()
        .get_user_session_id(claims.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match db_session {
        Some(ref db_sid) if db_sid == session_id => {
            // Populate cache on successful validation
            let _ = vm_redis::insert_redis_data_with_ttl(
                cache_key,
                db_sid,
                SESSION_CACHE_TTL,
            ).await;
            Ok(())
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

pub async fn guard(
    TypedHeader(bearer): TypedHeader<Authorization<Bearer>>,
    Extension(state): Extension<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let validation = Validation::new(Algorithm::EdDSA);
    let token_data = decode::<Claims>(bearer.token(), &KEYS.decoding, &validation)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    if !token_data.claims.whitelisted {
        return Err(StatusCode::UNAUTHORIZED);
    }

    validate_session(&token_data.claims, &state).await?;

    Ok(next.run(request).await)
}

pub async fn guard_admin(
    TypedHeader(bearer): TypedHeader<Authorization<Bearer>>,
    Extension(state): Extension<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let validation = Validation::new(Algorithm::EdDSA);
    let token_data = decode::<Claims>(bearer.token(), &KEYS.decoding, &validation)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    if !token_data.claims.whitelisted || token_data.claims.group_id != 1 {
        return Err(StatusCode::UNAUTHORIZED);
    }

    validate_session(&token_data.claims, &state).await?;

    Ok(next.run(request).await)
}
