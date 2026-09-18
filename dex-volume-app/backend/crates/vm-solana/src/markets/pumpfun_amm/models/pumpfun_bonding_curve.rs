use borsh::BorshDeserialize;

#[derive(BorshDeserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PumpfunBondingCurve {
    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub real_sol_reserves: u64,
    pub token_total_supply: u64,
    pub complete: bool,
    pub creator: solana_pubkey::Pubkey,
}
