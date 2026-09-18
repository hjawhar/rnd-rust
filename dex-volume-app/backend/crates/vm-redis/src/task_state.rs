use chrono::Utc;
use redis::AsyncCommands;

use crate::{get_conn, GenericError};

/// Maximum consecutive failures before auto-stop.
pub const MAX_FAILURES: i32 = 15;
/// Daily volume key TTL in seconds (24 hours).
pub const DAILY_VOLUME_TTL_SECS: u64 = 86400;

// ---------------------------------------------------------------------------
// Key helpers
// ---------------------------------------------------------------------------

fn failures_key(chain: &str, project_id: i32) -> String {
    format!("{chain}:task_failures:{project_id}")
}

fn daily_volume_key(chain: &str, project_id: i32) -> String {
    let today = Utc::now().format("%Y-%m-%d");
    format!("{chain}:daily_volume:{project_id}:{today}")
}

fn price_key(chain: &str) -> String {
    format!("{chain}:price")
}

// ---------------------------------------------------------------------------
// Failure Tracking
// ---------------------------------------------------------------------------

/// Get the current consecutive failure count for a task.
/// Returns 0 if no failures have been recorded.
pub async fn get_task_failures(chain: &str, project_id: i32) -> Result<i32, GenericError> {
    let mut conn = get_conn().await;
    let key = failures_key(chain, project_id);
    let val: Option<String> = conn.get(&key).await?;
    match val {
        Some(s) => Ok(s.parse::<i32>().unwrap_or(0)),
        None => Ok(0),
    }
}

/// Atomically increment the failure count and return the new value.
pub async fn increment_task_failures(chain: &str, project_id: i32) -> Result<i32, GenericError> {
    let mut conn = get_conn().await;
    let key = failures_key(chain, project_id);
    let new_val: i32 = conn.incr(&key, 1i32).await?;
    Ok(new_val)
}

/// Reset (delete) the failure counter for a task.
pub async fn reset_task_failures(chain: &str, project_id: i32) -> Result<(), GenericError> {
    let mut conn = get_conn().await;
    let key = failures_key(chain, project_id);
    redis::cmd("DEL")
        .arg(&key)
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Daily Volume Tracking
// ---------------------------------------------------------------------------

/// Add to today's accumulated volume for a project and return the new total.
/// The key auto-expires after 24 hours.
pub async fn add_daily_volume(
    chain: &str,
    project_id: i32,
    amount_usdc: f64,
) -> Result<f64, GenericError> {
    let mut conn = get_conn().await;
    let key = daily_volume_key(chain, project_id);
    let new_total: f64 = redis::cmd("INCRBYFLOAT")
        .arg(&key)
        .arg(amount_usdc)
        .query_async(&mut conn)
        .await?;
    conn.expire::<_, bool>(&key, DAILY_VOLUME_TTL_SECS as i64).await?;
    Ok(new_total)
}

/// Get the current daily volume for a project. Returns 0.0 if no volume recorded today.
pub async fn get_daily_volume(chain: &str, project_id: i32) -> Result<f64, GenericError> {
    let mut conn = get_conn().await;
    let key = daily_volume_key(chain, project_id);
    let val: Option<String> = conn.get(&key).await?;
    match val {
        Some(s) => Ok(s.parse::<f64>().unwrap_or(0.0)),
        None => Ok(0.0),
    }
}

/// Get remaining volume until `daily_target` is reached. Never negative.
pub async fn get_daily_volume_remaining(
    chain: &str,
    project_id: i32,
    daily_target: f64,
) -> Result<f64, GenericError> {
    let current = get_daily_volume(chain, project_id).await?;
    Ok((daily_target - current).max(0.0))
}

// ---------------------------------------------------------------------------
// Native Price Cache
// ---------------------------------------------------------------------------

/// Cache the native token price for a chain.
pub async fn set_native_price(chain: &str, price: f64) -> Result<(), GenericError> {
    let mut conn = get_conn().await;
    let key = price_key(chain);
    let data = serde_json::to_string(&price)?;
    conn.set::<_, _, ()>(&key, data).await?;
    Ok(())
}

/// Get the cached native token price for a chain, if present.
pub async fn get_native_price(chain: &str) -> Result<Option<f64>, GenericError> {
    let mut conn = get_conn().await;
    let key = price_key(chain);
    let val: Option<String> = conn.get(&key).await?;
    match val {
        Some(s) => {
            let price: f64 = serde_json::from_str(&s)?;
            Ok(Some(price))
        }
        None => Ok(None),
    }
}
