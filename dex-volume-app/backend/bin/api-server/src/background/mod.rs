mod price_refresh;
mod status_broadcast;
mod orphan_sweep;

pub use price_refresh::spawn_price_refresh;
pub use status_broadcast::spawn_status_broadcast;
pub use orphan_sweep::spawn_orphan_sweep;

/// Acquire a distributed Redis lock (SET NX EX).
/// Returns `true` if this instance won the lock, `false` if another instance holds it.
pub(crate) async fn try_acquire_lock(key: &str, ttl_secs: u64) -> bool {
    let mut conn = vm_redis::get_conn().await;
    let result: Option<String> = redis::cmd("SET")
        .arg(key)
        .arg("locked")
        .arg("NX")
        .arg("EX")
        .arg(ttl_secs)
        .query_async(&mut conn)
        .await
        .unwrap_or(None);
    result.is_some()
}
