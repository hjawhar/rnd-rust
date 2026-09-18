use async_recursion::async_recursion;
use futures_util::{SinkExt, StreamExt};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    Mutex,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, protocol::Message, Utf8Bytes},
};

pub fn get_current_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

#[derive(Clone)]
pub struct WsClient {
    pub timeout: u64,
    pub rx_client: Arc<Mutex<Receiver<String>>>,
    pub tx_server: Sender<String>,
    pub api_endpoint: String,
}

impl WsClient {
    pub fn new(
        api_endpoint: String,
        rx_client: Arc<Mutex<Receiver<String>>>,
        tx_server: Sender<String>,
        timeout: u64,
    ) -> Self {
        Self {
            api_endpoint,
            timeout,
            rx_client,
            tx_server,
        }
    }

    #[async_recursion]
    pub async fn connect(&mut self) {
        let req = self.api_endpoint.clone().into_client_request().unwrap();
        match connect_async(req).await {
            Ok((stream, _)) => {
                tracing::info!("Successfully connected to ws central server");
                let (mut write, mut read) = stream.split();
                let (tx_task, mut rx_task) = mpsc::channel::<String>(1000);
                let tx_task1 = tx_task.clone();
                let tx_task2 = tx_task.clone();
                let tx_task3 = tx_task.clone();
                let s1 = self.clone();
                // let s2 = self.clone();
                tokio::task::spawn(async move {
                    while let Some(msg) = rx_task.recv().await {
                        let data = Message::Text(Utf8Bytes::from(msg));
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
                });

                tokio::task::spawn(async move {
                    loop {
                        let _ = tx_task1.send("PING".to_string()).await;
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                });

                // Receive data from client and forward to server
                tokio::task::spawn(async move {
                    let mut rx = s1.rx_client.lock().await;
                    while let Some(data) = rx.recv().await {
                        let _ = tx_task2.send(data).await;
                    }
                });

                while let Some(data) = read.next().await {
                    match data {
                        Ok(text) => match text {
                            Message::Text(utf8_bytes) => {
                                // Receive data from server and forward to client
                                let _text = utf8_bytes.to_string();
                                let _ = s1.tx_server.send(_text).await;
                            }
                            Message::Ping(_) => {
                                let _ = tx_task3.send("PONG".to_string()).await;
                            }
                            Message::Pong(_) => {}
                            _ => {}
                        },
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
}
