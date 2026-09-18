use vm_data::db::Database;
use vm_data::utils::encryption::decrypt;
use vm_data::models::{
    streams::{StreamInfo, StreamType, TaskStatusInfo},
    task::{ProcessTask, ProcessWalletInfo},
};
use vm_nats::subjects;

pub async fn start_task_req(
    db: &Database,
    nc: &async_nats::Client,
    js: &async_nats::jetstream::Context,
    user_id: i32,
    project_id: i32,
) -> Option<bool> {
    let project = db.get_project(user_id, project_id).await.unwrap_or(None)?;

    // Idempotency guard: if already running and heartbeat is alive, skip the start.
    // This prevents the sweep from creating duplicate chains when the worker is
    // alive but sleeping between iterations.
    if project.status == "running"
        && vm_redis::task_heartbeat::is_alive("sol", project_id).await.unwrap_or(false) {
            tracing::info!("[RPC] start_task_req skipped for project {} (already running, heartbeat alive)", project_id);
            return Some(true);
        }

    let all_wallets = db.get_project_wallets(user_id, project.id).await.unwrap_or_default();
    let all_wallets: Vec<_> = all_wallets.iter().filter(|x| !x.is_main).cloned().collect();

    let mut wallets = vec![];
    for wallet in all_wallets {
        if let Ok(decrypted) = decrypt(wallet.pk) {
            let kp = vm_solana::utils::helpers::str_to_pk(decrypted);
            if let Ok(kp) = kp {
                wallets.push(ProcessWalletInfo {
                    private_key: kp.to_base58_string(),
                    address: wallet.address,
                    wallet_id: wallet.id,
                });
            }
        }
    }

    if wallets.is_empty() {
        return Some(false);
    }

    let task = ProcessTask {
        project,
        wallets,
        last_active: 0,
    };

    // Update DB status BEFORE publishing to JetStream so the consumer's
    // status guard sees "running" by the time it processes the message.
    let _ = db.update_project_status(project_id, "running").await;

    // Write task heartbeat (30s TTL) — orphan sweep in api-server detects expired keys
    let _ = vm_redis::task_heartbeat::refresh_heartbeat("sol", project_id).await;

    let serialized = serde_json::to_vec(&StreamInfo {
        stream_type: StreamType::StartTask(task.clone()),
        user_id: task.project.user_id,
    })
    .unwrap_or_default();

    let _ = js
        .publish(subjects::cmd::sol::TASK_START, serialized.into())
        .await;

    // Broadcast status update to WebSocket clients
    let status_bytes = serde_json::to_vec(&StreamInfo {
        user_id: task.project.user_id,
        stream_type: StreamType::TaskStatusUpdate(TaskStatusInfo {
            project_id,
            status: "running".to_string(),
        }),
    })
    .unwrap_or_default();
    let _ = nc
        .publish(subjects::events::sol::TASK_STATUS, status_bytes.into())
        .await;

    Some(true)
}
