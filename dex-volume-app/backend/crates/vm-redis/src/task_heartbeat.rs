use redis::AsyncCommands;

use crate::{get_conn, GenericError};

/// TTL for heartbeat keys. A task is considered dead if its heartbeat
/// is not refreshed within this window.
pub const HEARTBEAT_TTL_SECS: u64 = 30;

/// How often a live worker should call `refresh_heartbeat`.
/// Must be strictly less than `HEARTBEAT_TTL_SECS` to avoid false expirations.
pub const HEARTBEAT_REFRESH_INTERVAL_SECS: u64 = 10;

fn heartbeat_key(chain: &str, project_id: i32) -> String {
    format!("task:hb:{chain}:{project_id}")
}

/// Mark a task as actively owned by setting its heartbeat key with a TTL.
///
/// Workers call this on a timer (`HEARTBEAT_REFRESH_INTERVAL_SECS`) to prove
/// they are still processing the task. If the key expires, other workers may
/// reclaim the task.
pub async fn refresh_heartbeat(chain: &str, project_id: i32) -> Result<(), GenericError> {
    let mut conn = get_conn().await;
    conn.set_ex::<_, _, ()>(heartbeat_key(chain, project_id), "active", HEARTBEAT_TTL_SECS)
        .await?;
    Ok(())
}

/// Remove the heartbeat key, signalling that the task is no longer owned.
///
/// Called on graceful shutdown or when a worker finishes processing a task.
pub async fn clear_heartbeat(chain: &str, project_id: i32) -> Result<(), GenericError> {
    let mut conn = get_conn().await;
    conn.del::<_, ()>(heartbeat_key(chain, project_id)).await?;
    Ok(())
}

/// Check whether a task's heartbeat is still present (i.e. a worker owns it).
pub async fn is_alive(chain: &str, project_id: i32) -> Result<bool, GenericError> {
    let mut conn = get_conn().await;
    let exists: bool = conn.exists(heartbeat_key(chain, project_id)).await?;
    Ok(exists)
}
