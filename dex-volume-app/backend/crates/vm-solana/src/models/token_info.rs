use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenInfo {
    pub supply: u64,
    pub decimals: u8,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
}
