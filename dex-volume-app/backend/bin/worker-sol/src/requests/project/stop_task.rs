use vm_data::db::Database;
use vm_data::models::streams::{StreamInfo, StreamType, TaskStatusInfo};
use vm_nats::subjects;

pub async fn stop_task_req(
    db: &Database,
    nc: &async_nats::Client,
    js: &async_nats::jetstream::Context,
    user_id: i32,
    project_id: i32,
) -> Option<bool> {
    let project = db.get_project(user_id, project_id).await.unwrap_or(None)?;

    let serialized = serde_json::to_vec(&StreamInfo {
        stream_type: StreamType::StopTask(project.id),
        user_id: project.user_id,
    })
    .unwrap_or_default();

    let _ = js
        .publish(subjects::cmd::sol::TASK_STOP, serialized.into())
        .await;

    // Clear task heartbeat
    let _ = vm_redis::task_heartbeat::clear_heartbeat("sol", project_id).await;

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
        .publish(subjects::events::sol::TASK_STATUS, status_bytes.into())
        .await;

    Some(true)
}
