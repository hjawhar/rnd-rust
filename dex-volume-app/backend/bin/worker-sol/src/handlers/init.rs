use crate::state::get_state;
use vm_data::db::Database;
use vm_data::models::streams::StreamType;
use vm_nats::subjects;
use std::time::Duration;

/// Self-initialize worker by requesting init data from api-server and recovering
/// any orphaned task ownership from a prior crash. Cache population is deferred
/// to TASK_START — only global config (CLMM) is fetched here.
/// Non-fatal on failure — worker can still receive runtime commands.
pub async fn self_initialize(nc: &async_nats::Client, _db: &Database, worker_id: &str) {
    let state = get_state();
    tracing::info!("[{}][INIT] Starting self-initialization...", worker_id);

    // Recover ownership from a prior crash: release any projects this worker
    // previously owned whose heartbeat has expired.
    match vm_redis::task_ownership::get_owned_projects("sol", worker_id).await {
        Ok(project_ids) => {
            for pid in &project_ids {
                match vm_redis::task_heartbeat::is_alive("sol", *pid).await {
                    Ok(false) => {
                        if let Err(e) =
                            vm_redis::task_ownership::force_release("sol", *pid).await
                        {
                            tracing::warn!(
                                "[{}][INIT] Failed to force-release project {}: {}",
                                worker_id, pid, e
                            );
                        } else {
                            tracing::info!(
                                "[{}][INIT] Released orphaned project {} (heartbeat dead)",
                                worker_id, pid
                            );
                        }
                    }
                    Ok(true) => {
                        // Heartbeat still alive — should not happen after a restart, but
                        // leave it alone; the TTL will expire naturally.
                        tracing::warn!(
                            "[{}][INIT] Project {} still has live heartbeat after restart",
                            worker_id, pid
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[{}][INIT] Failed to check heartbeat for project {}: {}",
                            worker_id, pid, e
                        );
                    }
                }
            }
            if !project_ids.is_empty() {
                tracing::info!(
                    "[{}][INIT] Ownership recovery complete, checked {} project(s)",
                    worker_id, project_ids.len()
                );
            }
        }
        Err(e) => {
            tracing::warn!("[{}][INIT] Failed to query owned projects: {}", worker_id, e);
        }
    }

    // Fetch CLMM configs — global config needed for fetch_pair() calls later
    let _ = state.fetch_clmm_configs().await;
    tracing::info!("[{}][INIT] CLMM configs fetched", worker_id);

    let request_bytes = serde_json::to_vec(&StreamType::RequestInitData).unwrap_or_default();

    let response = vm_nats::request_with_timeout(
        nc,
        subjects::rpc::sol::INIT_DATA,
        request_bytes,
        Duration::from_secs(30),
    )
    .await;

    let msg = match response {
        Ok(msg) => msg,
        Err(e) => {
            tracing::error!("[{}][INIT] Failed to request init data (non-fatal): {}", worker_id, e);
            return;
        }
    };

    let stream_type: StreamType = match serde_json::from_slice(&msg.payload) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("[{}][INIT] Failed to deserialize init data (non-fatal): {}", worker_id, e);
            return;
        }
    };

    if let StreamType::ResponseInitData(payload) = stream_type {
        // Cache population (track_token, track_address, wallets, Geyser subscriptions)
        // is deferred to TASK_START handlers — each project's state is populated when
        // ownership is claimed. At init time we own nothing yet.
        tracing::info!(
            "[{}][INIT] Self-initialization complete: {} projects, {} wallets available (cache deferred to TASK_START)",
            worker_id, payload.projects.len(), payload.wallets_projects.len()
        );
    } else {
        tracing::error!("[{}][INIT] Unexpected response type (non-fatal)", worker_id);
    }
}
