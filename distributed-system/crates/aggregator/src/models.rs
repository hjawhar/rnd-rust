use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::Serialize;
use uuid::Uuid;

use crate::schema::results;

/// For inserting a new result row.
#[derive(Insertable)]
#[diesel(table_name = results)]
pub struct NewResult {
    pub request_id: Uuid,
    pub task_type: String,
    pub payload: serde_json::Value,
    pub enrichment: Option<serde_json::Value>,
    pub processed_at: DateTime<Utc>,
    pub processor_id: String,
}

/// For querying result rows.
#[derive(Queryable, Selectable, Serialize)]
#[diesel(table_name = results)]
pub struct ResultRow {
    pub id: Uuid,
    pub request_id: Uuid,
    pub task_type: String,
    pub payload: serde_json::Value,
    pub enrichment: Option<serde_json::Value>,
    pub processed_at: DateTime<Utc>,
    pub processor_id: String,
}
