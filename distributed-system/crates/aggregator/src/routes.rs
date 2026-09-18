use std::sync::Arc;

use axum::{Json, extract::{Query, State}, http::StatusCode};
use common::messages::{AggregationStats, TaskTypeCount};
use serde::Deserialize;

use crate::AppState;
use crate::db;
use crate::models::ResultRow;

#[derive(Deserialize)]
pub struct ResultsQuery {
    pub task_type: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    50
}

/// GET /results?task_type=compute&limit=20
pub async fn get_results(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ResultsQuery>,
) -> Result<Json<Vec<ResultRow>>, (StatusCode, String)> {
    let results = db::query_results(&state.db_pool, params.task_type.as_deref(), params.limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(results))
}

/// GET /stats
pub async fn get_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AggregationStats>, (StatusCode, String)> {
    let (total, by_type) = db::query_stats(&state.db_pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(AggregationStats {
        total_results: total,
        by_task_type: by_type
            .into_iter()
            .map(|(task_type, count)| TaskTypeCount { task_type, count })
            .collect(),
    }))
}
