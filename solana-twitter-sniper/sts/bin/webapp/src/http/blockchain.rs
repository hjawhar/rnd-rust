use std::{collections::HashMap, str::FromStr, sync::Arc};

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use futures::future;
use serde_json::json;
use solana_pubkey::Pubkey;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{signature::Keypair, signer::Signer};
use tokio::sync::Mutex;

use crate::{
    models::{
        claims::Claims,
        pool::Pool,
        state::AppState,
        wallet::{
            AddWalletsPayload, DeleteWalletsPayload, GenerateWalletPayload, NewWallet,
            ViewWalletsPayload, Wallet, WalletResponse,
        },
    },
    utils::encryption::{decrypt, encrypt},
};

pub async fn get_servers_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let body = Json(json!({
        "data": state.servers
    }));
    return (StatusCode::OK, body);
}

pub async fn get_block_leaders_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let body = Json(json!({
        "data": state.block_leaders
    }));
    return (StatusCode::OK, body);
}

pub async fn get_pools_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let pools: Vec<Pool> = vec![
        Pool {
            id: "ALL".to_string(),
            name: "All".to_string(),
        },
        Pool {
            id: "PUMPFUN_ONLY".to_string(),
            name: "Pumpfun Only".to_string(),
        },
        Pool {
            id: "EXCLUDE_PUMPFUN".to_string(),
            name: "Exclude Pumpfun".to_string(),
        },
    ];
    let body = Json(json!({
        "data": pools
    }));
    return (StatusCode::OK, body);
}
