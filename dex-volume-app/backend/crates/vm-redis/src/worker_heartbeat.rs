use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{hdel_redis, hset_redis};

/// How often the worker heartbeat is refreshed in Redis.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

/// Spawn a loop that writes `workers:{chain}` → `{worker_id}: {unix_ts}`
/// into a Redis hash every 15 seconds. On shutdown, the entry is removed.
///
/// Both worker binaries (sol, evm) have identical heartbeat loops.
/// This extracts that into a shared helper.
pub fn spawn_worker_heartbeat(
    chain: &'static str,
    worker_id: String,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    let key = format!("workers:{chain}");
    tokio::spawn(async move {
        loop {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .to_string();
            let _ = hset_redis(&key, &worker_id, &timestamp).await;
            tokio::select! {
                _ = tokio::time::sleep(HEARTBEAT_INTERVAL) => {}
                _ = shutdown.cancelled() => {
                    let _ = hdel_redis(&key, &worker_id).await;
                    break;
                }
            }
        }
    })
}
