use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use redis::Script;
use serde::{Deserialize, Serialize};

use crate::{get_conn, GenericError};

/// Ownership record for a task (project) within a chain.
/// Stored as JSON in a Redis hash field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOwnership {
    pub worker_id: String,
    pub generation: u64,
    pub started_at: u64,
}

/// Returns the Redis hash key for task ownership within a chain.
fn ownership_key(chain: &str) -> String {
    format!("task:owners:{chain}")
}

/// Returns the current unix timestamp in seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Atomically claim ownership of a project within a chain.
///
/// - If unclaimed, creates a new ownership record (generation=1).
/// - If already owned by the same worker, bumps generation and updates started_at (idempotent restart).
/// - If owned by a different worker, returns `None`.
pub async fn claim_task(
    chain: &str,
    project_id: i32,
    worker_id: &str,
) -> Result<Option<TaskOwnership>, GenericError> {
    let script = Script::new(
        r#"
local current = redis.call('HGET', KEYS[1], ARGV[1])
if current then
    local data = cjson.decode(current)
    if data.worker_id == ARGV[2] then
        data.generation = data.generation + 1
        data.started_at = tonumber(ARGV[3])
        local new = cjson.encode(data)
        redis.call('HSET', KEYS[1], ARGV[1], new)
        return new
    else
        return nil
    end
else
    local new = cjson.encode({worker_id=ARGV[2], generation=1, started_at=tonumber(ARGV[3])})
    redis.call('HSET', KEYS[1], ARGV[1], new)
    return new
end
"#,
    );

    let mut conn = get_conn().await;
    let result: Option<String> = script
        .key(ownership_key(chain))
        .arg(project_id.to_string())
        .arg(worker_id)
        .arg(now_secs())
        .invoke_async(&mut conn)
        .await?;

    match result {
        Some(json) => Ok(Some(serde_json::from_str(&json)?)),
        None => Ok(None),
    }
}

/// Atomically release ownership of a project, but only if the caller is the current owner.
///
/// Returns `true` if the task was released, `false` if it was not owned by this worker
/// (or not owned at all).
pub async fn release_task(
    chain: &str,
    project_id: i32,
    worker_id: &str,
) -> Result<bool, GenericError> {
    let script = Script::new(
        r#"
local current = redis.call('HGET', KEYS[1], ARGV[1])
if current then
    local data = cjson.decode(current)
    if data.worker_id == ARGV[2] then
        redis.call('HDEL', KEYS[1], ARGV[1])
        return 1
    end
end
return 0
"#,
    );

    let mut conn = get_conn().await;
    let result: i32 = script
        .key(ownership_key(chain))
        .arg(project_id.to_string())
        .arg(worker_id)
        .invoke_async(&mut conn)
        .await?;

    Ok(result == 1)
}

/// Get the current ownership record for a project within a chain.
///
/// Returns `None` if the project has no owner.
pub async fn get_owner(
    chain: &str,
    project_id: i32,
) -> Result<Option<TaskOwnership>, GenericError> {
    let mut conn = get_conn().await;
    let val: Option<String> = redis::cmd("HGET")
        .arg(ownership_key(chain))
        .arg(project_id.to_string())
        .query_async(&mut conn)
        .await?;

    match val {
        Some(json) => Ok(Some(serde_json::from_str(&json)?)),
        None => Ok(None),
    }
}

/// Get all project IDs currently owned by a specific worker within a chain.
pub async fn get_owned_projects(
    chain: &str,
    worker_id: &str,
) -> Result<Vec<i32>, GenericError> {
    let all = get_all_owners(chain).await?;
    let projects = all
        .into_iter()
        .filter(|(_, ownership)| ownership.worker_id == worker_id)
        .map(|(pid, _)| pid)
        .collect();
    Ok(projects)
}

/// Atomically bump the generation counter for a project's ownership record.
///
/// Returns the new generation, or `None` if the project has no owner.
pub async fn bump_generation(
    chain: &str,
    project_id: i32,
) -> Result<Option<u64>, GenericError> {
    let script = Script::new(
        r#"
local current = redis.call('HGET', KEYS[1], ARGV[1])
if current then
    local data = cjson.decode(current)
    data.generation = data.generation + 1
    local new = cjson.encode(data)
    redis.call('HSET', KEYS[1], ARGV[1], new)
    return data.generation
else
    return nil
end
"#,
    );

    let mut conn = get_conn().await;
    let result: Option<u64> = script
        .key(ownership_key(chain))
        .arg(project_id.to_string())
        .invoke_async(&mut conn)
        .await?;

    Ok(result)
}

/// Unconditionally remove ownership of a project. Used by orphan sweep.
pub async fn force_release(chain: &str, project_id: i32) -> Result<(), GenericError> {
    let mut conn = get_conn().await;
    redis::cmd("HDEL")
        .arg(ownership_key(chain))
        .arg(project_id.to_string())
        .query_async::<()>(&mut conn)
        .await?;
    Ok(())
}

/// Get all ownership records for a chain as a map of project_id → ownership.
pub async fn get_all_owners(
    chain: &str,
) -> Result<HashMap<i32, TaskOwnership>, GenericError> {
    let mut conn = get_conn().await;
    let raw: HashMap<String, String> = redis::cmd("HGETALL")
        .arg(ownership_key(chain))
        .query_async(&mut conn)
        .await?;

    let mut result = HashMap::with_capacity(raw.len());
    for (field, value) in raw {
        let pid: i32 = field.parse()?;
        let ownership: TaskOwnership = serde_json::from_str(&value)?;
        result.insert(pid, ownership);
    }
    Ok(result)
}
