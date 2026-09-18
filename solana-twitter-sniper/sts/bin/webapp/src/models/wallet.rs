use serde::{Deserialize, Serialize};
use validator::Validate;

use diesel::{
    prelude::{Insertable, Queryable},
    Selectable,
};

use crate::db::schema::wallets;

// Database

#[derive(Debug, Clone, Deserialize, Insertable, Selectable)]
#[diesel(table_name = wallets)]
pub struct NewWallet {
    pub user_id: i32,
    pub pk: String,
    pub address: String,
    pub name: String,
    pub comments: Option<String>,
}

#[derive(Deserialize, Serialize, Queryable, Debug, Selectable, Clone)]
pub struct Wallet {
    pub id: i32,
    pub user_id: i32,
    pub address: String,
    pub name: String,
    pub comments: Option<String>,
    pub pk: String,
    pub nonce_account_address: Option<String>,
}

#[derive(Deserialize, Serialize, Queryable, Debug, Selectable, Clone)]
#[diesel(table_name = wallets)]
pub struct WalletRetrieved {
    pub id: i32,
    pub address: String,
    pub name: String,
    pub comments: Option<String>,
    pub nonce_account_address: Option<String>,
}

// Responses

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletResponse {
    pub id: i32,
    pub address: String,
    pub name: String,
    pub comments: Option<String>,
    pub balance: f64,
    pub nonce_account_address: Option<String>,
}

// Payloads

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddWalletsPayload {
    pub pks: String,
    pub name: String,
    pub comments: Option<String>,
}

#[derive(Debug, Validate, Clone, Serialize, Deserialize)]
pub struct GenerateWalletPayload {
    pub value: u32,
    pub name: String,
    pub comments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewWalletsPayload {
    pub ids: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteWalletsPayload {
    pub ids: Vec<i32>,
}
