use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use vm_data::models::streams::{StreamInfo, StreamType};
use vm_nats::subjects;
use crate::models::state::AppState;
use crate::routes::is_evm_network;
use super::try_acquire_lock;

pub fn spawn_orphan_sweep(state: Arc<AppState>, shutdown: CancellationToken) {
    tokio::task::spawn(async move {
        // Initial delay to let workers start up and write heartbeats
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            // Auto-lock projects whose lock_at time has passed
            // Acquire distributed lock so only one api-server runs this sweep
            let auto_lock_acquired = try_acquire_lock("api:auto_lock_sweep", 60).await;

            if auto_lock_acquired
                && let Ok(due) = state.get_db().get_projects_due_for_lock().await {
                    for (project_id, user_id, _network, status) in due {
                        tracing::info!("[SWEEP] Auto-locking project {} (scheduled lock_at reached)", project_id);
                        if let Ok(Some((net, _))) = state.get_db().set_project_locked(project_id, true).await
                            && status == "running" {
                                let subject = if is_evm_network(&net) {
                                    subjects::rpc::evm::PROJECT_STOP_TASK
                                } else {
                                    subjects::rpc::sol::PROJECT_STOP_TASK
                                };
                                let payload = serde_json::to_vec(&StreamInfo {
                                    user_id,
                                    stream_type: StreamType::RequestStopTask(project_id),
                                }).unwrap_or_default();
                                let _ = vm_nats::request_with_timeout(
                                    &state.nats_client,
                                    subject,
                                    payload,
                                    Duration::from_secs(15),
                                ).await;
                            }
                    }
                }

            if let Ok(running) = state.get_db().get_running_projects().await {
                for (project_id, user_id, network) in running {
                    let chain = if is_evm_network(&network) {
                        "evm"
                    } else {
                        "sol"
                    };

                    // Check if heartbeat is alive via the centralized module
                    let alive = vm_redis::task_heartbeat::is_alive(chain, project_id)
                        .await
                        .unwrap_or(true); // default to alive on Redis error

                    if !alive {
                        // Check ownership to log which worker died
                        if let Ok(Some(owner)) = vm_redis::task_ownership::get_owner(chain, project_id).await {
                            tracing::warn!(
                                "[SWEEP] Orphan task detected: project {} ({}) — worker {} (gen {}) died, releasing ownership",
                                project_id,
                                network,
                                owner.worker_id,
                                owner.generation,
                            );
                            if let Err(e) = vm_redis::task_ownership::force_release(chain, project_id).await {
                                tracing::error!(
                                    "[SWEEP] Failed to force-release ownership for project {}: {}",
                                    project_id,
                                    e,
                                );
                            }
                        } else {
                            tracing::warn!(
                                "[SWEEP] Orphan task detected: project {} ({}) — no owner recorded",
                                project_id,
                                network,
                            );
                        }

                        // Acquire restart lock (60s TTL, SET NX) to prevent duplicate restarts
                        let lock_key = format!("task:restart_lock:{}", project_id);
                        let lock_acquired = try_acquire_lock(&lock_key, 60).await;

                        if lock_acquired {
                            let subject = if chain == "evm" {
                                subjects::rpc::evm::PROJECT_START_TASK
                            } else {
                                subjects::rpc::sol::PROJECT_START_TASK
                            };

                            let request = StreamType::RequestStartTask(project_id);
                            let payload = serde_json::to_vec(&StreamInfo {
                                user_id,
                                stream_type: request,
                            })
                            .unwrap_or_default();

                            match vm_nats::request_with_timeout(
                                &state.nats_client,
                                subject,
                                payload,
                                Duration::from_secs(15),
                            )
                            .await
                            {
                                Ok(_) => {
                                    tracing::info!(
                                        "[SWEEP] Restart request sent for project {}",
                                        project_id,
                                    );
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "[SWEEP] Failed to restart project {}: {}",
                                        project_id,
                                        e,
                                    );
                                }
                            }
                        }
                    }
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                _ = shutdown.cancelled() => break,
            }
        }
    });
}
