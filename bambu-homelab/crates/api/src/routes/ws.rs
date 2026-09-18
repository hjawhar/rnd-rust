use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    extract::ws::{Message, WebSocket},
    extract::{State, WebSocketUpgrade},
    response::IntoResponse,
};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::RwLock;
use tracing::{debug, info};
use uuid::Uuid;

use crate::auth::jwt;
use crate::db::schema::printer_assignments;
use crate::models::{WsClientMessage, WsServerMessage};
use crate::state::AppState;

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

struct WsSession {
    user_id: Uuid,
    role: String,
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Auth handshake: first message must be auth within 5 seconds
    let session = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        wait_for_auth(&mut receiver, &state),
    )
    .await
    {
        Ok(Some(session)) => session,
        _ => {
            let err = serde_json::to_string(&WsServerMessage::Error {
                message: "authentication required".into(),
            })
            .unwrap_or_default();
            let _ = sender.send(Message::Text(err.into())).await;
            let _ = sender
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 4001,
                    reason: "unauthorized".into(),
                })))
                .await;
            return;
        }
    };

    info!(user_id = %session.user_id, role = %session.role, "WebSocket client authenticated");

    let mut broadcast_rx = state.ws_broadcast.subscribe();
    let sub_set: Arc<RwLock<HashSet<String>>> = Arc::new(RwLock::new(HashSet::new()));
    let sub_set_reader = sub_set.clone();
    let session_user_id = session.user_id;

    // Forward matching broadcast messages to this client
    let send_task = tokio::spawn(async move {
        while let Ok(msg) = broadcast_rx.recv().await {
            let should_send = match &msg {
                WsServerMessage::Telemetry { printer_id, .. } => {
                    sub_set_reader.read().await.contains(printer_id)
                }
                WsServerMessage::AssignmentAdded { target_user_id, .. } => {
                    *target_user_id == session_user_id
                }
                WsServerMessage::AssignmentRemoved {
                    target_user_id,
                    printer_id,
                } => {
                    if *target_user_id == session_user_id {
                        // Auto-unsubscribe from this printer
                        sub_set_reader.write().await.remove(printer_id);
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            };

            if should_send {
                let json = serde_json::to_string(&msg).unwrap_or_default();
                if sender.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    // Handle incoming messages from client
    let db = state.db.clone();
    let session_role = session.role.clone();
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => match serde_json::from_str::<WsClientMessage>(&text) {
                Ok(WsClientMessage::Subscribe { printer_ids }) => {
                    let allowed =
                        filter_allowed_printers(&db, &session.user_id, &session_role, &printer_ids)
                            .await;
                    let mut subs = sub_set.write().await;
                    for id in &allowed {
                        subs.insert(id.clone());
                    }
                    debug!(printers = ?allowed, "client subscribed");
                }
                Ok(WsClientMessage::Unsubscribe { printer_ids }) => {
                    let mut subs = sub_set.write().await;
                    for id in &printer_ids {
                        subs.remove(id);
                    }
                    debug!(printers = ?printer_ids, "client unsubscribed");
                }
                Ok(WsClientMessage::Auth { .. }) => {
                    // Already authenticated, ignore
                }
                Err(e) => {
                    debug!(error = %e, "invalid WS message");
                }
            },
            Message::Close(_) => break,
            _ => {}
        }
    }

    send_task.abort();
    info!(user_id = %session.user_id, "WebSocket client disconnected");
}

async fn wait_for_auth(
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
    state: &AppState,
) -> Option<WsSession> {
    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(text) = msg {
            if let Ok(WsClientMessage::Auth { token }) =
                serde_json::from_str::<WsClientMessage>(&text)
            {
                match jwt::validate_token(&token, &state.jwt_secret) {
                    Ok(claims) => {
                        return Some(WsSession {
                            user_id: claims.sub,
                            role: claims.role,
                        });
                    }
                    Err(_) => return None,
                }
            } else {
                return None;
            }
        }
    }
    None
}

/// Filter printer_ids to only those the user is assigned to (admins get all).
async fn filter_allowed_printers(
    db: &crate::db::Pool,
    user_id: &Uuid,
    role: &str,
    printer_ids: &[String],
) -> Vec<String> {
    if role == "admin" {
        return printer_ids.to_vec();
    }
    let Ok(mut conn) = db.get().await else {
        return Vec::new();
    };
    printer_assignments::table
        .filter(printer_assignments::user_id.eq(user_id))
        .filter(printer_assignments::printer_id.eq_any(printer_ids))
        .select(printer_assignments::printer_id)
        .load(&mut conn)
        .await
        .unwrap_or_default()
}
