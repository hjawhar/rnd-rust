use super::*;
use crate::db::schema::audit_logs;
use crate::models::audit_log::{AuditLog, NewAuditLog};

impl Database {
    pub async fn insert_audit_log(&self, log: &NewAuditLog) -> DbResult<AuditLog> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let result = diesel::insert_into(audit_logs::table)
            .values(log)
            .returning(AuditLog::as_returning())
            .get_result(connection)
            .await?;
        Ok(result)
    }

    pub async fn get_audit_logs(
        &self,
        user_id: Option<i32>,
        project_id: Option<i32>,
        limit: i64,
        offset: i64,
    ) -> DbResult<Vec<AuditLog>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;

        let mut query = audit_logs::table.into_boxed();

        if let Some(uid) = user_id {
            query = query.filter(audit_logs::user_id.eq(uid));
        }
        if let Some(pid) = project_id {
            query = query.filter(audit_logs::project_id.eq(pid));
        }

        let logs = query
            .order(audit_logs::created_at.desc())
            .limit(limit)
            .offset(offset)
            .load(connection)
            .await?;

        Ok(logs)
    }
}
