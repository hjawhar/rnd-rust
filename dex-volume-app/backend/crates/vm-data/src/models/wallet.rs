use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use diesel::{
    Selectable,
    prelude::{Insertable, Queryable},
};

use crate::db::schema::wallets;

// Database

#[derive(Debug, Clone, Serialize, Deserialize, Insertable, Selectable)]
#[diesel(table_name = wallets)]
pub struct NewWallet {
    pub project_id: i32,
    pub address: String,
    pub pk: String,
    pub is_main: bool,
    pub date_added: SystemTime,
}

#[derive(Deserialize, Serialize, Queryable, Debug, Selectable, Clone)]
pub struct Wallet {
    pub id: i32,
    pub project_id: i32,
    pub address: String,
    pub pk: String,
    pub is_main: bool,
    pub date_added: SystemTime,
}

// Payloads

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddWalletsPayload {
    pub pks: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateWalletPayload {
    pub value: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewWalletsPayload {
    pub ids: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteWalletsPayload {
    pub ids: Vec<i32>,
}

// Stored Wallet data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredWallet {
    pub user_id: i32,
    pub wallet: Wallet,
}
