use std::sync::Arc;

use axum::{extract::State, response::IntoResponse, Json};
use models::broadcast::WsBroadcastGeneric;
use reqwest::StatusCode;
use serde_json::json;

use crate::{
    models::{
        buy::{BuyRequestOutgoingPayload, BuyRequestPayload},
        claims::Claims,
        state::AppState,
        tasks::TaskWallet,
    },
    utils::encryption::decrypt,
};

pub async fn buy_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<BuyRequestPayload>,
) -> impl IntoResponse {
    let current_logged_in_user = state.get_db().get_user_by_id(claims.id).await;
    let current_logged_in_user = match current_logged_in_user {
        Some(user) => user,
        None => {
            let body = Json(json!({
                "error": format!("User not logged in properly")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }
    };
    let wallet = state.get_db().get_wallet(&payload.wallet_id).await;
    let wallet = match wallet {
        Some(wallet) => wallet,
        None => {
            let body = Json(json!({
                "error": "Wallet not found"
            }));
            return (StatusCode::NOT_FOUND, body);
        }
    };
    if current_logged_in_user.id != wallet.user_id && current_logged_in_user.group_id == 3 {
        let body = Json(json!({
            "error": format!("Failed to assign wallet that doesn't belong to you")
        }));
        return (StatusCode::BAD_REQUEST, body);
    }

    let private_key = decrypt(wallet.pk);
    let private_key = match private_key {
        Ok(private_key) => private_key,
        Err(err) => {
            let body = Json(json!({
                "error": format!("Failed to decrypt wallet: {:#?}", err)
            }));
            return (StatusCode::BAD_REQUEST, body);
        }
    };

    let nonce_account_address = match wallet.nonce_account_address {
        Some(nonce_account_address) => nonce_account_address,
        None => {
            let body = Json(json!({
                "error": format!("Please create a nonce account first")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }
    };

    let payload: WsBroadcastGeneric<BuyRequestOutgoingPayload> = WsBroadcastGeneric {
        server: state.server_name.clone(),
        r#type: "BUY_REQUEST".to_string(),
        data: BuyRequestOutgoingPayload {
            servers: payload.servers.clone(),
            block_leaders: payload.block_leaders.clone(),
            value: payload.value.clone(),
            tip: payload.tip.clone(),
            slippage: payload.slippage.clone(),
            tries: payload.tries.clone(),
            frontrunning_protection: payload.frontrunning_protection.clone(),
            enable_alerts: payload.enable_alerts.clone(),
            selected_pool: payload.selected_pool.clone(),
            mint_address: payload.mint_address,
            private_key,
            nonce_account_address,
        },
    };

    let msg_string = serde_json::to_string(&payload).unwrap();
    let _ = state.tx_broadcast.send((msg_string, false));

    let body = Json(json!({
        "data": "Successfully triggered buy request"
    }));
    return (StatusCode::OK, body);
}
