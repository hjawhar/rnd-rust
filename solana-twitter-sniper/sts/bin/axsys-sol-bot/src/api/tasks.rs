use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc::Receiver;
use uuid::Uuid;

use crate::models::{
    buy::{BuyRequest, BuyRequestOutgoingPayload},
    mpsc::GeyserTracker,
    state::AppState,
    tasks::TaskWallet,
};

use super::solana::full_buy;

pub async fn start_tasks(orders_state: Arc<AppState>, mut rx_server: Receiver<String>) {
    while let Some(ref incoming_event) = rx_server.recv().await {
        let parsed = serde_json::from_str::<Value>(incoming_event);
        if let Ok(parsed) = parsed {
            if parsed.get("type").is_some() && parsed.get("data").is_some() {
                if let Some(ws_type) = parsed.get("type").and_then(|x| x.as_str()) {
                    if ws_type == "BUY_REQUEST" {
                        if let Ok(buy_req) = serde_json::from_value::<BuyRequestOutgoingPayload>(
                            parsed.get("data").unwrap().clone(),
                        ) {
                            let final_payload = BuyRequest {
                                retry_uuid: Uuid::new_v4().to_string(),
                                uuid: Uuid::new_v4().to_string(),
                                private_key: buy_req.private_key,
                                nonce_account_address: buy_req.nonce_account_address,
                                mint: buy_req.mint_address,
                                value: buy_req.value,
                                tip: buy_req.tip,
                                slippage: buy_req.slippage,
                                tries: buy_req.tries,
                                frontrunning_protection: buy_req.frontrunning_protection,
                                enable_alerts: buy_req.enable_alerts,
                                servers: Some(buy_req.servers),
                                block_leaders: Some(buy_req.block_leaders),
                                selected_pool: buy_req.selected_pool,
                            };
                            full_buy(orders_state.clone(), final_payload).await;
                        }
                    } else if ws_type == "SET_TASKS" {
                        if let Ok(tasks) = serde_json::from_value::<Vec<TaskWallet>>(
                            parsed.get("data").unwrap().clone(),
                        ) {
                            orders_state.set_tasks(&tasks).await;
                            for task in &tasks {
                                if let Some(pub_key) = &task.public_key {
                                    orders_state
                                        .tx_geyser_tracker
                                        .send(GeyserTracker::Start(pub_key.clone()))
                                        .await;
                                }
                            }
                        }
                    } else if ws_type == "ADD_TASK" {
                        if let Ok(task) = serde_json::from_value::<TaskWallet>(
                            parsed.get("data").unwrap().clone(),
                        ) {
                            orders_state.add_task(&task).await;
                            if let Some(pub_key) = &task.public_key {
                                orders_state
                                    .tx_geyser_tracker
                                    .send(GeyserTracker::Start(pub_key.clone()))
                                    .await;
                            }
                        }
                    } else if ws_type == "UPDATE_TASK" {
                        if let Ok(task) = serde_json::from_value::<TaskWallet>(
                            parsed.get("data").unwrap().clone(),
                        ) {
                            let mut old_pub_key: Option<String> = None;
                            let mut new_pub_key: Option<String> = None;
                            let current_task = orders_state.get_task(task.id).await;
                            if let Some(current_task) = &current_task {
                                if let Some(pub_key) = &current_task.public_key {
                                    old_pub_key = Some(pub_key.clone());
                                }
                            }

                            if let Some(pub_key) = &task.public_key {
                                new_pub_key = Some(pub_key.clone());
                            }

                            if old_pub_key.is_none() && new_pub_key.is_some() {
                                orders_state
                                    .tx_geyser_tracker
                                    .send(GeyserTracker::Start(new_pub_key.clone().unwrap()))
                                    .await;
                            } else if old_pub_key.is_none() && new_pub_key.is_some() {
                                // do nothing
                            } else if old_pub_key.is_some() && new_pub_key.is_some() {
                                if old_pub_key.eq(&new_pub_key) {
                                    // do nothing
                                } else {
                                    let existing_old_pub_key = old_pub_key.clone().unwrap();
                                    let existing_new_pub_key = new_pub_key.clone().unwrap();

                                    let tasks = orders_state
                                        .get_tasks_with_pub_key(existing_old_pub_key.clone())
                                        .await;
                                    if tasks.len() > 1 {
                                        // do not delete any subscription with same wallet
                                    } else {
                                        orders_state
                                            .tx_geyser_tracker
                                            .send(GeyserTracker::Stop(existing_old_pub_key.clone()))
                                            .await;
                                    }

                                    let tasks = orders_state
                                        .get_tasks_with_pub_key(existing_new_pub_key.clone())
                                        .await;
                                    if tasks.len() == 0 {
                                        orders_state
                                            .tx_geyser_tracker
                                            .send(GeyserTracker::Start(
                                                existing_new_pub_key.clone(),
                                            ))
                                            .await;
                                    }
                                }
                            }

                            orders_state.update_task(&task).await;
                        }
                    } else if ws_type == "DELETE_TASK" {
                        if let Ok(task_idx) =
                            serde_json::from_value::<i32>(parsed.get("data").unwrap().clone())
                        {
                            let current_task = orders_state.get_task(task_idx).await;
                            if let Some(current_task) = &current_task {
                                if let Some(pub_key) = &current_task.public_key {
                                    let tasks =
                                        orders_state.get_tasks_with_pub_key(pub_key.clone()).await;
                                    if tasks.len() == 1 {
                                        orders_state
                                            .tx_geyser_tracker
                                            .send(GeyserTracker::Stop(pub_key.clone()))
                                            .await;
                                    }
                                }
                            }
                            orders_state.remove_task(&task_idx).await;
                        }
                    }
                }
            }
        }
    }
}
