use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use uuid::Uuid;

use super::jwt;
use crate::state::AppState;

/// Extractor that validates JWT and provides the authenticated user info.
pub struct AuthUser {
    pub user_id: Uuid,
    pub username: String,
    pub role: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = StatusCode;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let claims =
            jwt::validate_token(token, &state.jwt_secret).map_err(|_| StatusCode::UNAUTHORIZED)?;

        Ok(AuthUser {
            user_id: claims.sub,
            username: claims.username,
            role: claims.role,
        })
    }
}
