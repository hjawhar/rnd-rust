use common::messages::TaskResult;
use deadpool_diesel::postgres::Pool;
use diesel::prelude::*;
use diesel::dsl::count_star;

use crate::models::{NewResult, ResultRow};
use crate::schema::results;

/// Insert a processed TaskResult into the database.
pub async fn insert_result(
    pool: &Pool,
    task_result: &TaskResult,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let new_result = NewResult {
        request_id: task_result.request_id,
        task_type: task_result.task_type.clone(),
        payload: task_result.payload.clone(),
        enrichment: Some(serde_json::to_value(&task_result.enrichment)?),
        processed_at: task_result.processed_at,
        processor_id: task_result.processor_id.clone(),
    };

    let conn = pool.get().await?;
    conn.interact(move |conn| {
        diesel::insert_into(results::table)
            .values(&new_result)
            .execute(conn)
    })
    .await
    .map_err(|e| format!("interact error: {e}"))??;

    Ok(())
}

/// Query recent results, optionally filtered by task_type.
/// Returns up to `limit` results ordered by processed_at descending.
pub async fn query_results(
    pool: &Pool,
    task_type: Option<&str>,
    limit: i64,
) -> Result<Vec<ResultRow>, Box<dyn std::error::Error + Send + Sync>> {
    let task_type = task_type.map(String::from);

    let conn = pool.get().await?;
    let rows = conn
        .interact(move |conn| {
            let mut query = results::table
                .order(results::processed_at.desc())
                .limit(limit)
                .into_boxed();

            if let Some(ref tt) = task_type {
                query = query.filter(results::task_type.eq(tt));
            }

            query.select(ResultRow::as_select()).load::<ResultRow>(conn)
        })
        .await
        .map_err(|e| format!("interact error: {e}"))??;

    Ok(rows)
}

/// Get count of results grouped by task_type, plus total count.
pub async fn query_stats(
    pool: &Pool,
) -> Result<(i64, Vec<(String, i64)>), Box<dyn std::error::Error + Send + Sync>> {
    let conn = pool.get().await?;

    let stats = conn
        .interact(|conn| {
            let total: i64 = results::table
                .select(count_star())
                .first(conn)?;

            let by_type: Vec<(String, i64)> = results::table
                .group_by(results::task_type)
                .select((results::task_type, count_star()))
                .order(count_star().desc())
                .load::<(String, i64)>(conn)?;

            Ok::<_, diesel::result::Error>((total, by_type))
        })
        .await
        .map_err(|e| format!("interact error: {e}"))??;

    Ok(stats)
}
