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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::models::axsys::TweetEvent;
use crate::models::mpsc::{MpscLogs, MpscTask};
use crate::models::state::AppState;

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize, Debug)]
pub struct WebsocketPaylaod {
    pub token: String,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    _user_agent: Option<TypedHeader<headers::UserAgent>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    Query(query): Query<WebsocketPaylaod>,
) -> impl IntoResponse {
    let valid_bot = query.token == state.axsys_external_api_key;
    // let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
    //     user_agent.to_string()
    // } else {
    //     String::from("Unknown browser")
    // };
    // tracing::info!("`{user_agent}` at {addr} connected.");
    // let cloned_state = state.clone();
    if valid_bot {
        ws.on_upgrade(move |socket| handle_socket(socket, addr, state))
    } else {
        tracing::info!("Failed to authenticate");
        let body = Json(json!({
            "error": "Failed to authenticate"
        }));
        return (StatusCode::FORBIDDEN, body).into_response();
    }
}

async fn handle_socket(socket: WebSocket, who: SocketAddr, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let (tx_user, mut rx_user) = mpsc::channel::<(MpscLogs, bool)>(1000);
    let tx_pong = tx_user.clone();

    let state_clone = state.clone();
    let mut thread_handles: Vec<JoinHandle<()>> = vec![];
    tokio::task::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            let s = state_clone.clone();
            match msg {
                Message::Text(t) => {
                    if t.as_str() != "PING" && t.as_str() != "PONG" {
                        match serde_json::from_str::<Value>(t.as_str()) {
                            Ok(data) => {
                                tracing::info!(">>> {} sent str: {:?}", who, t);
                                if let Some(current_type) = data.get("type") {
                                    let current_type = current_type.as_str().unwrap();
                                    if current_type == "tweet.mini.update"
                                        || current_type == "tweet.update"
                                    {
                                        let tweet = serde_json::from_value::<TweetEvent>(data);
                                        match tweet {
                                            Ok(tweet) => {
                                                let _ =
                                                    s.tx_task.send(MpscTask::Tweet(tweet)).await;
                                            }
                                            Err(err) => {
                                                tracing::info!(
                                                    "Error deserializing tweet from ws {:#?}",
                                                    err
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            Err(err) => tracing::info!("{err:#?}"),
                        }
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
                // You should never need to manually handle Message::Ping, as axum's websocket library
                // will do so for you automagically by replying with Pong and copying the v according to
                // spec. But if you need the contents of the pings you can see them here.
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
            let _ = tx_pong
                .send((MpscLogs::Ping("PING".to_string()), false))
                .await;
            // latencies_2
            // .insert_ws_client(
            //     who.clone().to_string(),
            //     Some(get_current_time_ms() as i128),
            //     None,
            // )
            // .await;
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }));

    thread_handles.push(tokio::task::spawn(async move {
        while let Some((msg, _)) = rx_user.recv().await {
            // let msg_string = serde_json::to_string(&msg).unwrap();
            // let data = Message::Text(Utf8Bytes::from(msg_string));

            let is_ping = match msg {
                MpscLogs::Ping(_) => true,
                _ => false,
            };

            if is_ping {
                // let msg_string = serde_json::to_string(&msg).unwrap();
                let data = Message::Text(Utf8Bytes::from("PING"));
                match sender.send(data.clone()).await {
                    Ok(_) => {
                        // tracing::info!(
                        //     "{} - Successfully ping ws clients {:#?}",
                        //     who.to_string().clone(),
                        //     data
                        // )
                    }
                    Err(_) => {
                        // tracing::info!("Error sending message to bot client {:#?}", err);
                        break;
                    }
                }
            }
        }
    }));
}
