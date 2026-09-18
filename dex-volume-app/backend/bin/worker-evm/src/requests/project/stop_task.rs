use vm_data::db::Database;
use vm_data::models::streams::{StreamInfo, StreamType, TaskStatusInfo};
use vm_nats::subjects;

pub async fn stop_task_req(
    db: &Database,
    nc: &async_nats::Client,
    user_id: i32,
    project_id: i32,
) -> Option<bool> {
    let project = db.get_project(user_id, project_id).await.ok().flatten()?;

    // Remove task directly from cache so the volume maker loop sees it immediately.
    // The JetStream command is also published for any other coordination, but the
    // direct cache removal is the authoritative stop signal.
    let _ = crate::cache::remove_task(project_id).await;
    let _ = crate::cache::reset_task_failures(project_id).await;

    let serialized = serde_json::to_vec(&StreamInfo {
        stream_type: StreamType::StopTask(project.id),
        user_id: project.user_id,
    }).unwrap_or_default();

    let _ = nc
        .publish(subjects::cmd::evm::TASK_STOP, serialized.into())
        .await;

    // Clear task heartbeat and release ownership
    let _ = vm_redis::task_heartbeat::clear_heartbeat("evm", project_id).await;
    let _ = crate::cache::release_task_ownership(project_id).await;

    // Update DB status and broadcast to WebSocket clients
    let _ = db.update_project_status(project_id, "stopped").await;
    let status_bytes = serde_json::to_vec(&StreamInfo {
        user_id: project.user_id,
        stream_type: StreamType::TaskStatusUpdate(TaskStatusInfo {
            project_id,
            status: "stopped".to_string(),
        }),
    })
    .unwrap_or_default();
    let _ = nc
        .publish(subjects::events::evm::TASK_STATUS, status_bytes.into())
        .await;

    Some(true)
}
