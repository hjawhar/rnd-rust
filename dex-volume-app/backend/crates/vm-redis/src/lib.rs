use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};

use redis::aio::MultiplexedConnection;
use redis::AsyncTypedCommands;
use serde::{Deserialize, Serialize};
use tokio::sync::{OnceCell, RwLock};

pub mod task_ownership;
pub mod task_state;
pub mod task_heartbeat;
pub mod worker_heartbeat;

pub type GenericError = Box<dyn std::error::Error + Send + Sync>;

/// Redis connection pool with automatic reconnection.
/// Each slot holds a MultiplexedConnection. On failure, the slot is
/// lazily replaced with a fresh connection.
struct RedisPool {
    client: redis::Client,
    connections: RwLock<Vec<MultiplexedConnection>>,
    pool_size: usize,
}

impl RedisPool {
    async fn new() -> Self {
        let redis_url = env::var("REDIS_URL").expect("REDIS_URL is required");
        let pool_size: usize = env::var("REDIS_POOL_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8);

        let client = redis::Client::open(redis_url).expect("Failed to open redis connection");
        let mut conns = Vec::with_capacity(pool_size);
        for i in 0..pool_size {
            let conn = client
                .get_multiplexed_async_connection()
                .await
                .unwrap_or_else(|e| panic!("Failed to create Redis connection {}/{}: {}", i + 1, pool_size, e));
            conns.push(conn);
        }
        tracing::info!("Redis connection pool created (size={})", pool_size);

        Self {
            client,
            connections: RwLock::new(conns),
            pool_size,
        }
    }

    /// Get a connection, reconnecting the slot if it's dead.
    async fn get(&self) -> MultiplexedConnection {
        let idx = POOL_INDEX.fetch_add(1, Ordering::Relaxed) % self.pool_size;

        // Fast path: clone under read lock
        let conn = {
            let conns = self.connections.read().await;
            conns[idx].clone()
        };

        // Quick health check: if PING fails, reconnect this slot
        let mut test_conn = conn.clone();
        let healthy = redis::cmd("PING")
            .query_async::<String>(&mut test_conn)
            .await
            .is_ok();

        if healthy {
            return conn;
        }

        // Slow path: reconnect under write lock
        tracing::warn!("[Redis] Connection {}/{} is dead, reconnecting...", idx + 1, self.pool_size);
        let mut conns = self.connections.write().await;
        // Double-check: another task may have already reconnected this slot
        let mut recheck = conns[idx].clone();
        if redis::cmd("PING")
            .query_async::<String>(&mut recheck)
            .await
            .is_ok()
        {
            return conns[idx].clone();
        }

        match self.client.get_multiplexed_async_connection().await {
            Ok(new_conn) => {
                conns[idx] = new_conn.clone();
                tracing::info!("[Redis] Connection {}/{} reconnected", idx + 1, self.pool_size);
                new_conn
            }
            Err(e) => {
                tracing::error!("[Redis] Failed to reconnect slot {}/{}: {}", idx + 1, self.pool_size, e);
                // Return the dead connection — caller will get an error on use
                conns[idx].clone()
            }
        }
    }
}

static REDIS_POOL: OnceCell<RedisPool> = OnceCell::const_new();
static POOL_INDEX: AtomicUsize = AtomicUsize::new(0);

async fn get_pool() -> &'static RedisPool {
    REDIS_POOL.get_or_init(RedisPool::new).await
}

/// Get a Redis connection from the round-robin pool.
/// Automatically reconnects dead connections.
pub async fn get_conn() -> MultiplexedConnection {
    get_pool().await.get().await
}

/// Legacy alias — prefer `get_conn()` for new code.
pub async fn connect_redis() -> MultiplexedConnection {
    get_conn().await
}

// ============================================================================
// Health Check
// ============================================================================

/// Ping Redis to verify connectivity. Returns Ok(true) if healthy.
pub async fn ping_redis() -> Result<bool, GenericError> {
    let mut conn = get_conn().await;
    let pong: String = redis::cmd("PING")
        .query_async(&mut conn)
        .await?;
    Ok(pong == "PONG")
}

// ============================================================================
// Generic Cache Operations
// ============================================================================

/// Get data from Redis cache with automatic deserialization
pub async fn get_redis_data<T: for<'a> Deserialize<'a>>(
    key: String,
) -> Result<Option<T>, GenericError> {
    let mut redis = get_conn().await;

    let data_exec: Option<String> = redis.get(key.clone()).await
        .map_err(|_| -> GenericError { format!("Failed to fetch {key}").into() })?;

    if let Some(data_found) = data_exec {
        let data = serde_json::from_str::<T>(&data_found)
            .map_err(|_| -> GenericError { format!("Failed to deserialize {key}").into() })?;
        Ok(Some(data))
    } else {
        Ok(None)
    }
}

/// Insert data into Redis with automatic serialization
pub async fn insert_redis_data<T: Serialize>(key: String, value: T) -> Result<(), GenericError> {
    let mut redis = get_conn().await;
    let data = serde_json::to_string(&value)?;
    redis.set(key, data).await?;
    Ok(())
}

/// Insert data into Redis with a TTL (time-to-live) in seconds
pub async fn insert_redis_data_with_ttl<T: Serialize>(
    key: String,
    value: T,
    ttl_seconds: u64,
) -> Result<(), GenericError> {
    let mut redis = get_conn().await;
    let data = serde_json::to_string(&value)?;
    redis.set_ex(key, data, ttl_seconds).await?;
    Ok(())
}

/// Delete a key from Redis
pub async fn del_redis(key: &str) -> Result<(), GenericError> {
    let mut conn = get_conn().await;
    redis::cmd("DEL")
        .arg(key)
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

/// Check if a key exists in Redis
pub async fn exists_redis(key: &str) -> Result<bool, GenericError> {
    let mut conn = get_conn().await;
    let val: bool = redis::cmd("EXISTS")
        .arg(key)
        .query_async(&mut conn)
        .await?;
    Ok(val)
}

// ============================================================================
// Redis Hash Operations (atomic per-field writes, no read-modify-write)
// ============================================================================

/// Set a field in a Redis Hash
pub async fn hset_redis(key: &str, field: &str, value: &str) -> Result<(), GenericError> {
    let mut conn = get_conn().await;
    redis::cmd("HSET")
        .arg(key).arg(field).arg(value)
        .query_async::<()>(&mut conn).await?;
    Ok(())
}

/// Get a field from a Redis Hash
pub async fn hget_redis(key: &str, field: &str) -> Result<Option<String>, GenericError> {
    let mut conn = get_conn().await;
    let val: Option<String> = redis::cmd("HGET")
        .arg(key).arg(field)
        .query_async(&mut conn).await?;
    Ok(val)
}

/// Delete a field from a Redis Hash
pub async fn hdel_redis(key: &str, field: &str) -> Result<(), GenericError> {
    let mut conn = get_conn().await;
    redis::cmd("HDEL")
        .arg(key).arg(field)
        .query_async::<()>(&mut conn).await?;
    Ok(())
}

/// Get all fields and values from a Redis Hash
pub async fn hgetall_redis(key: &str) -> Result<std::collections::HashMap<String, String>, GenericError> {
    let mut conn = get_conn().await;
    let val: std::collections::HashMap<String, String> = redis::cmd("HGETALL")
        .arg(key)
        .query_async(&mut conn).await?;
    Ok(val)
}

/// Check if a field exists in a Redis Hash
pub async fn hexists_redis(key: &str, field: &str) -> Result<bool, GenericError> {
    let mut conn = get_conn().await;
    let val: bool = redis::cmd("HEXISTS")
        .arg(key).arg(field)
        .query_async(&mut conn).await?;
    Ok(val)
}

/// Set a typed (JSON-serialized) value in a Redis Hash field
pub async fn hset_redis_typed<T: Serialize>(key: &str, field: &str, value: &T) -> Result<(), GenericError> {
    let serialized = serde_json::to_string(value)?;
    hset_redis(key, field, &serialized).await
}

/// Get a typed (JSON-deserialized) value from a Redis Hash field
pub async fn hget_redis_typed<T: for<'a> Deserialize<'a>>(key: &str, field: &str) -> Result<Option<T>, GenericError> {
    let val = hget_redis(key, field).await?;
    match val {
        Some(s) => {
            let parsed = serde_json::from_str::<T>(&s)?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}
