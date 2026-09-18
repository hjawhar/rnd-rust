use borsh::BorshDeserialize;

#[derive(
    Debug, BorshDeserialize, serde::Deserialize, serde::Serialize, PartialEq, Eq, Clone, Hash,
)]
pub struct RewardInfo {
    /// Reward state
    pub reward_state: u8,
    /// Reward open time
    pub open_time: u64,
    /// Reward end time
    pub end_time: u64,
    /// Reward last update time
    pub last_update_time: u64,
    /// Q64.64 number indicates how many tokens per second are earned per unit of liquidity.
    pub emissions_per_second_x64: u128,
    /// The total amount of reward emissioned
    pub reward_total_emissioned: u64,
    /// The total amount of claimed reward
    pub reward_claimed: u64,
    /// Reward token mint.
    pub token_mint: solana_pubkey::Pubkey,
    /// Reward vault token account.
    pub token_vault: solana_pubkey::Pubkey,
    /// The owner that has permission to set reward param
    pub authority: solana_pubkey::Pubkey,
    /// Q64.64 number that tracks the total tokens earned per unit of liquidity since the reward
    /// emissions were turned on.
    pub reward_growth_global_x64: u128,
}

#[derive(
    Debug, BorshDeserialize, serde::Deserialize, serde::Serialize, PartialEq, Eq, Clone, Hash,
)]
pub struct RaydiumCLMMPool {
    pub bump: [u8; 1],
    pub amm_config: solana_pubkey::Pubkey,
    pub owner: solana_pubkey::Pubkey,
    pub token_mint_0: solana_pubkey::Pubkey,
    pub token_mint_1: solana_pubkey::Pubkey,
    pub token_vault_0: solana_pubkey::Pubkey,
    pub token_vault_1: solana_pubkey::Pubkey,
    pub observation_key: solana_pubkey::Pubkey,
    pub mint_decimals_0: u8,
    pub mint_decimals_1: u8,
    pub tick_spacing: u16,
    pub liquidity: u128,
    pub sqrt_price_x64: u128,
    pub tick_current: i32,
    pub padding3: u16,
    pub padding4: u16,
    pub fee_growth_global_0_x64: u128,
    pub fee_growth_global_1_x64: u128,
    pub protocol_fees_token_0: u64,
    pub protocol_fees_token_1: u64,
    pub swap_in_amount_token_0: u128,
    pub swap_out_amount_token_1: u128,
    pub swap_in_amount_token_1: u128,
    pub swap_out_amount_token_0: u128,
    pub status: u8,
    pub padding: [u8; 7],
    pub reward_infos: [RewardInfo; 3],
    pub tick_array_bitmap: [u64; 16],
    pub total_fees_token_0: u64,
    pub total_fees_claimed_token_0: u64,
    pub total_fees_token_1: u64,
    pub total_fees_claimed_token_1: u64,
    pub fund_fees_token_0: u64,
    pub fund_fees_token_1: u64,
    pub open_time: u64,
    pub recent_epoch: u64,
    pub padding1: [u64; 24],
    pub padding2: [u64; 32],
}
