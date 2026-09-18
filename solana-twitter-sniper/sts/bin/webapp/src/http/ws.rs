use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::str;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::Utf8Bytes;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::Json;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use axum_extra::{headers, TypedHeader};
use futures::stream::StreamExt;
use futures_util::SinkExt;
use models::broadcast::WsBroadcastGeneric;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::http::middleware::token_is_valid;
use crate::models::state::AppState;
use crate::models::tasks::{Task, TaskWallet};
use crate::utils::encryption::decrypt;

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug)]
pub struct WebsocketPaylaod {
    pub token: String,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    Query(query): Query<WebsocketPaylaod>,
) -> impl IntoResponse {
    let valid_bot = query.token == state.x_api_key;
    let valid_user = token_is_valid(&query.token);
    let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        String::from("Unknown browser")
    };
    tracing::info!("`{user_agent}` at {addr} connected.");
    if valid_user || valid_bot {
        ws.on_upgrade(move |socket| handle_socket(socket, addr, state, valid_user, valid_bot))
    } else {
        tracing::info!("Failed to authenticate");
        let body = Json(json!({
            "error": "Failed to authenticate"
        }));
        return (StatusCode::FORBIDDEN, body).into_response();
    }
}

async fn handle_socket(
    socket: WebSocket,
    who: SocketAddr,
    state: Arc<AppState>,
    valid_user: bool,
    valid_bot: bool,
) {
    let mut rx = state.tx_broadcast.subscribe();
    let (mut sender, mut receiver) = socket.split();
    let (tx_user, mut rx_user) = mpsc::channel::<(String, bool)>(1000);
    let tx_pong = tx_user.clone();
    let tx_broadcast = tx_user.clone();
    let tx_tasks = tx_user.clone();

    let state_clone = state.clone();
    let state_clone1 = state.clone();
    let mut thread_handles: Vec<JoinHandle<()>> = vec![];
    tokio::task::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            let s = state_clone1.clone();
            match msg {
                Message::Text(ref t) => {
                    if t.to_string() != "PING" && t.to_string() != "PONG" {
                        if let Ok(res) = serde_json::from_str::<Value>(&t.to_string()) {
                            if let Some(current_type) = res.get("type") {
                                if let Some(val) = current_type.as_str() {
                                    if val != "SLOT_NUMBER" {
                                        tracing::info!(">>> {} sent str: {:?}", who, msg.clone());
                                    }
                                }
                            }
                        }
                    }
                    if valid_bot {
                        let _ = s.tx_broadcast.send((t.to_string(), true));
                    }
                }
                Message::Binary(d) => {
                    tracing::info!(">>> {} sent {} bytes: {:?}", who, d.len(), d);
                }
                Message::Close(c) => {
                    if let Some(cf) = c {
                        tracing::info!(
                            ">>> {} sent close with code {} and reason `{}`",
                            who,
                            cf.code,
                            cf.reason
                        );
                    } else {
                        // tracing::info!(">>> {} somehow sent close message without CloseFrame", who);
                    }
                    return ControlFlow::Break(());
                }
                Message::Pong(v) => {
                    tracing::info!(">>> {} sent pong with {:?}", who, v);
                }
                Message::Ping(v) => {
                    tracing::info!(">>> {} sent ping with {:?}", who, v);
                }
            }
        }
        tracing::info!("Websocket context {} destroyed", who);
        ControlFlow::Continue(())
    });

    thread_handles.push(tokio::task::spawn(async move {
        loop {
            let _ = tx_pong.send(("PING".to_string(), true)).await;
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }));

    thread_handles.push(tokio::task::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            // In any websocket error, break loop.
            match tx_broadcast.send(msg).await {
                Ok(res) => {
                    // tracing::info!("Successfully broadcasted message to ws clients {:#?}", data)
                }
                Err(err) => {
                    // tracing::info!("Error sending message to ws clients {:#?}", err);
                    break;
                }
            }
        }
    }));

    thread_handles.push(tokio::task::spawn(async move {
        while let Some((msg, user)) = rx_user.recv().await {
            // let msg_string = serde_json::to_string(&msg).unwrap();

            let is_ping = msg == "PING".to_string();

            if is_ping {
                // let msg_string = serde_json::to_string(&msg).unwrap();
                let data = Message::Text(Utf8Bytes::from("PING"));
                match sender.send(data.clone()).await {
                    Ok(res) => {
                        // tracing::info!(
                        //     "{} - Successfully ping ws clients {:#?}",
                        //     who.to_string().clone(),
                        //     data
                        // )
                    }
                    Err(err) => {
                        // tracing::info!("Error sending message to bot client {:#?}", err);
                        break;
                    }
                }
            } else {
                let data = Message::Text(Utf8Bytes::from(msg.clone()));
                if valid_bot && !user {
                    match sender.send(data.clone()).await {
                        Ok(res) => {
                            // tracing::info!("Successfully broadcasted message to ws clients {:#?}", data)
                        }
                        Err(err) => {
                            // tracing::info!("Error sending message to user client {:#?}", err);
                            break;
                        }
                    }
                }
                if valid_user && user {
                    // println!("Sending data to {:#?}", who);
                    match sender.send(data.clone()).await {
                        Ok(res) => {
                            // tracing::info!("Successfully broadcasted message to ws clients {:#?}", data)
                        }
                        Err(err) => {
                            // tracing::info!("Error sending message to user client {:#?}", err);
                            break;
                        }
                    }
                }
            }
        }
    }));

    let tasks = state_clone.get_db().get_tasks().await;
    let wallets = state.get_db().get_all_wallets().await;

    let tasks: Vec<TaskWallet> = tasks
        .iter()
        .map(|task| {
            let mut private_key = None;
            let mut public_key = None;
            let mut nonce_account_address = None;
            if let Some(wallet_id) = task.wallet_id {
                let found_wallet = wallets.iter().find(|wallet| wallet.id == wallet_id);
                if let Some(wallet) = found_wallet {
                    if let Ok(pk) = decrypt(wallet.pk.clone()) {
                        private_key = Some(pk.clone());
                        let encoded = bs58::decode(pk.clone()).into_vec();
                        if let Ok(encoded) = encoded {
                            let kp = Keypair::from_bytes(&encoded[..]);
                            if let Ok(kp) = kp {
                                public_key = Some(kp.pubkey().to_string());
                                nonce_account_address = wallet.nonce_account_address.clone();
                            }
                        }
                    }
                }
            }

            return TaskWallet {
                id: task.id.clone(),
                user_id: task.user_id.clone(),
                private_key,
                nonce_account_address,
                public_key,
                twitter_id: task.twitter_id.clone(),
                twitter_handle: task.twitter_handle.clone(),
                servers: task.servers.clone(),
                block_leaders: task.block_leaders.clone(),
                value: task.value.clone(),
                tip: task.tip.clone(),
                slippage: task.slippage.clone(),
                tries: task.tries.clone(),
                frontrunning_protection: task.frontrunning_protection.clone(),
                enable_alerts: task.enable_alerts.clone(),
                selected_pool: task.selected_pool.clone(),
                twitter_api: task.twitter_api.clone(),
                twitter_strategy: task.twitter_strategy.clone(),
                twitter_handle_checker: task.twitter_handle_checker.clone(),
                twitter_token_override: task.twitter_token_override.clone(),
                words: task.words.clone(),
            };
        })
        .collect();
    if valid_bot {
        let payload: WsBroadcastGeneric<Vec<TaskWallet>> = WsBroadcastGeneric {
            server: state.server_name.clone(),
            r#type: "SET_TASKS".to_string(),
            data: tasks.clone(),
        };
        let msg_string = serde_json::to_string(&payload).unwrap();
        let _ = tx_tasks.send((msg_string, false)).await;
    }
}
