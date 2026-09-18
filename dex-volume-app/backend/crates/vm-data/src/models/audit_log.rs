use std::time::SystemTime;

use crate::db::schema::audit_logs;
use diesel::{
    Selectable,
    prelude::{Insertable, Queryable},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Queryable, Selectable)]
#[diesel(table_name = audit_logs)]
pub struct AuditLog {
    pub id: i32,
    pub user_id: i32,
    pub project_id: Option<i32>,
    pub action: String,
    pub details: Option<serde_json::Value>,
    pub created_at: SystemTime,
}

#[derive(Debug, Serialize, Deserialize, Clone, Insertable)]
#[diesel(table_name = audit_logs)]
pub struct NewAuditLog {
    pub user_id: i32,
    pub project_id: Option<i32>,
    pub action: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub user_id: Option<i32>,
    pub project_id: Option<i32>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
