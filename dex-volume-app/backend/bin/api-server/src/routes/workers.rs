use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Json,
    extract::State,
    response::IntoResponse,
};
use serde::Serialize;

use crate::models::state::AppState;

/// A worker is considered offline if its last heartbeat is older than this.
const WORKER_ONLINE_THRESHOLD_SECS: u64 = 45;

#[derive(Serialize)]
pub struct WorkersResponse {
    pub sol: Vec<WorkerInfo>,
    pub evm: Vec<WorkerInfo>,
}

#[derive(Serialize)]
pub struct WorkerInfo {
    pub worker_id: String,
    pub last_heartbeat: u64,
    pub online: bool,
    pub projects: Vec<WorkerProject>,
}

#[derive(Serialize)]
pub struct WorkerProject {
    pub project_id: i32,
    pub name: String,
    pub status: String,
    pub generation: u64,
    pub started_at: u64,
    pub heartbeat_alive: bool,
}

pub async fn get_workers_req(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match build_workers_response(&state).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => {
            tracing::error!("failed to build workers response: {e}");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("internal error: {e}"),
            )
                .into_response()
        }
    }
}

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

async fn build_workers_response(state: &AppState) -> Result<WorkersResponse, BoxErr> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let (sol_workers, evm_workers) = tokio::try_join!(
        build_chain_workers(state, "sol", now),
        build_chain_workers(state, "evm", now),
    )?;

    Ok(WorkersResponse {
        sol: sol_workers,
        evm: evm_workers,
    })
}

async fn build_chain_workers(
    state: &AppState,
    chain: &str,
    now: u64,
) -> Result<Vec<WorkerInfo>, BoxErr> {
    let registrations = vm_redis::hgetall_redis(&format!("workers:{chain}")).await?;
    let ownership = vm_redis::task_ownership::get_all_owners(chain).await?;

    // Group ownership by worker_id.
    let mut worker_projects: HashMap<String, Vec<(i32, &vm_redis::task_ownership::TaskOwnership)>> =
        HashMap::new();
    for (project_id, owner) in &ownership {
        worker_projects
            .entry(owner.worker_id.clone())
            .or_default()
            .push((*project_id, owner));
    }

    let mut results: Vec<WorkerInfo> = Vec::new();
    let mut seen_workers: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Registered workers first.
    for (worker_id, ts_str) in &registrations {
        seen_workers.insert(worker_id.clone());
        let last_heartbeat: u64 = ts_str.parse().unwrap_or(0);
        let online = now.saturating_sub(last_heartbeat) <= WORKER_ONLINE_THRESHOLD_SECS;

        let projects = match worker_projects.get(worker_id) {
            Some(owned) => build_worker_projects(state, chain, owned).await?,
            None => Vec::new(),
        };

        results.push(WorkerInfo {
            worker_id: worker_id.clone(),
            last_heartbeat,
            online,
            projects,
        });
    }

    // Stale/crashed workers that still own projects but are no longer registered.
    for (worker_id, owned) in &worker_projects {
        if seen_workers.contains(worker_id) {
            continue;
        }
        let projects = build_worker_projects(state, chain, owned).await?;
        results.push(WorkerInfo {
            worker_id: worker_id.clone(),
            last_heartbeat: 0,
            online: false,
            projects,
        });
    }

    // Online workers first, then alphabetical by worker_id.
    results.sort_by(|a, b| b.online.cmp(&a.online).then(a.worker_id.cmp(&b.worker_id)));

    Ok(results)
}

async fn build_worker_projects(
    state: &AppState,
    chain: &str,
    owned: &[(i32, &vm_redis::task_ownership::TaskOwnership)],
) -> Result<Vec<WorkerProject>, BoxErr> {
    let mut projects = Vec::with_capacity(owned.len());

    for &(project_id, ownership) in owned {
        let name = match state.db.get_project_by_id(project_id).await {
            Ok(Some(p)) => p.name.unwrap_or_default(),
            _ => String::new(),
        };

        let heartbeat_alive = vm_redis::task_heartbeat::is_alive(chain, project_id)
            .await
            .unwrap_or(false);

        let status = if heartbeat_alive { "running" } else { "stopped" };

        projects.push(WorkerProject {
            project_id,
            name,
            status: status.to_string(),
            generation: ownership.generation,
            started_at: ownership.started_at,
            heartbeat_alive,
        });
    }

    projects.sort_by_key(|p| p.project_id);
    Ok(projects)
}
