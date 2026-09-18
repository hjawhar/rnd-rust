use bigdecimal::BigDecimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuyRequestPayload {
    pub mint_address: String,
    pub wallet_id: i32,
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
