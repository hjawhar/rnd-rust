use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::SaltString;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use serde::Deserialize;
use tracing::info;

use super::jwt;
use super::middleware::AuthUser;
use crate::db::models::{NewUser, User};
use crate::db::schema::users;
use crate::models::{ChangePasswordRequest, LoginRequest, LoginResponse};
use crate::state::AppState;

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, StatusCode> {
    let mut conn = state
        .db
        .get()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let user: User = users::table
        .filter(users::username.eq(&req.username))
        .select(User::as_select())
        .first(&mut conn)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let parsed_hash =
        PasswordHash::new(&user.password_hash).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed_hash)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let token = jwt::create_token(user.id, &user.username, &user.role, &state.jwt_secret)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    info!(username = %user.username, "user logged in");

    Ok(Json(LoginResponse {
        token,
        username: user.username,
        role: user.role,
    }))
}

pub async fn change_password(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<StatusCode, StatusCode> {
    let mut conn = state
        .db
        .get()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let user: User = users::table
        .filter(users::id.eq(auth.user_id))
        .select(User::as_select())
        .first(&mut conn)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let parsed_hash =
        PasswordHash::new(&user.password_hash).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Argon2::default()
        .verify_password(req.current_password.as_bytes(), &parsed_hash)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let salt = SaltString::generate(&mut OsRng);
    let new_hash = Argon2::default()
        .hash_password(req.new_password.as_bytes(), &salt)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .to_string();

    diesel::update(users::table.filter(users::id.eq(auth.user_id)))
        .set(users::password_hash.eq(&new_hash))
        .execute(&mut conn)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    info!(username = %auth.username, "password changed");
    Ok(StatusCode::OK)
}


#[derive(Debug, serde::Serialize)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub role: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    #[serde(default = "default_role")]
    pub role: String,
}
fn default_role() -> String { "user".to_string() }

pub async fn list_users(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<UserInfo>>, StatusCode> {
    if auth.role != "admin" { return Err(StatusCode::FORBIDDEN); }
    let mut conn = state.db.get().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let list: Vec<User> = users::table.select(User::as_select()).load(&mut conn).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(list.into_iter().map(|u| UserInfo { id: u.id.to_string(), username: u.username, role: u.role }).collect()))
}

pub async fn create_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateUserRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if auth.role != "admin" { return Err((StatusCode::FORBIDDEN, "admin only".into())); }
    let mut conn = state.db.get().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(req.password.as_bytes(), &salt)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?.to_string();
    let u = NewUser { username: &req.username, password_hash: &hash, role: "user" };
    diesel::insert_into(users::table).values(&u).execute(&mut conn).await
        .map_err(|_| (StatusCode::CONFLICT, "username exists".into()))?;
    info!(username = %req.username, "user created");
    Ok(StatusCode::CREATED)
}

pub async fn delete_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if auth.role != "admin" { return Err((StatusCode::FORBIDDEN, "admin only".into())); }
    let mut conn = state.db.get().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let uid = uuid::Uuid::parse_str(&user_id).map_err(|_| (StatusCode::BAD_REQUEST, "invalid id".into()))?;
    if uid == auth.user_id { return Err((StatusCode::BAD_REQUEST, "cannot delete yourself".into())); }
    // Prevent deleting the root admin
    let target: User = users::table.find(uid).select(User::as_select()).first(&mut conn).await
        .map_err(|_| (StatusCode::NOT_FOUND, "user not found".into()))?;
    if target.role == "admin" {
        return Err((StatusCode::FORBIDDEN, "cannot delete admin user".into()));
    }
    diesel::delete(users::table.find(uid)).execute(&mut conn).await
        .map_err(|_| (StatusCode::NOT_FOUND, "not found".into()))?;
    Ok(StatusCode::NO_CONTENT)
}