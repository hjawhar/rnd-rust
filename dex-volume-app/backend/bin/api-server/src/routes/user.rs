use std::{sync::Arc, time::SystemTime};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use vm_data::models::audit_log::AuditLogQuery;
use vm_data::models::user::{AddUserPayload, NewUser};
use serde_json::json;
use uuid::Uuid;

use crate::{models::state::AppState, routes::{auth::validate_address, middleware::Claims}};

use super::helpers::audit_log;

pub async fn grant_project_access_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path((user_id, project_id)): Path<(i32, i32)>,
) -> impl IntoResponse {
    // Verify user exists
    let user = state.get_db().get_user_by_id(user_id).await.ok().flatten();
    if user.is_none() {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "User not found" }))).into_response();
    }

    // Verify project exists (admin endpoint — no user_id filter)
    let project = match state.get_db().get_project_by_id(project_id).await.ok().flatten() {
        Some(p) => p,
        None => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found" }))).into_response();
        }
    };

    // Cannot grant access to the owner
    if project.user_id == user_id {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "User is already the project owner" }))).into_response();
    }

    match state.get_db().grant_project_access(project_id, user_id).await {
        Ok(access) => {
            state.ws_clients.add_project_for_user(user_id, project_id).await;
            audit_log(&state, claims.id, Some(project_id), "access.grant", Some(json!({ "target_user_id": user_id })));
            (StatusCode::OK, Json(json!({ "data": access, "message": "Access granted" }))).into_response()
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("duplicate key") || msg.contains("unique") {
                (StatusCode::CONFLICT, Json(json!({ "error": "Access already granted" }))).into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to grant access: {}", msg) }))).into_response()
            }
        }
    }
}

pub async fn revoke_project_access_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path((user_id, project_id)): Path<(i32, i32)>,
) -> impl IntoResponse {
    match state.get_db().revoke_project_access(project_id, user_id).await {
        Ok(true) => {
            state.ws_clients.remove_project_for_user(user_id, project_id).await;
            audit_log(&state, claims.id, Some(project_id), "access.revoke", Some(json!({ "target_user_id": user_id })));
            (StatusCode::OK, Json(json!({ "message": "Access revoked" })))
        }
        Ok(false) => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": "Access not found" })))
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to revoke access: {}", e) })))
        }
    }
}

pub async fn get_project_access_req(
    _claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<i32>,
) -> impl IntoResponse {
    match state.read_db().get_project_access_users(project_id).await {
        Ok(access_list) => {
            (StatusCode::OK, Json(json!({ "data": access_list })))
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed to get access list: {}", e) })))
        }
    }
}

pub async fn get_users_req(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let users = state.read_db().get_users().await.unwrap_or_default();
    let body = Json(json!({
        "data":  users
    }));
    (StatusCode::OK, body).into_response()
}

pub async fn add_user_req(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AddUserPayload>,
) -> impl IntoResponse {
    if payload.group_id != 3 {
        let body = Json(json!({
            "error":  "Only user of trader type is allowed to be added!"
        }));
        return (StatusCode::BAD_REQUEST, body).into_response();
    }

    let user_address = match validate_address(&payload.address) {
        Some((addr, _chain)) => addr,
        None => {
            let body = Json(json!({
                "error": "Error public key format",
            }));
            return (StatusCode::BAD_REQUEST, body).into_response();
        }
    };

    let existing_user = state.get_db().get_user_by_address(&user_address).await.unwrap_or(None);
    if existing_user.is_some() {
        let body = Json(json!({
            "error": "User already exists"
        }));
        return (StatusCode::BAD_REQUEST, body).into_response();
    }

    let id = Uuid::new_v4();

    let new_user = NewUser {
        address: user_address.clone(),
        nonce: id.clone().to_string(),
        whitelisted: true,
        group_id: payload.group_id,
        date_added: SystemTime::now(),
    };

    let new_user = match state.get_db().add_user(&new_user).await {
        Ok(u) => u,
        Err(_) => {
            let body = Json(json!({
                "error": "Failed to add user"
            }));
            return (StatusCode::INTERNAL_SERVER_ERROR, body).into_response();
        }
    };
    let body = Json(json!({
        "data": new_user,
        "message": format!("Successfully added {}", user_address)
    }));
    (StatusCode::OK, body).into_response()
}

pub async fn whitelist_user_req(
    _claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    let user_id = id;
    let user = state.get_db().get_user_by_id(user_id).await.ok().flatten();
    if let Some(user) = user {
        if user.group_id == 1 {
            let body = Json(json!({
                "error": "User not allowed to be modified"
            }));
            return (StatusCode::FORBIDDEN, body).into_response();
        }
        let new_user = NewUser {
            address: user.address,
            nonce: user.nonce,
            whitelisted: !user.whitelisted,
            group_id: user.group_id,
            date_added: SystemTime::now(),
        };
        let updated = state.get_db().update_user(&user_id, &new_user).await.unwrap_or(false);
        let body = Json(json!({
            "data":  if updated {
                "Successfully updated user"
            } else {
                "Failed to update user"
            }
        }));
        (StatusCode::OK, body).into_response()
    } else {
        let body = Json(json!({
            "error":  "User not found"
        }));
        (StatusCode::NOT_FOUND, body).into_response()
    }
}

pub async fn get_audit_logs_req(
    _claims: Claims,
    State(state): State<Arc<AppState>>,
    Query(params): Query<AuditLogQuery>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(50).min(1000);
    let offset = params.offset.unwrap_or(0);

    match state
        .read_db()
        .get_audit_logs(params.user_id, params.project_id, limit, offset)
        .await
    {
        Ok(logs) => (StatusCode::OK, Json(json!({ "data": logs }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to fetch audit logs: {}", e) })),
        ),
    }
}
