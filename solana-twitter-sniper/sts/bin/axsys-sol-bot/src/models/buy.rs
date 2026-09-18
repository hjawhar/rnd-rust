use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuyRequestOutgoingPayload {
    pub mint_address: String,
    pub private_key: String,
    pub nonce_account_address: String,
    pub servers: String,
    pub block_leaders: String,
    pub value: f64,
    pub tip: f64,
    pub slippage: i32,
    pub tries: i32,
    pub frontrunning_protection: bool,
    pub enable_alerts: bool,
    pub selected_pool: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuyRequest {
    pub retry_uuid: String,
    pub uuid: String,
    pub private_key: String,
    pub nonce_account_address: String,
    pub mint: String,
    pub value: f64,
    pub tip: f64,
    pub slippage: i32,
    pub tries: i32,
    pub frontrunning_protection: bool,
    pub enable_alerts: bool,
    pub selected_pool: String,
    pub servers: Option<String>,
    pub block_leaders: Option<String>,
}
