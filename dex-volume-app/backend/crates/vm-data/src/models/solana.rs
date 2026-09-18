use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectWalletInfo {
    pub id: i32,
    pub main: bool,
    pub address: String,
    pub native_balance: f64,
    pub token_balance: f64,
    pub native_usdc: f64,
    pub token_usdc: f64,
    pub total_usdc: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PortfolioSummary {
    pub total_native: f64,
    pub total_tokens: f64,
    pub total_native_usdc: f64,
    pub total_token_usdc: f64,
    pub total_usdc: f64,
    pub native_price: f64,
    pub token_price: f64,
    pub token_price_usdc: f64,
    pub daily_volume_usdc: f64,
    pub daily_volume_target_usdc: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WalletsFinancialsResponse {
    pub project_id: i32,
    pub wallets: Vec<ProjectWalletInfo>,
    pub summary: PortfolioSummary,
}

impl PortfolioSummary {
    pub fn from_wallets(
        wallets: &[ProjectWalletInfo],
        native_price: f64,
        token_price_usdc: f64,
        daily_volume_usdc: f64,
        daily_volume_target_usdc: f64,
    ) -> Self {
        let total_native: f64 = wallets.iter().map(|w| w.native_balance).sum();
        let total_tokens: f64 = wallets.iter().map(|w| w.token_balance).sum();
        let total_native_usdc = total_native * native_price;
        let total_token_usdc = total_tokens * token_price_usdc;
        let total_usdc = total_native_usdc + total_token_usdc;

        Self {
            total_native,
            total_tokens,
            total_native_usdc,
            total_token_usdc,
            total_usdc,
            native_price,
            token_price: token_price_usdc / native_price.max(f64::EPSILON),
            token_price_usdc,
            daily_volume_usdc,
            daily_volume_target_usdc,
        }
    }
}
