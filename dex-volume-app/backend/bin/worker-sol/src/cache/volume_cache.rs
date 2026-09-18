//! Daily volume tracking cache
//!
//! Manages Redis cache for tracking daily trading volume per project
//! Used for daily volume caps and reporting

use std::error::Error;

use vm_redis::{get_redis_data, insert_redis_data_with_ttl, connect_redis as redis_instance};

type GenericError = Box<dyn Error + Send + Sync>;

// ============================================================================
// Daily Volume Tracking
// ============================================================================

/// Get the current UTC date string (YYYY-MM-DD)
fn get_utc_date_string() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Redis key for today's daily volume — date embedded in key so it auto-resets daily.
fn daily_volume_key(project_id: i32) -> String {
    format!("sol:daily_volume:{}:{}", project_id, get_utc_date_string())
}

/// Get daily volume for a project today. Returns None if no tracking exists for today.
pub async fn get_daily_volume(project_id: i32) -> Result<Option<f64>, GenericError> {
    let key = daily_volume_key(project_id);
    let mut redis = redis_instance().await;
    let value: Option<f64> = redis::cmd("GET")
        .arg(&key)
        .query_async(&mut redis)
        .await?;
    Ok(value)
}

/// Atomically add volume to today's tracking for a project using INCRBYFLOAT.
/// No read-modify-write race — Redis handles the increment atomically.
/// Key expires at end of day (86400s TTL set on first write).
pub async fn add_daily_volume(project_id: i32, volume_usdc: f64) -> Result<f64, GenericError> {
    let key = daily_volume_key(project_id);
    let mut redis = redis_instance().await;

    // INCRBYFLOAT is atomic — no race conditions
    let new_total: f64 = redis::cmd("INCRBYFLOAT")
        .arg(&key)
        .arg(volume_usdc)
        .query_async(&mut redis)
        .await?;

    // Set TTL on first write (86400s = 24h, will auto-expire)
    // EXPIRE is idempotent — safe to call every time
    let _: () = redis::cmd("EXPIRE")
        .arg(&key)
        .arg(86400i64)
        .query_async(&mut redis)
        .await?;

    Ok(new_total)
}

// ============================================================================
// Pending Trade Volume (set on submit, consumed on Geyser confirmation)
// ============================================================================

/// Store the expected USDC value of a trade keyed by tx hash.
/// Called in buy.rs/sell.rs after signing (when the signature is known).
/// TTL of 120s — if the tx doesn't land by then, the entry expires.
pub async fn set_trade_volume_for_tx(
    tx_hash: &str,
    usdc_value: f64,
) -> Result<(), GenericError> {
    let key = format!("sol:tx_trade_volume:{}", tx_hash);
    insert_redis_data_with_ttl(key, usdc_value, 120).await
}

/// Consume the trade USDC volume for a confirmed transaction.
/// Returns the USDC value and deletes the key so it's only counted once.
pub async fn take_trade_volume_for_tx(
    tx_hash: &str,
) -> Result<Option<f64>, GenericError> {
    let key = format!("sol:tx_trade_volume:{}", tx_hash);
    let value = get_redis_data::<f64>(key.clone()).await?;
    if value.is_some() {
        let mut redis = redis_instance().await;
        let _: () = redis::cmd("DEL")
            .arg(&key)
            .query_async(&mut redis)
            .await?;
    }
    Ok(value)
}

/// Get remaining daily volume budget for a project
/// Returns (remaining_usdc, used_usdc, daily_target_usdc)
pub async fn get_daily_volume_remaining(
    project_id: i32,
    daily_target_usdc: f64,
) -> Result<(f64, f64, f64), GenericError> {
    let used = get_daily_volume(project_id).await?.unwrap_or(0.0);
    let remaining = (daily_target_usdc - used).max(0.0);

    Ok((remaining, used, daily_target_usdc))
}
