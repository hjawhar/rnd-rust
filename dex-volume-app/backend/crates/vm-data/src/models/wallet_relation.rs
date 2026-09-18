use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
// #[serde(untagged)]
pub enum BalanceType {
    WSOL,
    TOKENS,
    BASE,
    QUOTE,
    POOL
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletRelation {
    pub user_id: i32,
    pub project_id: i32,
    pub owner: String,
    pub address: String,
    pub balance_type: BalanceType,
    pub mint: String,
}

impl WalletRelation {
    pub fn to_string(&self) -> String {
        format!("{}_{}_{}", self.user_id, self.project_id, self.owner)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceUpdate {
    pub balance: f64,
    pub relation: WalletRelation,
}
