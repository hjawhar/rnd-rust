use std::time::SystemTime;

use diesel::{
    prelude::{Insertable, Queryable},
    Selectable,
};
use serde::{Deserialize, Serialize};

use crate::db::schema::users;

#[derive(Debug, Deserialize)]
pub struct LoginNoncePayload {
    pub address: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginPayload {
    pub address: String,
    pub nonce: String,
    pub signed_nonce: String,
}

#[derive(Debug, Deserialize)]
pub struct AddUserPayload {
    pub address: String,
    pub group_id: i32,
}

#[derive(Deserialize, Serialize, Queryable, Debug, Selectable, Clone)]
pub struct User {
    pub id: i32,
    pub group_id: i32,
    pub address: String,
    pub nonce: String,
    pub whitelisted: bool,
    pub date_added: SystemTime,
    pub refresh_token: Option<String>,
    pub refresh_token_expires_at: Option<SystemTime>,
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize, Insertable, Selectable)]
#[diesel(table_name = users)]
pub struct NewUser {
    pub address: String,
    pub nonce: String,
    pub whitelisted: bool,
    pub group_id: i32,
    pub date_added: SystemTime
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub address: String,
    pub jwt: String,
    pub refresh_token: String,
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub struct RefreshTokenPayload {
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub jwt: String,
    pub refresh_token: String,
}
