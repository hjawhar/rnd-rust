use vm_data::utils::helpers::big_int_to_f64;
use vm_data::models::{
    project::{NewProjectPayload, UpdateProjectPayload},
    streams::StreamType,
};
use vm_nats::subjects;
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

pub async fn add_project_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<NewProjectPayload>,
) -> impl IntoResponse {
    let network = payload.network.clone();
    let st = match nats_rpc(
        &state.nats_client, &network,
        subjects::rpc::sol::PROJECT_NEW, subjects::rpc::evm::PROJECT_NEW,
        claims.id, StreamType::RequestNewProject(payload),
        Duration::from_secs(120),
    ).await {
        Ok(st) => st,
        Err(e) => return e.into_response(),
    };
    match st {
        StreamType::ResponseNewProject(Some(project)) => {
            state.ws_clients.add_project_for_user(claims.id, project.id).await;
            audit_log(&state, claims.id, Some(project.id), "project.create", Some(json!({
                "name": project.name, "network": project.network
            })));
            (StatusCode::OK, Json(json!({ "data": project }))).into_response()
        }
        _ => (StatusCode::BAD_REQUEST, Json(json!({ "error": "Failed to create project" }))).into_response(),
    }
}

pub async fn get_projects_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let projects = state.read_db().get_projects(claims.id).await.unwrap_or_default();
    let body = Json(json!({
        "data": projects
    }));
    (StatusCode::OK, body)
}

pub async fn get_all_projects_req(
    _claims: Claims,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let projects = state.read_db().get_all_projects().await.unwrap_or_default();
    let body = Json(json!({
        "data": projects
    }));
    (StatusCode::OK, body)
}

pub async fn get_project_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    let project = match state.read_db().get_project(claims.id, id).await.ok().flatten() {
        Some(project) => project,
        None => {
            let body = Json(json!({
                "error": "Error project not found"
            }));
            return (StatusCode::NOT_FOUND, body);
        }
    };
    let body = Json(json!({
        "data": project
    }));
    (StatusCode::OK, body)
}

pub async fn update_project_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Json(payload): Json<UpdateProjectPayload>,
) -> impl IntoResponse {
    let project = match state.get_db().get_project(claims.id, id).await.ok().flatten() {
        Some(project) => project,
        None => {
            let body = Json(json!({
                "error": "Error project not found"
            }));
            return (StatusCode::NOT_FOUND, body);
        }
    };
    if project.locked {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "Project is locked" })));
    }
    if project.trading_strategy == "BUY_SELL" {
        let body = Json(json!({
            "error": "Cannot update buy / sell project"
        }));
        return (StatusCode::BAD_REQUEST, body);
    }
    if project.trading_strategy == "VOLUME_MAKER" {
        let Some(ref trading_daily_volume) = payload.trading_daily_volume else {
            let body = Json(json!({
                "error": "Daily volume is missing"
            }));
            return (StatusCode::BAD_REQUEST, body);
        };
        let Some(ref trading_interval) = payload.trading_interval else {
            let body = Json(json!({
                "error": "Trading interval is missing"
            }));
            return (StatusCode::BAD_REQUEST, body);
        };

        if big_int_to_f64(trading_daily_volume.clone()) <= 0.0 {
            let body = Json(json!({
                "error": "Trading volume cannot be negative or null"
            }));
            return (StatusCode::BAD_REQUEST, body);
        }

        if big_int_to_f64(trading_interval.clone()) <= 0.0 {
            let body = Json(json!({
                "error": "Trading interval cannot be negative or null"
            }));
            return (StatusCode::BAD_REQUEST, body);
        }

        if let Some(bps) = payload.max_market_impact_bps
            && bps <= 0 {
                let body = Json(json!({
                    "error": "Max market impact bps must be greater than 0"
                }));
                return (StatusCode::BAD_REQUEST, body);
            }

        if let Some(multiplier) = payload.trade_multiplier
            && multiplier <= 0.0 {
                let body = Json(json!({
                    "error": "Trade multiplier must be greater than 0"
                }));
                return (StatusCode::BAD_REQUEST, body);
            }
    }

    let updated_task = UpdateProjectPayload {
        trading_interval: payload.trading_interval,
        trading_daily_volume: payload.trading_daily_volume,
        max_market_impact_bps: payload.max_market_impact_bps,
        trade_multiplier: payload.trade_multiplier,
        slippage: payload.slippage,
        bundle_enabled: payload.bundle_enabled,
        jito_tip: payload.jito_tip,
    };

    let _updated = state
        .get_db()
        .update_project(&claims.id, &project.id, &updated_task)
        .await
        .unwrap_or(false);

    let project = match state
        .get_db()
        .get_project(claims.id, project.id)
        .await
        .ok()
        .flatten()
    {
        Some(project) => project,
        None => {
            let body = Json(json!({
                "error": "Failed to retrieve updated project"
            }));
            return (StatusCode::INTERNAL_SERVER_ERROR, body);
        }
    };

    audit_log(&state, claims.id, Some(project.id), "project.update", Some(json!(updated_task)));

    let body = Json(json!({
        "data": project,
        "message": "Successfully updated project"
    }));
    (StatusCode::OK, body)
}

pub async fn delete_project_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    // Look up project to determine network
    let project = match state.get_db().get_project(claims.id, id).await.ok().flatten() {
        Some(p) => p,
        None => {
            let body = Json(json!({ "error": "Project not found" }));
            return (StatusCode::NOT_FOUND, body).into_response();
        }
    };

    if project.locked {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "Project is locked" }))).into_response();
    }

    let st = match nats_rpc(
        &state.nats_client, &project.network,
        subjects::rpc::sol::PROJECT_DELETE, subjects::rpc::evm::PROJECT_DELETE,
        claims.id, StreamType::RequestDeleteProject(id),
        Duration::from_secs(15),
    ).await {
        Ok(st) => st,
        Err(e) => return e.into_response(),
    };
    match st {
        StreamType::ResponseDeleteProject(Some(true)) => {
            state.ws_clients.remove_project_for_all(id).await;
            audit_log(&state, claims.id, Some(id), "project.delete", None);
            (StatusCode::OK, Json(json!({ "data": id, "message": "Successfully deleted project" }))).into_response()
        }
        StreamType::ResponseDeleteProject(None) => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found" }))).into_response()
        }
        _ => (StatusCode::BAD_REQUEST, Json(json!({ "error": "Error deleting project" }))).into_response(),
    }
}
