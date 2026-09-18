use std::time::SystemTime;

use crate::db::schema::transactions;
use bigdecimal::BigDecimal;
use diesel::{
    QueryableByName, Selectable,
    prelude::{Insertable, Queryable},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Insertable, Queryable, Selectable)]
pub struct Transaction {
    pub id: i32,
    pub project_id: i32,
    pub slot: i64,
    pub sol_price: BigDecimal,
    pub value: BigDecimal,
    pub tokens: BigDecimal,
    pub address: String,
    pub tx_hash: String,
    pub tx_type: String,
    pub token_in: String,
    pub token_out: String,
    pub date_added: SystemTime,
}

#[derive(Debug, Serialize, Deserialize, Clone, Insertable, Queryable, Selectable)]
#[diesel(table_name = transactions)]
pub struct NewTransaction {
    pub slot: i64,
    pub project_id: i32,
    pub sol_price: BigDecimal,
    pub value: BigDecimal,
    pub tokens: BigDecimal,
    pub address: String,
    pub tx_hash: String,
    pub tx_type: String,
    pub token_in: String,
    pub token_out: String,
    pub date_added: SystemTime,
}

use diesel::sql_types::Decimal;
#[derive(Clone, Debug, QueryableByName)]
pub struct TransactionSummaryOverall {
    #[diesel(sql_type = Decimal)]
    pub value: BigDecimal,
    #[diesel(sql_type = Decimal)]
    pub total: BigDecimal,
}

#[derive(Clone, Debug, QueryableByName)]
pub struct TransactionSummaryBuy {
    #[diesel(sql_type = Decimal)]
    pub value: BigDecimal,
    #[diesel(sql_type = Decimal)]
    pub total: BigDecimal,
    #[diesel(sql_type = Decimal)]
    pub tokens: BigDecimal,
}

#[derive(Clone, Debug, QueryableByName)]
pub struct TransactionSummarySell {
    #[diesel(sql_type = Decimal)]
    pub value: BigDecimal,
    #[diesel(sql_type = Decimal)]
    pub total: BigDecimal,
    #[diesel(sql_type = Decimal)]
    pub tokens: BigDecimal,
}
