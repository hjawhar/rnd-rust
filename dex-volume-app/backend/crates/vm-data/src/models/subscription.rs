use std::time::SystemTime;

use crate::db::schema::subscriptions;
use bigdecimal::BigDecimal;
use diesel::{
    Selectable,
    prelude::{Insertable, Queryable},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Queryable, Selectable)]
#[diesel(table_name = subscriptions)]
pub struct Subscription {
    pub id: i32,
    pub project_id: i32,
    pub monthly_rate: BigDecimal,
    pub currency: String,
    pub started_at: SystemTime,
    pub next_payment_due: SystemTime,
    pub status: String,
    pub notes: Option<String>,
    pub created_at: SystemTime,
}

#[derive(Debug, Deserialize, Insertable)]
#[diesel(table_name = subscriptions)]
pub struct NewSubscription {
    pub project_id: i32,
    pub monthly_rate: BigDecimal,
    pub currency: String,
    pub started_at: SystemTime,
    pub next_payment_due: SystemTime,
    pub status: String,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSubscriptionPayload {
    pub monthly_rate: f64,
    #[serde(default = "default_currency")]
    pub currency: String,
    pub next_payment_due: String,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSubscriptionPayload {
    pub monthly_rate: Option<f64>,
    pub currency: Option<String>,
    pub next_payment_due: Option<String>,
    pub status: Option<String>,
    pub notes: Option<String>,
}

fn default_currency() -> String {
    "USD".to_string()
}
