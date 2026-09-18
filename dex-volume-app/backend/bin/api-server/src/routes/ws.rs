// TODO: Frontend update required — `initSocket()` in `main.component.ts` must:
//   1. Open the WebSocket WITHOUT the token in the URL
//   2. Send the JWT as the first text message after connection opens
//   3. Wait for the `{ "type": "AUTH", "success": true }` response before considering the connection ready

use std::net::SocketAddr;
use std::ops::ControlFlow;
use std::sync::Arc;

use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::Utf8Bytes;
use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use axum_extra::{TypedHeader, headers};
use futures::stream::StreamExt;
use futures_util::SinkExt;
use vm_data::models::streams::StreamType;
use vm_data::models::ws::WsBroadcastGeneric;
use vm_nats::subjects;
use serde_json::json;

use crate::models::state::AppState;
use crate::routes::middleware::get_claims;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let user_agent_str = user_agent
        .map(|TypedHeader(ua)| ua.to_string())
        .unwrap_or_else(|| "Unknown".into());
    ws.on_upgrade(move |socket| handle_socket(user_agent_str, socket, addr, state))
}

async fn handle_socket(
    user_agent: String,
    socket: WebSocket,
    who: SocketAddr,
    state: Arc<AppState>,
) {
    let (mut sender, mut receiver) = socket.split();
    let real_ip = who.ip().to_string();

    // Wait for auth message (first message must be JWT)
    let claims = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        receiver.next(),
    )
    .await
    {
        Ok(Some(Ok(Message::Text(token_msg)))) => {
            match get_claims(token_msg.as_str()) {
                Ok(claims) => {
                    // Validate session
                    let session_valid = if let Some(ref sid) = claims.session_id {
                        state
                            .get_db()
                            .get_user_session_id(claims.id)
                            .await
                            .ok()
                            .flatten()
                            .as_deref()
                            == Some(sid.as_str())
                    } else {
                        false
                    };
                    if !session_valid {
                        tracing::info!("[WS] [{real_ip}] Invalid session");
                        let _ = sender
                            .send(Message::Text(Utf8Bytes::from(
                                json!({ "error": "Session expired" }).to_string(),
                            )))
                            .await;
                        let _ = sender.close().await;
                        return;
                    }
                    claims
                }
                Err(_) => {
                    tracing::info!("[WS] [{real_ip}] Invalid token");
                    let _ = sender
                        .send(Message::Text(Utf8Bytes::from(
                            json!({ "error": "Invalid token" }).to_string(),
                        )))
                        .await;
                    let _ = sender.close().await;
                    return;
                }
            }
        }
        _ => {
            tracing::info!("[WS] [{real_ip}] Auth timeout or invalid first message");
            let _ = sender.close().await;
            return;
        }
    };

    tracing::info!(
        "[WS] [{real_ip}] `{user_agent}` connected (user_id={})",
        claims.id
    );

    // Load authorized project IDs
    let project_ids: Vec<i32> = state
        .get_db()
        .get_projects(claims.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.id)
        .collect();

    // Register with WsClientManager
    let (client_id, mut rx) = state.ws_clients.register(claims.id, project_ids).await;

    // Send auth success confirmation
    let _ = sender
        .send(Message::Text(Utf8Bytes::from(
            json!({ "type": "AUTH", "success": true }).to_string(),
        )))
        .await;

    let state_clone = state.clone();
    let send_state = state.clone();

    // Spawn receive task: initial price sends + incoming message processing
    tokio::task::spawn(async move {
        // Request SOL price via NATS request-reply, send only to this user
        if let Ok(msg) = vm_nats::request_with_timeout(
            &state.nats_client,
            subjects::rpc::sol::PRICE,
            vec![],
            std::time::Duration::from_secs(5),
        )
        .await
            && let Ok(stream_type) = serde_json::from_slice::<StreamType>(&msg.payload)
            && let StreamType::ResponsePrice(sol_price) = stream_type
        {
            let ws_broadcast_payload = WsBroadcastGeneric::<StreamType> {
                r#type: "SOL_PRICE".to_string(),
                data: StreamType::ResponsePrice(sol_price),
                id: Some(claims.id),
                message: None,
            };
            state_clone
                .send_to_user(&ws_broadcast_payload, claims.id)
                .await;
        }

        // Request ETH price
        if let Ok(msg) = vm_nats::request_with_timeout(
            &state.nats_client,
            subjects::rpc::evm::PRICE,
            vec![],
            std::time::Duration::from_secs(5),
        )
        .await
            && let Ok(stream_type) = serde_json::from_slice::<StreamType>(&msg.payload)
            && let StreamType::ResponsePrice(eth_price) = stream_type
        {
            let ws_broadcast_payload = WsBroadcastGeneric::<StreamType> {
                r#type: "ETH_PRICE".to_string(),
                data: StreamType::ResponsePrice(eth_price),
                id: Some(claims.id),
                message: None,
            };
            state_clone
                .send_to_user(&ws_broadcast_payload, claims.id)
                .await;
        }

        // Read loop
        while let Some(Ok(msg)) = receiver.next().await {
            let s = state_clone.clone();
            let _ = process_message(msg, who, s).await;
        }
        tracing::info!("[WS] [{real_ip}] Disconnected (user_id={})", claims.id);
        send_state.ws_clients.unregister(client_id).await;
    });

    // Send loop: forward messages from mpsc channel to WebSocket, with periodic pings
    let _send_task = tokio::task::spawn(async move {
        let mut ping_interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some(json) => {
                            let data = Message::Text(Utf8Bytes::from(json.as_str().to_owned()));
                            if sender.send(data).await.is_err() {
                                break;
                            }
                        }
                        None => break, // channel closed
                    }
                }
                _ = ping_interval.tick() => {
                    if sender.send(Message::Ping(vec![].into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
}

/// Process incoming WebSocket messages. Has special treatment for Close.
async fn process_message(
    msg: Message,
    who: SocketAddr,
    _state: Arc<AppState>,
) -> ControlFlow<(), ()> {
    match msg {
        Message::Text(t) => {
            tracing::debug!("[WS] {} sent str: {:?}", who, t);
        }
        Message::Binary(d) => {
            tracing::debug!("[WS] {} sent {} bytes", who, d.len());
        }
        Message::Close(c) => {
            if let Some(cf) = c {
                tracing::debug!(
                    "[WS] {} sent close with code {} and reason `{}`",
                    who,
                    cf.code,
                    cf.reason
                );
            } else {
                tracing::debug!("[WS] {} sent close without CloseFrame", who);
            }
            return ControlFlow::Break(());
        }
        Message::Pong(_) | Message::Ping(_) => {}
    }
    ControlFlow::Continue(())
}
