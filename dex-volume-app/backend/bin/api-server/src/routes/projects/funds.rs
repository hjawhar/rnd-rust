use vm_data::models::streams::StreamType;
use vm_nats::subjects;
use std::sync::Arc;
use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use crate::models::state::AppState;
use crate::routes::middleware::Claims;
use super::tasks::chain_aware_project_op;

pub async fn collect_sol_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    chain_aware_project_op(
        &state, claims.id, id,
        StreamType::RequestCollectNative,
        subjects::rpc::sol::PROJECT_COLLECT_SOL, subjects::rpc::evm::PROJECT_COLLECT_ETH,
        "Successfully started collecting", "Failed to collect",
        "collect.native",
    ).await
}

pub async fn collect_tokens_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    chain_aware_project_op(
        &state, claims.id, id,
        StreamType::RequestCollectTokens,
        subjects::rpc::sol::PROJECT_COLLECT_TOKENS, subjects::rpc::evm::PROJECT_COLLECT_TOKENS,
        "Successfully started collecting tokens", "Failed to collect tokens",
        "collect.tokens",
    ).await
}

pub async fn disperse_sol_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    chain_aware_project_op(
        &state, claims.id, id,
        StreamType::RequestDisperseNative,
        subjects::rpc::sol::PROJECT_DISPERSE_SOL, subjects::rpc::evm::PROJECT_DISPERSE_ETH,
        "Successfully started dispersing", "Failed to disperse",
        "disperse.native",
    ).await
}

pub async fn disperse_tokens_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<i32>,
) -> impl IntoResponse {
    chain_aware_project_op(
        &state, claims.id, id,
        StreamType::RequestDisperseTokens,
        subjects::rpc::sol::PROJECT_DISPERSE_TOKENS, subjects::rpc::evm::PROJECT_DISPERSE_TOKENS,
        "Successfully started dispersing tokens", "Failed to disperse tokens",
        "disperse.tokens",
    ).await
}
