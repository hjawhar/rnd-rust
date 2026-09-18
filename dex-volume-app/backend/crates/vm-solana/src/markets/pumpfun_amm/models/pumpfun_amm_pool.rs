use borsh::BorshDeserialize;

use crate::markets::pumpfun_amm::models::pumpfun_bonding_curve::PumpfunBondingCurve;

#[derive(BorshDeserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PumpfunAmmPool {
    pub pool_bump: u8,
    pub index: u16,
    pub creator: solana_pubkey::Pubkey,
    pub base_mint: solana_pubkey::Pubkey,
    pub quote_mint: solana_pubkey::Pubkey,
    pub lp_mint: solana_pubkey::Pubkey,
    pub pool_base_token_account: solana_pubkey::Pubkey,
    pub pool_quote_token_account: solana_pubkey::Pubkey,
    pub lp_supply: u64,
    pub coin_creator: solana_pubkey::Pubkey,
    pub is_mayhem_mode: bool,
    pub is_cashback_coin: bool,
    /// Bonding curve data — fetched separately, not part of pool account borsh layout
    #[borsh(skip)]
    #[serde(default)]
    pub bonding_curve: Option<PumpfunBondingCurve>,
}
