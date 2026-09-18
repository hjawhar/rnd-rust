use std::time::SystemTime;

use crate::db::schema::project_access;
use diesel::{
    Selectable,
    prelude::{Insertable, Queryable},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Queryable, Selectable)]
#[diesel(table_name = project_access)]
pub struct ProjectAccess {
    pub id: i32,
    pub project_id: i32,
    pub user_id: i32,
    pub created_at: SystemTime,
}

#[derive(Debug, Serialize, Deserialize, Clone, Insertable)]
#[diesel(table_name = project_access)]
pub struct NewProjectAccess {
    pub project_id: i32,
    pub user_id: i32,
    pub created_at: SystemTime,
}
