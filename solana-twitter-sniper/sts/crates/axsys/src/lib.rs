use std::{
    error::Error,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub mod models;
use async_recursion::async_recursion;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc::{self, Sender};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, protocol::Message, Utf8Bytes},
};

use crate::models::{AddWatchedProfilePayload, PlanResponse, TwitterMiniTweet, WatchProfiles};

pub fn get_current_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

#[derive(Clone)]
pub struct AxsysClient {
    pub id: String,
    pub timeout: u64,
    pub tx: Sender<Value>,
    pub api_key: String,
    pub axsys_ws_endpoint: String,
}

impl AxsysClient {
    pub fn new(
        id: String,
        axsys_ws_endpoint: String,
        api_key: String,
        tx: Sender<Value>,
        timeout: u64,
    ) -> Self {
        Self {
            id,
            axsys_ws_endpoint,
            api_key,
            timeout,
            tx,
        }
    }

    #[async_recursion]
    pub async fn connect(&mut self) {
        let mut req = self
            .axsys_ws_endpoint
            .clone()
            .into_client_request()
            .unwrap();
        let headers = req.headers_mut();
        headers.append("Authorization", self.api_key.parse().unwrap());

        match connect_async(req).await {
            Ok((stream, _)) => {
                tracing::info!("Successfully connected to axsys websocket");
                let (mut write, mut read) = stream.split();
                let (tx_task, mut rx_task) = mpsc::channel::<bool>(1000);
                let tx_task1 = tx_task.clone();
                let tx_task2 = tx_task.clone();
                tokio::task::spawn(async move {
                    while let Some(flag) = rx_task.recv().await {
                        if flag {
                            let data = Message::Text(Utf8Bytes::from("PONG"));
                            let _ = write.send(data).await;
                        } else {
                            let data = Message::Text(Utf8Bytes::from("PING"));
                            let res = write.send(data).await;
                            match res {
                                Ok(_) => {
                                    // tracing::info!("Successfully sent ping");
                                }
                                Err(err) => {
                                    tracing::info!("Error sending ping: {:#?}", err);
                                    break;
                                }
                            }
                        }
                    }
                });

                tokio::task::spawn(async move {
                    loop {
                        let _ = tx_task1.send(false).await;
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                });

                // tx_task.send(true).await;

                while let Some(data) = read.next().await {
                    match data {
                        Ok(text) => {
                            // println!("{:#?}", text);
                            // let text = text.to_string();
                            // if !text.is_empty() {
                            //     if text == "PING" || text == "PONG" || text == "" {
                            //         // tx_task.send(true).await;
                            //         tracing::info!("received pong {:#?}", text);
                            //     } else {
                            //         tracing::info!("{:#?}", text);
                            //         // if !text.is_empty() {
                            //         if let Ok(res) = serde_json::from_str::<Value>(&text) {
                            //             let _ = self.tx.clone().send(res).await;
                            //         } else {
                            //             tracing::info!("not ok");
                            //         }
                            //         // }
                            //     }
                            // }

                            match text {
                                Message::Text(utf8_bytes) => {
                                    let text = utf8_bytes.to_string();
                                    if !text.is_empty() {
                                        if text != "PING" && text != "PONG" {
                                            tracing::info!(
                                                "Received stream of data on {:#?} >> {:#?}",
                                                self.id.clone(),
                                                text
                                            );
                                            if let Ok(res) = serde_json::from_str::<Value>(&text) {
                                                let _ = self.tx.clone().send(res).await;
                                            } else {
                                                tracing::info!("not ok");
                                            }
                                        }
                                    }
                                }
                                // Message::Binary(bytes) => todo!(),
                                Message::Ping(_) => {
                                    // tracing::info!("Received a server ping");
                                    let _ = tx_task2.send(true).await;
                                }
                                Message::Pong(_) => {
                                    // println!("Received a server pong");
                                    // let res = write
                                    //     .send(Message::Ping(axum::body::Bytes::from_static(
                                    //         b"PING",
                                    //     )))
                                    //     .await;
                                }
                                _ => {} // Message::Pong(bytes) => todo!(),
                                        // Message::Close(close_frame) => todo!(),
                                        // Message::Frame(frame) => todo!(),
                            }
                        }
                        Err(err) => {
                            tracing::info!("Data is not ok, {:#?}", err);
                            tracing::info!("Reconnecting because of sudden error");
                            break;
                        }
                    }
                }
                let _ = tokio::time::sleep(Duration::from_millis(self.timeout)).await;
                tracing::info!("Trying to reconnect after exiting loop");
                self.connect().await;
            }
            Err(err) => {
                tracing::info!("Error connecting to websocket {} - sleeping", err);
                let _ = tokio::time::sleep(Duration::from_millis(self.timeout)).await;
                tracing::info!("Reconnecting because of init connect failure");
                self.connect().await;
            }
        }
    }

    pub async fn _get_text_from_tweet(
        api_key: &str,
        id: &str,
    ) -> Result<TwitterMiniTweet, Box<dyn Error + Send + Sync>> {
        let client = reqwest::Client::new();

        let result = client
            .get(format!("https://twitter-api.axsys.us/v1/data/tweet/{id}"))
            .header("Authorization", api_key)
            .send()
            .await?;

        let text = result.text().await?;
        let response_json = serde_json::from_str::<TwitterMiniTweet>(text.as_str())?;
        Ok(response_json)
    }

    pub async fn add_watched_profile(
        api_key: &str,
        payload: AddWatchedProfilePayload,
    ) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let client = reqwest::Client::new();

        let result = client
            .post("https://twitter-api.axsys.us/v1/watched")
            .json(&json!(payload))
            .header("Authorization", api_key)
            .send()
            .await?;

        let text = result.text().await?;
        let response_json = serde_json::from_str::<Value>(text.as_str())?;
        Ok(response_json)
    }

    pub async fn get_watched_profiles(
        api_key: &str,
    ) -> Result<WatchProfiles, Box<dyn Error + Send + Sync>> {
        let client = reqwest::Client::new();

        let result = client
            .get("https://twitter-api.axsys.us/v1/watched")
            .header("Authorization", api_key)
            .send()
            .await?;

        let text = result.text().await?;
        let response_json = serde_json::from_str::<WatchProfiles>(text.as_str())?;
        Ok(response_json)
    }

    pub async fn delete_watched_profile(
        api_key: &str,
        id: &str,
    ) -> Result<Value, Box<dyn Error + Send + Sync>> {
        let client = reqwest::Client::new();
        let url = format!("https://twitter-api.axsys.us/v1/watched/{id}");
        // tracing::info!("Deleting {url:#?}");
        let result = client
            .delete(url)
            .header("Authorization", api_key)
            .send()
            .await?;

        let text = result.text().await?;
        let response_json = serde_json::from_str::<Value>(text.as_str())?;
        Ok(response_json)
    }
}
