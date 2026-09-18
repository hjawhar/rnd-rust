use crate::db::schema::uniswap_v4_pools;
use diesel::prelude::{Insertable, Queryable};
use diesel::Selectable;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Queryable, Selectable)]
#[diesel(table_name = uniswap_v4_pools)]
pub struct UniswapV4Pool {
    pub id: i32,
    pub network_id: i32,
    pub token_id: i64,
    pub pool_key: String,
    pub currency0: String,
    pub currency1: String,
    pub tick_spacing: String,
    pub fee: String,
    pub hooks: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Insertable)]
#[diesel(table_name = uniswap_v4_pools)]
pub struct NewUniswapV4Pool {
    pub network_id: i32,
    pub token_id: i64,
    pub pool_key: String,
    pub currency0: String,
    pub currency1: String,
    pub tick_spacing: String,
    pub fee: String,
    pub hooks: String,
}
