#![allow(unused)]
#![allow(clippy::all)]

use ::models::broadcast::WsBroadcastGeneric;
use api::solana::full_buy;
use api::tasks::start_tasks;
use api::twitter::handle_tweet;
use api::yellowstone::start_yellowstone_service;
use axsys::AxsysClient;
use client::WsClient;
use dotenv::dotenv;
use futures::future::join_all;
use http::start_app_router;
use models::bundle_types::BundleType;
use models::buy::{BuyRequest, BuyRequestOutgoingPayload};
use models::monitor::MonitorTx;
use models::mpsc::{GeyserProcessedTx, GeyserTracker, TaskLogs, TweetLogs, TxStatus};
use models::tasks::TaskWallet;
use models::{
    axsys::TweetEvent,
    mpsc::{MpscLogs, MpscTask},
    state::AppState,
};
use serde_json::Value;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use std::time::Duration;
use std::{collections::HashMap, error::Error, sync::Arc};
use tokio::sync::{mpsc, Mutex, RwLock};
use utils::helpers::get_current_time_ms;
use uuid::Uuid;

pub mod api;
pub mod http;
pub mod models;
pub mod utils;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv().ok();

    let file_appender = tracing_appender::rolling::daily("./logs/", "logger.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_max_level(tracing::Level::INFO)
        .with_thread_names(true)
        .with_thread_ids(true)
        .with_target(false)
        .init();

    let (tx_twitter, mut rx_twitter) = mpsc::channel::<Value>(1000);
    let (tx_task, mut rx_task) = mpsc::channel::<MpscTask>(1000);
    let (tx_geyser_tracker, rx_geyser_tracker) = mpsc::channel::<GeyserTracker>(1000);

    let tx_task1 = tx_task.clone();
    let tx_task2 = tx_task.clone();

    let (tx_logs, mut rx_logs) = mpsc::channel::<MpscLogs>(1000);

    let axsys_external_api_key =
        std::env::var("AXSYS_EXTERNAL_API_KEY").expect("axsys API external key is required");
    let axsys_api_key = std::env::var("AXSYS_API_KEY").expect("axsys API key is required");
    let axsys_api_key_normal =
        std::env::var("AXSYS_API_KEY_NORMAL").expect("axsys API key is required");
    let axsys_ws_endpoint =
        std::env::var("AXSYS_WS_ENDPOINT").expect("axsys ws endpoint is required");
    let ocr_api_endpoint = std::env::var("OCR_API_ENDPOINT").expect("OCR endpoint is required");

    let rpc_endpoint = std::env::var("RPC_ENDPOINT").expect("RPC endpoint is required");
    let geyser_endpoint = std::env::var("GEYSER_ENDPOINT").expect("Geyser endpoint is required");
    let geyser_token = std::env::var("GEYSER_TOKEN").expect("Geyser token is required");
    let jupiter_endpoint = std::env::var("JUPITER_ENDPOINT").expect("Jupiter endpoint is required");
    let jupiter_api_key = std::env::var("JUPITER_API_KEY").expect("Jupiter endpoint is required");
    let nextblock_endpoint =
        std::env::var("NEXTBLOCK_ENDPOINT").expect("Nextblock endpoint is required");
    let nextblock_api_key =
        std::env::var("NEXTBLOCK_API_KEY").expect("Nextblock API key is required");
    let jito_api_endpoint = std::env::var("JITO_API_ENDPOINT").expect("JITO endpoint is required");
    let jito_api_key = std::env::var("JITO_API_KEY").expect("Jito API key is required");

    let server_ip = std::env::var("SERVER_IP").expect("Server ip is required");
    let x_api_key = std::env::var("X_API_KEY").expect("X_API_KEY is required");
    let server_name = std::env::var("SERVER_NAME").expect("Server name is required");

    tracing::info!("Starting solana web app...");

    let mut thread_handles = vec![];

    let shared_state = Arc::new(AppState {
        server_name,
        tx_task,
        ws_clients: Arc::new(RwLock::new(HashMap::new())),
        axsys_external_api_key,
        axsys_ws_endpoint,
        rpc_endpoint,
        geyser_endpoint,
        geyser_token,
        jupiter_endpoint,
        jupiter_api_key,
        nextblock_endpoint,
        nextblock_api_key,
        jito_api_endpoint,
        jito_api_key,
        axsys_api_key,
        axsys_api_key_normal,
        ocr_api_endpoint,
        tx_logs,
        tasks: Arc::new(RwLock::new(vec![])),
        spam_tasks: Arc::new(RwLock::new(vec![])),
        spam_tasks_available: Arc::new(RwLock::new(HashMap::new())),
        tasks_can_retry: Arc::new(RwLock::new(HashMap::new())),
        tweets_detected: Arc::new(RwLock::new(HashMap::new())),
        tweets_bought: Arc::new(RwLock::new(HashMap::new())),
        tweets_images: Arc::new(RwLock::new(HashMap::new())),
        retries: Arc::new(RwLock::new(HashMap::new())),
        tx_geyser_tracker,
    });

    let route_state = shared_state.clone();
    let axsys_state_2 = shared_state.clone();
    let axsys_state_3 = shared_state.clone();
    let spam_tasks_state = shared_state.clone();
    let retries_state = shared_state.clone();
    let logs_state = shared_state.clone();

    thread_handles.push(tokio::task::spawn(async move {
        start_app_router(route_state).await;
    }));

    let (tx_client, rx_client) = mpsc::channel::<String>(1000);
    let (tx_server, mut rx_server) = mpsc::channel::<String>(1000);

    let tx_client1 = tx_client.clone();
    let tx_server1 = tx_server.clone();

    thread_handles.push(tokio::task::spawn(async move {
        let mut client = WsClient::new(
            format!("{}?token={}", server_ip, x_api_key),
            Arc::new(Mutex::new(rx_client)),
            tx_server1,
            5_000,
        );
        let _ = client.connect().await;
    }));

    let orders_state = shared_state.clone();
    thread_handles.push(tokio::task::spawn(async move {
        start_tasks(orders_state, rx_server).await;
    }));

    for i in 0..6 {
        let tx_twitter = tx_twitter.clone();
        let axsys_state_2 = axsys_state_2.clone();
        let axsys_ws_endpoint = axsys_state_2.axsys_ws_endpoint.clone();
        let axsys_api_key: String;
        if i > 2 {
            axsys_api_key = axsys_state_2.axsys_api_key_normal.clone();
        } else {
            axsys_api_key = axsys_state_2.axsys_api_key.clone();
        }
        thread_handles.push(tokio::task::spawn(async move {
            let mut axsys_client = AxsysClient::new(
                format!("axsys_{}", i + 1),
                axsys_ws_endpoint,
                axsys_api_key,
                tx_twitter,
                5_000,
            );
            let _ = axsys_client.connect().await;
        }));
    }

    let geyser_state = shared_state.clone();
    thread_handles.push(tokio::task::spawn(async move {
        let _ = start_yellowstone_service(
            geyser_state.geyser_endpoint.clone(),
            geyser_state.geyser_token.clone(),
            geyser_state.tx_task.clone(),
            rx_geyser_tracker,
        )
        .await;
    }));

    thread_handles.push(tokio::task::spawn(async move {
        while let Some(data) = rx_twitter.recv().await {
            if let Some(current_type) = data.get("type") {
                let current_type = current_type.as_str().unwrap();
                if current_type == "tweet.mini.update" || current_type == "tweet.update" {
                    let tweet = serde_json::from_value::<TweetEvent>(data);
                    match tweet {
                        Ok(tweet) => {
                            let _ = tx_task1.send(MpscTask::Tweet(tweet)).await;
                        }
                        Err(err) => {
                            tracing::info!("Error deserializing tweet {:#?}", err);
                        }
                    }
                }
            }
        }
    }));

    thread_handles.push(tokio::task::spawn(async move {
        loop {
            let spam_tasks = spam_tasks_state.get_spam_tasks().await;
            for spam_task in spam_tasks.iter() {
                let mut buy_request = spam_task.clone();
                buy_request.retry_uuid = Uuid::new_v4().to_string();
                let _ = spam_tasks_state
                    .tx_task
                    .send(MpscTask::Retry(buy_request))
                    .await;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }));

    thread_handles.push(tokio::task::spawn(async move {
        let mut current_slot = 0;
        let mut watched_txs: Vec<MonitorTx> = vec![];
        let mut processed_txs: HashMap<String, GeyserProcessedTx> = HashMap::new();
        let mut tasks_can_retry: HashMap<String, bool> = HashMap::new();
        // let current_tasks: Vec<Task> = axsys_state_3.get_db().get_tasks().await;
        // axsys_state_3.set_tasks(&current_tasks).await;
        // let rpc = RpcClient::new(axsys_state_3.rpc_endpoint.clone());
        // let mut last_slot_fetched = 0;
        // let mut leaders: BTreeMap<u64, Pubkey> = BTreeMap::new();
        // let limit = 20;
        while let Some(ref incoming_event) = rx_task.recv().await {
            match incoming_event {
                MpscTask::Slot(slot) => {
                    current_slot = *slot;
                    let _ = axsys_state_3
                        .tx_logs
                        .send(MpscLogs::Slot(current_slot))
                        .await;
                    // if last_slot_fetched == 0 || current_slot - last_slot_fetched >= limit {
                    //     last_slot_fetched = current_slot;
                    //     let result_leaders = rpc.get_slot_leaders(*slot, limit);
                    //     if let Ok(result_leaders) = result_leaders {
                    //         let mut idx = 0;
                    //         for leader in result_leaders {
                    //             leaders.insert(last_slot_fetched + idx, leader);
                    //             idx += 1;
                    //         }
                    //     }
                    // }
                    // tracing::info!(
                    //     "Current slot: {:#?} - Jito: {} - Nextblock: {}",
                    //     current_slot,
                    //     is_jito(leaders.get(&current_slot).unwrap()),
                    //     is_nextblock(leaders.get(&current_slot).unwrap())
                    // );
                    // println!("{:#?}")
                }
                MpscTask::TxSent(monitored_tx) => {
                    let tx_slot = current_slot;
                    {
                        tracing::info!("Pushing to spam txs....");
                        let mut spam_tasks_available =
                            axsys_state_3.spam_tasks_available.write().await;
                        if let None = spam_tasks_available.get(&monitored_tx.uuid) {
                            spam_tasks_available.insert(monitored_tx.uuid.clone(), true);
                            if let Some(old_buy_request) = &monitored_tx.task {
                                let mut buy_request = old_buy_request.clone();
                                buy_request.retry_uuid = Uuid::new_v4().to_string();
                                axsys_state_3.add_spam_task(&buy_request).await;
                            }
                        }
                        drop(spam_tasks_available);
                    }
                    watched_txs.push(monitored_tx.clone());

                    let text = if monitored_tx.bundle_type.clone() as i32 == BundleType::JITO as i32
                    {
                        format!(
                            "Successfully sent jito bundle at slot {tx_slot} with {}ms",
                            monitored_tx.timelapsed
                        )
                    } else if monitored_tx.bundle_type.clone() as i32
                        == BundleType::NEXTBLOCK as i32
                    {
                        format!(
                            "Successfully sent nextblock bundle at slot {tx_slot} with {}ms",
                            monitored_tx.timelapsed
                        )
                    } else if monitored_tx.bundle_type.clone() as i32
                        == BundleType::BLOXROUTE as i32
                    {
                        format!(
                            "Successfully sent bloxroute bundle at slot {tx_slot} with {}ms",
                            monitored_tx.timelapsed
                        )
                    } else {
                        format!(
                            "Successfully zero slot bloxroute bundle at slot {tx_slot} with {}ms",
                            monitored_tx.timelapsed
                        )
                    };
                    let _ = axsys_state_3
                        .tx_logs
                        .send(MpscLogs::Tx(TaskLogs {
                            logs_type: monitored_tx.logs_type.clone(),
                            tx_hash: Some(monitored_tx.signature.clone()),
                            token: None,
                            bundle_hash: Some(monitored_tx.bundle_hash.clone()),
                            sender: None,
                            text,
                            confirmed: false,
                            status: TxStatus::SENT,
                            timestamp: monitored_tx.timestamp,
                        }))
                        .await;
                }
                MpscTask::TxConfirmation(tx) => {
                    processed_txs.insert(tx.clone().signature, tx.clone());
                    let idx = watched_txs.iter().position(|x| {
                        x.signature == tx.signature.clone() && !x.confirmed && !x.checked
                    });
                    if let Some(idx) = idx {
                        {
                            if let Some(task) = &watched_txs[idx].task {
                                axsys_state_3
                                    .remove_spam_tasks_by_uuid(task.uuid.clone())
                                    .await;
                            }
                        }

                        watched_txs[idx].confirmed = true;
                        watched_txs[idx].checked = true;
                        watched_txs[idx].success = tx.success;

                        let text = if watched_txs[idx].bundle_type.clone() as i32
                            == BundleType::JITO as i32
                        {
                            if tx.success {
                                format!("Successfully confirmed jito bundle at slot {}", tx.slot)
                            } else {
                                format!("Failed jito bundle at slot {}", tx.slot)
                            }
                        } else if watched_txs[idx].bundle_type.clone() as i32
                            == BundleType::NEXTBLOCK as i32
                        {
                            if tx.success {
                                format!(
                                    "Successfully confirmed nextblock bundle at slot {}",
                                    tx.slot
                                )
                            } else {
                                format!("Failed nextblock bundle at slot {}", tx.slot)
                            }
                        } else if watched_txs[idx].bundle_type.clone() as i32
                            == BundleType::BLOXROUTE as i32
                        {
                            if tx.success {
                                format!(
                                    "Successfully confirmed bloxroute bundle at slot {}",
                                    tx.slot
                                )
                            } else {
                                format!("Failed bloxroute bundle at slot {}", tx.slot)
                            }
                        } else {
                            if tx.success {
                                format!(
                                    "Successfully confirmed zero slot bundle at slot {}",
                                    tx.slot
                                )
                            } else {
                                format!("Failed zero slot bundle at slot {}", tx.slot)
                            }
                        };
                        let _ = axsys_state_3
                            .tx_logs
                            .send(MpscLogs::Tx(TaskLogs {
                                logs_type: watched_txs[idx].logs_type.clone(),
                                tx_hash: Some(watched_txs[idx].signature.clone()),
                                token: None,
                                bundle_hash: Some(watched_txs[idx].bundle_hash.clone()),
                                sender: None,
                                text,
                                confirmed: true,
                                status: TxStatus::CONFIRMED,
                                timestamp: get_current_time_ms(),
                            }))
                            .await;

                        if !tx.success {
                            if let Some(task) = &watched_txs[idx].task {
                                if let None = tasks_can_retry.get(&task.retry_uuid) {
                                    tasks_can_retry.insert(task.retry_uuid.clone(), true);

                                    let mut new_retry_task = task.clone();
                                    new_retry_task.retry_uuid = Uuid::new_v4().to_string();
                                    let _ = tx_task2.send(MpscTask::Retry(new_retry_task)).await;
                                }
                            }
                        }
                    }
                }
                MpscTask::Retry(retry_task) => {
                    let mut new_retry_task = retry_task.clone();
                    tracing::info!("Task id to retry: {:#?}", new_retry_task.uuid);
                    let found = watched_txs
                        .iter()
                        .find(|x| x.uuid == *new_retry_task.uuid && x.confirmed && x.success);

                    if let Some(found) = found {
                        tracing::info!("Task confirmed with following details: {:#?}", found);
                    } else {
                        let retries = retries_state.get_retries(new_retry_task.uuid.clone()).await;
                        if let Some(retries) = retries {
                            tracing::info!(
                                "Number of retries so far: {} - should retry {}",
                                retries,
                                retries < new_retry_task.tries
                            );
                            if retries < new_retry_task.tries {
                                retries_state
                                    .add_retries(new_retry_task.uuid.clone(), retries + 1)
                                    .await;
                                new_retry_task.retry_uuid = Uuid::new_v4().to_string();
                                full_buy(retries_state.clone(), new_retry_task.clone()).await;
                            } else {
                                axsys_state_3
                                    .remove_spam_tasks_by_uuid(new_retry_task.uuid)
                                    .await;
                            }
                        } else {
                            // let all_retries = axsys_state_3.get_all_retries().await;
                            tracing::info!("No retries found for {:#?}", new_retry_task.uuid);
                        }
                    }
                }
                MpscTask::Tweet(twitter_mini_tweet) => {
                    tokio::task::spawn(handle_tweet(
                        axsys_state_3.clone(),
                        twitter_mini_tweet.clone(),
                        current_slot.clone(),
                    ));
                }
                _ => {}
            }
        }
    }));

    thread_handles.push(tokio::task::spawn(async move {
        while let Some(ref incoming_event) = rx_logs.recv().await {
            match incoming_event {
                MpscLogs::Tx(task_logs) => {
                    tracing::info!("Received stream from bot {:#?}", incoming_event);
                    let payload: WsBroadcastGeneric<TaskLogs> = WsBroadcastGeneric {
                        server: logs_state.server_name.clone(),
                        r#type: "LOGS".to_string(),
                        data: task_logs.clone(),
                    };
                    let msg_string = serde_json::to_string(&payload).unwrap();
                    let _ = tx_client1.send(msg_string).await;
                }
                MpscLogs::Tweet(tweet_logs) => {
                    tracing::info!("Received stream from bot {:#?}", incoming_event);
                    let payload: WsBroadcastGeneric<TweetLogs> = WsBroadcastGeneric {
                        server: logs_state.server_name.clone(),
                        r#type: "TWEET".to_string(),
                        data: tweet_logs.clone(),
                    };
                    let msg_string = serde_json::to_string(&payload).unwrap();
                    let _ = tx_client1.send(msg_string).await;
                }
                MpscLogs::Slot(slot_number) => {
                    let payload: WsBroadcastGeneric<u64> = WsBroadcastGeneric {
                        server: logs_state.server_name.clone(),
                        r#type: "SLOT_NUMBER".to_string(),
                        data: slot_number.clone(),
                    };
                    let msg_string = serde_json::to_string(&payload).unwrap();
                    let _ = tx_client1.send(msg_string).await;
                }
                _ => {
                    continue;
                }
            }
        }
    }));

    let _join_rs = join_all(thread_handles).await;
    Ok(())
}
