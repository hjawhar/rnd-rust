use serde::{Deserialize, Serialize};

use crate::models::project::Project;

fn default_jito_tip() -> f64 {
    0.00002
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessWalletInfo {
    pub private_key: String,
    pub address: String,
    pub wallet_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessTask {
    pub project: Project,
    pub wallets: Vec<ProcessWalletInfo>,
    pub last_active: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessBuy {
    pub market_pair: String,
    pub project_id: i32,
    pub wallet_id: i32,
    pub token: String,
    pub pool: String,
    pub value: f64,
    pub tokens: f64,
    pub private_key: String,
    pub address: String,
    pub sol_price: f64,
    /// Maximum allowed market impact in basis points (100 = 1%). None = no limit.
    #[serde(default)]
    pub max_market_impact_bps: Option<u16>,
    /// USDC value of this trade for daily volume tracking (set by volume_maker)
    #[serde(default)]
    pub trade_volume_usdc: f64,
    /// Jito tip in SOL (configurable per-project, default 0.00002)
    #[serde(default = "default_jito_tip")]
    pub jito_tip: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSell {
    pub market_pair: String,
    pub project_id: i32,
    pub wallet_id: i32,
    pub token: String,
    pub pool: String,
    pub value: f64,
    pub tokens: f64,
    pub private_key: String,
    pub address: String,
    pub sol_price: f64,
    /// Maximum allowed market impact in basis points (100 = 1%). None = no limit.
    #[serde(default)]
    pub max_market_impact_bps: Option<u16>,
    /// USDC value of this trade for daily volume tracking (set by volume_maker)
    #[serde(default)]
    pub trade_volume_usdc: f64,
    /// Jito tip in SOL (configurable per-project, default 0.00002)
    #[serde(default = "default_jito_tip")]
    pub jito_tip: f64,
}

impl ProcessTask {
    pub fn is_wallet_running(&self, wallet_id: i32) -> bool {
        self
            .wallets
            .iter()
            .any(|x| x.wallet_id == wallet_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCollectSOL {
    pub senders_pks: Vec<String>,
    pub recipient_pk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCollectTokens {
    pub token: String,
    pub senders_pks: Vec<String>,
    pub recipient_pk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDisperseSOL {
    pub sender_pk: String,
    pub recipients_pubkeys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDisperseTokens {
    pub token: String,
    pub sender_pk: String,
    pub recipients_pubkeys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessBundleBuySell {
    pub market_pair: String,          // JSON-serialized MarketPair
    pub project_id: i32,
    pub token: String,
    pub pool: String,
    // Buy side
    pub buy_wallet_id: i32,
    pub buy_private_key: String,
    pub buy_address: String,
    pub buy_value: f64,               // SOL to spend
    pub buy_min_tokens: f64,          // tokens_out * 0.80 (slippage)
    // Sell side
    pub sell_wallet_id: i32,
    pub sell_private_key: String,
    pub sell_address: String,
    pub sell_min_value: f64,          // send_value * 0.80 (slippage)
    pub sell_tokens: f64,             // tokens_out (MATCHES buy expected output)
    // Shared
    pub sol_price: f64,
    #[serde(default)]
    pub max_market_impact_bps: Option<u16>,
    pub trade_volume_usdc: f64,       // per-leg USDC value
    /// Jito tip in SOL (configurable per-project, default 0.00002)
    #[serde(default = "default_jito_tip")]
    pub jito_tip: f64,
    /// Token decimals (e.g. 6 for USDC, 9 for most SPL tokens)
    #[serde(default = "default_token_decimals")]
    pub token_decimals: u8,
}

fn default_token_decimals() -> u8 {
    6
}

// ── EVM task types ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmProcessTask {
    pub project: Project,
    pub wallets: Vec<ProcessWalletInfo>,
    pub last_active: u128,
}

impl EvmProcessTask {
    pub fn is_wallet_running(&self, wallet_id: i32) -> bool {
        self.wallets.iter().any(|x| x.wallet_id == wallet_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmProcessBuy {
    pub project_id: i32,
    pub wallet_id: i32,
    pub user_id: i32,
    pub token: String,
    pub pool: String,
    pub value: f64,
    pub tokens: f64,
    pub private_key: String,
    pub address: String,
    pub eth_price: f64,
    pub network: String,
    /// JSON-serialized CustomToken
    #[serde(default)]
    pub token_info: String,
    /// JSON-serialized CustomPool
    #[serde(default)]
    pub pool_info: String,
    #[serde(default)]
    pub trade_volume_usdc: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmProcessSell {
    pub project_id: i32,
    pub wallet_id: i32,
    pub user_id: i32,
    pub token: String,
    pub pool: String,
    pub value: f64,
    pub tokens: f64,
    pub private_key: String,
    pub address: String,
    pub eth_price: f64,
    pub network: String,
    /// JSON-serialized CustomToken
    #[serde(default)]
    pub token_info: String,
    /// JSON-serialized CustomPool
    #[serde(default)]
    pub pool_info: String,
    #[serde(default)]
    pub trade_volume_usdc: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCollectETH {
    pub project_id: i32,
    pub user_id: i32,
    pub network: String,
    pub senders_pks: Vec<String>,
    pub recipient_pk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCollectEvmTokens {
    pub project_id: i32,
    pub user_id: i32,
    pub network: String,
    pub decimals: i32,
    pub token: String,
    pub senders_pks: Vec<String>,
    pub recipient_pk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDisperseETH {
    pub project_id: i32,
    pub user_id: i32,
    pub network: String,
    pub sender_pk: String,
    pub recipients_pubkeys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDisperseEvmTokens {
    pub project_id: i32,
    pub user_id: i32,
    pub network: String,
    pub decimals: i32,
    pub token: String,
    pub sender_pk: String,
    pub recipients_pubkeys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmProcessBundleBuySell {
    pub project_id: i32,
    pub user_id: i32,
    pub token: String,
    pub pool: String,
    // Single wallet for both legs (same as Solana bundle pattern)
    pub wallet_id: i32,
    pub private_key: String,
    pub address: String,
    // Buy side (ExactOut: get exact tokens, spend at most max ETH)
    pub buy_exact_tokens: f64,
    pub buy_max_eth: f64,
    // Sell side (ExactIn: sell exact tokens, get at least min ETH)
    pub sell_tokens: f64,
    pub sell_min_eth: f64,
    // Shared
    pub eth_price: f64,
    pub network: String,
    /// JSON-serialized CustomToken
    #[serde(default)]
    pub token_info: String,
    /// JSON-serialized CustomPool
    #[serde(default)]
    pub pool_info: String,
    #[serde(default)]
    pub trade_volume_usdc: f64,
}
