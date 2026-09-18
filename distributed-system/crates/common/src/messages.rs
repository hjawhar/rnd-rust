use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Client submits this to the gateway via HTTP POST /tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRequest {
    pub task_type: String,
    pub data: serde_json::Value,
}

/// Gateway publishes this to NATS subject `tasks.<task_type>`.
/// Wraps the client request with a unique ID and timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMessage {
    pub request_id: Uuid,
    pub task_type: String,
    pub data: serde_json::Value,
    pub submitted_at: DateTime<Utc>,
}

/// Processor sends this to Enricher via NATS request-reply on `enrich.request`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentRequest {
    pub request_id: Uuid,
    pub task_type: String,
    pub data: serde_json::Value,
}

/// Enricher responds with this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentResponse {
    pub request_id: Uuid,
    pub tags: Vec<String>,
    pub geo: GeoPoint,
    pub enriched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoPoint {
    pub lat: f64,
    pub lon: f64,
}

/// Processor publishes this to NATS subject `results.<task_type>`.
/// Contains original data + enrichment + processing metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub request_id: Uuid,
    pub task_type: String,
    pub payload: serde_json::Value,
    pub enrichment: EnrichmentResponse,
    pub processor_id: String,
    pub processed_at: DateTime<Utc>,
}

/// Gateway returns this to the client after accepting a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAccepted {
    pub request_id: Uuid,
    pub status: &'static str,
}

/// Aggregator returns this from GET /stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregationStats {
    pub total_results: i64,
    pub by_task_type: Vec<TaskTypeCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTypeCount {
    pub task_type: String,
    pub count: i64,
}

/// NATS subject helpers.
pub mod subjects {
    /// Returns `tasks.<task_type>` for publishing.
    pub fn task_subject(task_type: &str) -> String {
        format!("tasks.{task_type}")
    }

    /// Wildcard subscription for all task types.
    pub const TASKS_ALL: &str = "tasks.*";

    /// Queue group name for competing processor consumers.
    pub const PROCESSOR_QUEUE_GROUP: &str = "processors";

    /// Subject for enrichment request-reply.
    pub const ENRICH_REQUEST: &str = "enrich.request";

    /// Returns `results.<task_type>` for publishing.
    pub fn result_subject(task_type: &str) -> String {
        format!("results.{task_type}")
    }

    /// Wildcard subscription for all result types.
    pub const RESULTS_ALL: &str = "results.*";
}
