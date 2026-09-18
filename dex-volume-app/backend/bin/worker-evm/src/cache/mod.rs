//! Cache module - Domain-specific caching for worker-evm (generic ops in vm-redis)
//!
//! All keys are prefixed with `evm:` to avoid collisions with worker-sol.
//! Uses Redis Hashes for shared state (wallets, tracked addresses) to avoid
//! read-modify-write race conditions when multiple workers write concurrently.

use vm_data::models::task::EvmProcessTask;
use vm_data::models::wallet::StoredWallet;
use vm_evm::models::token::CustomPool;

// Re-export generic Redis operations from vm-redis
pub use vm_redis::{
    connect_redis, get_redis_data, insert_redis_data, insert_redis_data_with_ttl,
    hset_redis, hget_redis, hdel_redis, hgetall_redis, hexists_redis,
    hset_redis_typed, hget_redis_typed,
    GenericError,
};

/// Get the singleton Redis connection (delegates to vm-redis).
async fn get_connection() -> redis::aio::MultiplexedConnection {
    vm_redis::get_conn().await
}

// ============================================================================
// ETH Price Cache — delegates to shared task_state module
// ============================================================================

pub async fn get_eth_price() -> Result<Option<f64>, GenericError> {
    vm_redis::task_state::get_native_price("evm").await
}

pub async fn set_eth_price(price: f64) -> Result<(), GenericError> {
    vm_redis::task_state::set_native_price("evm", price).await
}

// ============================================================================
// Pool Cache — per-project best pool, set by volume_maker at task start
// ============================================================================

const POOLS_HASH: &str = "h:evm:pools";

pub async fn get_pool(project_id: i32) -> Result<Option<CustomPool>, GenericError> {
    hget_redis_typed::<CustomPool>(POOLS_HASH, &project_id.to_string()).await
}

pub async fn set_pool(project_id: i32, pool: &CustomPool) -> Result<(), GenericError> {
    hset_redis_typed(POOLS_HASH, &project_id.to_string(), pool).await
}

pub async fn remove_pool(project_id: i32) -> Result<(), GenericError> {
    hdel_redis(POOLS_HASH, &project_id.to_string()).await
}

// ============================================================================
// V4 Pool Key Params Cache — Redis Hash "h:evm:v4_pool_params"
// ============================================================================

const V4_POOL_PARAMS_HASH: &str = "h:evm:v4_pool_params";

pub async fn get_v4_pool_key_params(
    project_id: i32,
) -> Result<Option<vm_evm::simulation::market_impact::PoolKeyParams>, GenericError> {
    hget_redis_typed(V4_POOL_PARAMS_HASH, &project_id.to_string()).await
}

pub async fn set_v4_pool_key_params(
    project_id: i32,
    params: &vm_evm::simulation::market_impact::PoolKeyParams,
) -> Result<(), GenericError> {
    hset_redis_typed(V4_POOL_PARAMS_HASH, &project_id.to_string(), params).await
}

pub async fn remove_v4_pool_key_params(project_id: i32) -> Result<(), GenericError> {
    hdel_redis(V4_POOL_PARAMS_HASH, &project_id.to_string()).await
}

// ============================================================================
// Task Cache — Redis Hash "h:evm:tasks" (no read-modify-write)
// ============================================================================

const TASKS_HASH: &str = "h:evm:tasks";

pub async fn get_task(project_id: i32) -> Result<Option<EvmProcessTask>, GenericError> {
    hget_redis_typed::<EvmProcessTask>(TASKS_HASH, &project_id.to_string()).await
}

pub async fn add_task(task: EvmProcessTask) -> Result<(), GenericError> {
    hset_redis_typed(TASKS_HASH, &task.project.id.to_string(), &task).await
}

pub async fn remove_task(project_id: i32) -> Result<(), GenericError> {
    hdel_redis(TASKS_HASH, &project_id.to_string()).await
}

pub async fn get_all_tasks() -> Result<Vec<EvmProcessTask>, GenericError> {
    let raw = hgetall_redis(TASKS_HASH).await?;
    let mut tasks = Vec::with_capacity(raw.len());
    for val in raw.values() {
        if let Ok(task) = serde_json::from_str::<EvmProcessTask>(val) {
            tasks.push(task);
        }
    }
    Ok(tasks)
}

// ============================================================================
// Task Failure Tracking — delegates to shared task_state module
// ============================================================================

pub async fn get_task_failures(project_id: i32) -> Result<i32, GenericError> {
    vm_redis::task_state::get_task_failures("evm", project_id).await
}

pub async fn increment_task_failures(project_id: i32) -> Result<i32, GenericError> {
    vm_redis::task_state::increment_task_failures("evm", project_id).await
}

pub async fn reset_task_failures(project_id: i32) -> Result<(), GenericError> {
    vm_redis::task_state::reset_task_failures("evm", project_id).await
}

// ============================================================================
// Task Generation — delegates to shared task_ownership module
// The generation counter is embedded in the ownership record.
// ============================================================================

/// Claim ownership (or bump generation if already owned by this worker).
/// Returns the new generation number.
pub async fn claim_task_generation(project_id: i32) -> Result<u64, GenericError> {
    let worker_id = crate::get_worker_id();
    match vm_redis::task_ownership::claim_task("evm", project_id, worker_id).await? {
        Some(ownership) => Ok(ownership.generation),
        None => Err("task already owned by another worker".into()),
    }
}

/// Get the current generation for a project (0 if unclaimed).
pub async fn get_task_generation(project_id: i32) -> Result<u64, GenericError> {
    match vm_redis::task_ownership::get_owner("evm", project_id).await? {
        Some(ownership) => Ok(ownership.generation),
        None => Ok(0),
    }
}

/// Release ownership of a project.
pub async fn release_task_ownership(project_id: i32) -> Result<(), GenericError> {
    let worker_id = crate::get_worker_id();
    let _ = vm_redis::task_ownership::release_task("evm", project_id, worker_id).await?;
    Ok(())
}

// ============================================================================
// Wallet Cache — Redis Hash "h:evm:wallets"
// ============================================================================

const WALLETS_HASH: &str = "h:evm:wallets";

pub async fn add_wallet(stored: StoredWallet) -> Result<(), GenericError> {
    hset_redis_typed(WALLETS_HASH, &stored.wallet.id.to_string(), &stored).await
}

pub async fn get_wallet(wallet_id: i32) -> Result<Option<StoredWallet>, GenericError> {
    hget_redis_typed(WALLETS_HASH, &wallet_id.to_string()).await
}

pub async fn delete_wallet(wallet_id: i32) -> Result<(), GenericError> {
    hdel_redis(WALLETS_HASH, &wallet_id.to_string()).await
}

// ============================================================================
// Address Tracking — Redis Hash "h:evm:tracked_addresses"
// ============================================================================

const TRACKED_ADDRESSES_HASH: &str = "h:evm:tracked_addresses";

/// Track an EVM address for monitoring.
///
/// Stores a JSON payload as the hash field value containing user_id, project_id, and token.
pub async fn track_address(
    user_id: i32,
    project_id: i32,
    address: String,
    token: String,
) -> Result<(), GenericError> {
    let payload = serde_json::json!({
        "user_id": user_id,
        "project_id": project_id,
        "token": token,
    });
    hset_redis(TRACKED_ADDRESSES_HASH, &address, &payload.to_string()).await
}

/// Remove an EVM address from tracking.
pub async fn remove_tracked_address(address: &str) -> Result<(), GenericError> {
    hdel_redis(TRACKED_ADDRESSES_HASH, address).await
}

/// Get all tracked EVM addresses.
pub async fn get_all_tracked_addresses() -> Result<Vec<String>, GenericError> {
    let raw = hgetall_redis(TRACKED_ADDRESSES_HASH).await?;
    Ok(raw.keys().cloned().collect())
}

// ============================================================================
// Daily Volume Cache — delegates to shared task_state module
// ============================================================================

pub async fn get_daily_volume(project_id: i32) -> Result<Option<f64>, GenericError> {
    let vol = vm_redis::task_state::get_daily_volume("evm", project_id).await?;
    // Return Some(vol) to keep the existing caller convention (Option<f64>)
    Ok(Some(vol))
}

pub async fn add_daily_volume(project_id: i32, volume_usdc: f64) -> Result<f64, GenericError> {
    vm_redis::task_state::add_daily_volume("evm", project_id, volume_usdc).await
}

/// Add volume for a specific date (used by init to resolve pending txs from previous days).
/// This uses evm-specific keys since the shared module only handles today's date.
pub async fn add_daily_volume_for_date(
    project_id: i32,
    date: &str,
    volume_usdc: f64,
) -> Result<f64, GenericError> {
    let key = format!("evm:daily_volume:{}:{}", project_id, date);
    let mut conn = get_connection().await;
    let new_total: f64 = redis::cmd("INCRBYFLOAT")
        .arg(&key)
        .arg(volume_usdc)
        .query_async(&mut conn)
        .await?;
    let _: () = redis::cmd("EXPIRE")
        .arg(&key)
        .arg(86400i64)
        .query_async(&mut conn)
        .await?;
    Ok(new_total)
}