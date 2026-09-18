use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletInfo {
    pub address: String,
    pub tokens: f64,
    pub sol: f64,
}
