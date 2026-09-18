use std::time::SystemTime;

use crate::db::schema::payments;
use bigdecimal::BigDecimal;
use diesel::{
    Selectable,
    prelude::{Insertable, Queryable},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Queryable, Selectable)]
#[diesel(table_name = payments)]
pub struct Payment {
    pub id: i32,
    pub subscription_id: i32,
    pub amount: BigDecimal,
    pub currency: String,
    pub paid_at: SystemTime,
    pub recorded_by: i32,
    pub notes: Option<String>,
    pub created_at: SystemTime,
}

#[derive(Debug, Deserialize, Insertable)]
#[diesel(table_name = payments)]
pub struct NewPayment {
    pub subscription_id: i32,
    pub amount: BigDecimal,
    pub currency: String,
    pub paid_at: SystemTime,
    pub recorded_by: i32,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePaymentPayload {
    pub amount: f64,
    #[serde(default = "default_currency")]
    pub currency: String,
    pub paid_at: Option<String>,
    pub notes: Option<String>,
}

fn default_currency() -> String {
    "USD".to_string()
}
