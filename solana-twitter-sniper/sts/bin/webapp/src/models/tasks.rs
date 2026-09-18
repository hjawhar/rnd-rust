use crate::db::schema::tasks;
use bigdecimal::BigDecimal;
use diesel::{
    prelude::{Insertable, Queryable},
    Selectable,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Insertable, Queryable, Selectable)]
pub struct Task {
    pub id: i32,
    pub user_id: i32,
    pub wallet_id: Option<i32>,
    pub twitter_id: Option<String>,
    pub twitter_handle: Option<String>,
    pub servers: Option<String>,
    pub block_leaders: Option<String>,
    pub value: Option<BigDecimal>,
    pub tip: Option<BigDecimal>,
    pub slippage: i32,
    pub tries: i32,
    pub frontrunning_protection: bool,
    pub enable_alerts: bool,
    pub selected_pool: String,
    pub twitter_api: String,
    pub twitter_strategy: Option<String>,
    pub twitter_handle_checker: Option<String>,
    pub twitter_token_override: Option<String>,
    pub words: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TaskWallet {
    pub id: i32,
    pub user_id: i32,
    pub private_key: Option<String>,
    pub public_key: Option<String>,
    pub nonce_account_address: Option<String>,
    pub twitter_id: Option<String>,
    pub twitter_handle: Option<String>,
    pub servers: Option<String>,
    pub block_leaders: Option<String>,
    pub value: Option<BigDecimal>,
    pub tip: Option<BigDecimal>,
    pub slippage: i32,
    pub tries: i32,
    pub frontrunning_protection: bool,
    pub enable_alerts: bool,
    pub selected_pool: String,
    pub twitter_api: String,
    pub twitter_strategy: Option<String>,
    pub twitter_handle_checker: Option<String>,
    pub twitter_token_override: Option<String>,
    pub words: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TaskMinimal {
    pub id: i32,
    pub user_id: i32,
    pub wallet_id: Option<i32>,
    pub twitter_id: Option<String>,
    pub twitter_handle: Option<String>,
    pub servers: Option<String>,
    pub block_leaders: Option<String>,
    pub value: Option<BigDecimal>,
    pub tip: Option<BigDecimal>,
    pub slippage: i32,
    pub tries: i32,
    pub frontrunning_protection: bool,
    pub enable_alerts: bool,
    pub selected_pool: String,
    pub twitter_api: String,
    pub twitter_strategy: Option<String>,
    pub twitter_handle_checker: Option<String>,
    pub twitter_token_override: Option<String>,
    pub words: Option<String>,
}

#[derive(Debug, Deserialize, Insertable, Selectable)]
#[diesel(table_name = tasks)]
pub struct NewTask {
    pub user_id: i32,
    pub wallet_id: Option<i32>,
    pub twitter_id: Option<String>,
    pub twitter_handle: Option<String>,
    pub servers: Option<String>,
    pub block_leaders: Option<String>,
    pub value: Option<BigDecimal>,
    pub tip: Option<BigDecimal>,
    pub slippage: i32,
    pub tries: i32,
    pub frontrunning_protection: bool,
    pub enable_alerts: bool,
    pub selected_pool: String,
    pub twitter_api: String,
    pub twitter_strategy: Option<String>,
    pub twitter_handle_checker: Option<String>,
    pub twitter_token_override: Option<String>,
    pub words: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateTask {
    pub wallet_id: Option<i32>,
    pub twitter_id: Option<String>,
    pub twitter_handle: Option<String>,
    pub servers: Option<String>,
    pub block_leaders: Option<String>,
    pub value: Option<BigDecimal>,
    pub tip: Option<BigDecimal>,
    pub slippage: i32,
    pub tries: i32,
    pub frontrunning_protection: bool,
    pub enable_alerts: bool,
    pub selected_pool: String,
    pub twitter_api: String,
    pub twitter_strategy: Option<String>,
    pub twitter_handle_checker: Option<String>,
    pub twitter_token_override: Option<String>,
    pub words: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateTaskPayload {
    pub wallet_id: Option<i32>,
    pub twitter_id: Option<String>,
    pub twitter_handle: Option<String>,
    pub servers: Option<String>,
    pub block_leaders: Option<String>,
    pub value: Option<f64>,
    pub tip: Option<f64>,
    pub slippage: i32,
    pub tries: i32,
    pub frontrunning_protection: bool,
    pub enable_alerts: bool,
    pub selected_pool: String,
    pub twitter_api: String,
    pub twitter_strategy: Option<String>,
    pub twitter_handle_checker: Option<String>,
    pub twitter_token_override: Option<String>,
    pub words: Option<String>,
}
