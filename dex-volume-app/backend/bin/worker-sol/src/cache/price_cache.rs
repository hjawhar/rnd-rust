//! Price caching — Redis-backed (cross-service).
//!
//! SOL price and tx prices stay in Redis (api-server reads them).
//! Token price calculation lives in AppState::get_token_price().

use std::error::Error;

use vm_redis::{get_redis_data, insert_redis_data, insert_redis_data_with_ttl};

type GenericError = Box<dyn Error + Send + Sync>;

// ============================================================================
// SOL Price Cache (stays in Redis — api-server reads it)
// ============================================================================

pub async fn add_sol_price(sol_price: f64) -> Result<Option<f64>, GenericError> {
    insert_redis_data("sol:price".to_string(), sol_price).await?;
    Ok(Some(sol_price))
}

pub async fn get_sol_price() -> Result<Option<f64>, GenericError> {
    get_redis_data::<f64>("sol:price".to_string()).await
}

// ============================================================================
// Transaction Price Tracking (stays in Redis — short-lived, cross-handler)
// ============================================================================

pub async fn add_tx_sol_price_at_time(tx_hash: String, price: f64) -> Result<(), GenericError> {
    let key = format!("sol:tx_price:{}", tx_hash);
    insert_redis_data_with_ttl(key, price, 120).await
}

pub async fn get_tx_sol_price_at_time(tx_hash: String) -> Result<Option<f64>, GenericError> {
    let key = format!("sol:tx_price:{}", tx_hash);
    get_redis_data::<f64>(key).await
}
