use borsh::BorshDeserialize;

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub struct VaultBumps {
    pub vault_bump: u8,
    pub token_vault_bump: u8,
}

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub struct LockedProfitTracker {
    pub last_updated_locked_profit: u64,
    pub last_report: u64,
    pub locked_profile_degradation: u8,
}

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub struct VaultAuthority {
    pub enabled: [u8; 1],
    pub bumps: VaultBumps,
    pub total_amount: u64,
    pub token_vault: solana_pubkey::Pubkey,
    pub fee_vault: solana_pubkey::Pubkey,
    pub token_mint: solana_pubkey::Pubkey,
    pub lp_mint: solana_pubkey::Pubkey,
    pub strategies: [solana_pubkey::Pubkey; 30],
    pub base: solana_pubkey::Pubkey,
    pub admin: solana_pubkey::Pubkey,
    pub operator: solana_pubkey::Pubkey,
    pub locked_profit_tracker: LockedProfitTracker,
}
