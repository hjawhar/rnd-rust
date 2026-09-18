use std::time::SystemTime;

use crate::db::schema::projects;
use bigdecimal::BigDecimal;
use diesel::{
    AsChangeset, Selectable,
    prelude::{Insertable, Queryable},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Insertable, Queryable, Selectable)]
pub struct Project {
    pub id: i32,
    pub user_id: i32,
    pub address: String,
    pub pool: String,
    pub pool_type: String,
    pub trading_strategy: String,
    pub trading_interval: BigDecimal,
    pub trading_daily_volume: BigDecimal,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub date_added: SystemTime,
    pub fees: BigDecimal,
    pub network: String,
    pub decimals: Option<i32>,
    pub pair: Option<String>,
    pub max_market_impact_bps: Option<i32>,
    pub trade_multiplier: Option<f64>,
    pub status: String,
    pub slippage: Option<f64>,
    pub bundle_enabled: Option<bool>,
    pub jito_tip: Option<f64>,
    pub locked: bool,
    pub lock_at: Option<SystemTime>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Insertable, Selectable)]
#[diesel(table_name = projects)]
pub struct NewProject {
    pub user_id: i32,
    pub address: String,
    pub pool: String,
    pub pool_type: String,
    pub trading_strategy: String,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub date_added: SystemTime,
    pub fees: BigDecimal,
    pub network: String,
    pub decimals: Option<i32>,
    pub pair: Option<String>,
    pub max_market_impact_bps: Option<i32>,
    pub trade_multiplier: Option<f64>,
    pub slippage: Option<f64>,
    pub bundle_enabled: Option<bool>,
    pub jito_tip: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NewProjectPayload {
    pub address: String,
    pub pool: Option<String>,
    pub trading_strategy: String,
    #[serde(default = "default_network")]
    pub network: String,
}

fn default_network() -> String {
    "solana".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize, AsChangeset)]
#[diesel(table_name = projects)]
#[diesel(treat_none_as_null = false)]
pub struct UpdateProjectPayload {
    pub trading_interval: Option<BigDecimal>,
    pub trading_daily_volume: Option<BigDecimal>,
    pub max_market_impact_bps: Option<i32>,
    pub trade_multiplier: Option<f64>,
    pub slippage: Option<f64>,
    pub bundle_enabled: Option<bool>,
    pub jito_tip: Option<f64>,
}
