use super::*;
use crate::db::schema::{project_access, projects};
use crate::models::project_access::{NewProjectAccess, ProjectAccess};

impl Database {
    pub async fn grant_project_access(&self, project_id: i32, user_id: i32) -> DbResult<ProjectAccess> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let new_access = NewProjectAccess {
            project_id,
            user_id,
            created_at: SystemTime::now(),
        };
        let access = diesel::insert_into(project_access::table)
            .values(&new_access)
            .returning(ProjectAccess::as_returning())
            .get_result(connection)
            .await?;
        Ok(access)
    }

    pub async fn revoke_project_access(&self, project_id: i32, user_id: i32) -> DbResult<bool> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let deleted = diesel::delete(
            project_access::table.filter(
                project_access::project_id.eq(project_id)
                    .and(project_access::user_id.eq(user_id)),
            ),
        )
        .execute(connection)
        .await?;
        Ok(deleted > 0)
    }

    pub async fn get_project_access_users(&self, project_id: i32) -> DbResult<Vec<ProjectAccess>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;
        let access_list = project_access::table
            .filter(project_access::project_id.eq(project_id))
            .load(connection)
            .await?;
        Ok(access_list)
    }

    /// Returns the owner user_id + all granted user_ids for a project.
    pub async fn get_project_authorized_user_ids(&self, project_id: i32) -> DbResult<Vec<i32>> {
        let mut pool_conn = self.get_connection().await?;
        let connection = &mut *pool_conn;

        let owner_id: Option<i32> = projects::table
            .filter(projects::id.eq(project_id))
            .select(projects::user_id)
            .first(connection)
            .await
            .optional()?;

        let granted_ids: Vec<i32> = project_access::table
            .filter(project_access::project_id.eq(project_id))
            .select(project_access::user_id)
            .load(connection)
            .await?;

        let mut ids = granted_ids;
        if let Some(owner) = owner_id
            && !ids.contains(&owner) {
                ids.insert(0, owner);
            }
        Ok(ids)
    }
}
