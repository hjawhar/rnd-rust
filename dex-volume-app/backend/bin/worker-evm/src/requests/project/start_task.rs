use vm_data::db::Database;
use vm_data::models::{
    streams::{StreamInfo, StreamType, TaskStatusInfo},
    task::{EvmProcessTask, ProcessWalletInfo},
};
use vm_data::utils::encryption::decrypt;
use vm_nats::subjects;

pub async fn start_task_req(
    db: &Database,
    nc: &async_nats::Client,
    user_id: i32,
    project_id: i32,
) -> Option<bool> {
    let project = db.get_project(user_id, project_id).await.ok().flatten()?;

    // Idempotency guard: if already running and heartbeat is alive, skip the start.
    // This prevents the sweep from creating duplicate loops when the worker is
    // alive but sleeping between iterations.
    if project.status == "running"
        && vm_redis::task_heartbeat::is_alive("evm", project_id).await.unwrap_or(false) {
            tracing::info!("[EVM-RPC] start_task_req skipped for project {} (already running, heartbeat alive)", project_id);
            return Some(true);
        }

    let all_wallets = db.get_project_wallets(user_id, project.id).await.unwrap_or_default();
    let all_wallets: Vec<_> = all_wallets.iter().filter(|x| !x.is_main).cloned().collect();

    let mut wallets = vec![];
    for wallet in all_wallets {
        if let Ok(decrypted) = decrypt(wallet.pk) {
            // For EVM, the decrypted PK is a hex string — validate it
            if let Ok(_signer) = vm_evm::helpers::str_to_pk(&decrypted) {
                wallets.push(ProcessWalletInfo {
                    private_key: decrypted,
                    address: wallet.address,
                    wallet_id: wallet.id,
                });
            }
        }
    }

    if wallets.is_empty() {
        return Some(false);
    }

    let task = EvmProcessTask {
        project,
        wallets,
        last_active: 0,
    };

    // Update DB status BEFORE publishing to JetStream so the consumer's
    // status guard sees "running" by the time it processes the message.
    let _ = db.update_project_status(project_id, "running").await;

    // Write task heartbeat (30s TTL) — orphan sweep in api-server detects expired keys
    let _ = vm_redis::task_heartbeat::refresh_heartbeat("evm", project_id).await;

    let serialized = serde_json::to_vec(&StreamInfo {
        stream_type: StreamType::EvmStartTask(task.clone()),
        user_id: task.project.user_id,
    }).unwrap_or_default();

    let _ = nc
        .publish(subjects::cmd::evm::TASK_START, serialized.into())
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
        .publish(subjects::events::evm::TASK_STATUS, status_bytes.into())
        .await;

    Some(true)
}
