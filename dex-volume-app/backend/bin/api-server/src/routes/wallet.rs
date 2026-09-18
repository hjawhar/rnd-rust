use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use vm_data::models::{
    streams::{
        DeleteWalletsRpcPayload, GenerateWalletsPayload, ImportWalletsPayload,
        StreamType, ViewWalletsRpcPayload,
    },
    wallet::{
        AddWalletsPayload, DeleteWalletsPayload, GenerateWalletPayload,
        ViewWalletsPayload,
    },
};
use vm_nats::subjects;
use serde_json::json;

use crate::{models::state::AppState, routes::middleware::Claims};
use super::helpers::{audit_log, nats_rpc};

pub async fn import_wallets_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Json(payload): Json<AddWalletsPayload>,
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
        let body = Json(json!({ "error": "Project is locked" }));
        return (StatusCode::FORBIDDEN, body).into_response();
    }

    let import_payload = ImportWalletsPayload {
        project_id: id,
        pks: payload.pks,
    };

    let st = match nats_rpc(
        &state.nats_client, &project.network,
        subjects::rpc::sol::WALLETS_IMPORT, subjects::rpc::evm::WALLETS_IMPORT,
        claims.id, StreamType::RequestImportWallets(import_payload),
        Duration::from_secs(15),
    ).await {
        Ok(st) => st,
        Err(e) => return e.into_response(),
    };

    let (is_success, is_not_found) = match &st {
        StreamType::ResponseImportWallets(Some(w)) => (!w.is_empty(), false),
        StreamType::ResponseImportWallets(None) => (false, true),
        _ => (false, false),
    };
    if is_success {
        audit_log(&state, claims.id, Some(id), "wallet.import", None);
        let body = Json(json!({ "data": "Successfully imported wallets" }));
        (StatusCode::OK, body).into_response()
    } else if is_not_found {
        let body = Json(json!({ "error": "Project not found" }));
        (StatusCode::NOT_FOUND, body).into_response()
    } else {
        let body = Json(json!({ "error": "No wallets can be imported as some of them might be already existing" }));
        (StatusCode::BAD_REQUEST, body).into_response()
    }
}

pub async fn generate_wallets_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Json(payload): Json<GenerateWalletPayload>,
) -> impl IntoResponse {
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

    let gen_payload = GenerateWalletsPayload {
        project_id: id,
        count: payload.value,
    };

    let st = match nats_rpc(
        &state.nats_client, &project.network,
        subjects::rpc::sol::WALLETS_GENERATE, subjects::rpc::evm::WALLETS_GENERATE,
        claims.id, StreamType::RequestGenerateWallets(gen_payload),
        Duration::from_secs(15),
    ).await {
        Ok(st) => st,
        Err(e) => return e.into_response(),
    };

    match st {
        StreamType::ResponseGenerateWallets(Some(_)) => {
            audit_log(&state, claims.id, Some(id), "wallet.generate", Some(json!({ "count": payload.value })));
            let body = Json(json!({ "data": "Successfully generated wallets" }));
            (StatusCode::OK, body).into_response()
        }
        StreamType::ResponseGenerateWallets(None) => {
            let body = Json(json!({ "error": "Project not found" }));
            (StatusCode::NOT_FOUND, body).into_response()
        }
        _ => {
            let body = Json(json!({ "error": "Failed to generate wallets" }));
            (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
        }
    }
}

pub async fn view_wallets_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Json(payload): Json<ViewWalletsPayload>,
) -> impl IntoResponse {
    let project = match state.get_db().get_project(claims.id, id).await.ok().flatten() {
        Some(p) => p,
        None => {
            let body = Json(json!({ "error": "Project not found" }));
            return (StatusCode::NOT_FOUND, body).into_response();
        }
    };

    let view_payload = ViewWalletsRpcPayload {
        project_id: id,
        ids: payload.ids,
    };

    let st = match nats_rpc(
        &state.nats_client, &project.network,
        subjects::rpc::sol::WALLETS_VIEW, subjects::rpc::evm::WALLETS_VIEW,
        claims.id, StreamType::RequestViewWallets(view_payload),
        Duration::from_secs(15),
    ).await {
        Ok(st) => st,
        Err(e) => return e.into_response(),
    };

    match st {
        StreamType::ResponseViewWallets(Some(w)) if !w.is_empty() => {
            (StatusCode::OK, Json(json!({ "data": w }))).into_response()
        }
        StreamType::ResponseViewWallets(None) => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": "Project not found" }))).into_response()
        }
        _ => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": "No wallets found" }))).into_response()
        }
    }
}

pub async fn delete_wallets_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
    Json(payload): Json<DeleteWalletsPayload>,
) -> impl IntoResponse {
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

    let wallet_ids = payload.ids.clone();
    let del_payload = DeleteWalletsRpcPayload {
        project_id: id,
        ids: payload.ids,
    };

    let st = match nats_rpc(
        &state.nats_client, &project.network,
        subjects::rpc::sol::WALLETS_DELETE, subjects::rpc::evm::WALLETS_DELETE,
        claims.id, StreamType::RequestDeleteWallets(del_payload),
        Duration::from_secs(15),
    ).await {
        Ok(st) => st,
        Err(e) => return e.into_response(),
    };

    match st {
        StreamType::ResponseDeleteWallets(Some(true)) => {
            audit_log(&state, claims.id, Some(id), "wallet.delete", Some(json!({ "wallet_ids": wallet_ids })));
            let body = Json(json!({ "data": "Successfully deleted wallets" }));
            (StatusCode::OK, body).into_response()
        }
        StreamType::ResponseDeleteWallets(None) => {
            let body = Json(json!({ "error": "Project not found" }));
            (StatusCode::NOT_FOUND, body).into_response()
        }
        _ => {
            let body = Json(json!({ "error": "Failed to delete wallets" }));
            (StatusCode::BAD_REQUEST, body).into_response()
        }
    }
}
