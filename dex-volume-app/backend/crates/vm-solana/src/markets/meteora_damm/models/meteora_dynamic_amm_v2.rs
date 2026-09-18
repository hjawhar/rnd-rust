use borsh::BorshDeserialize;

#[derive(BorshDeserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BaseFee {
    pub cliff_fee_numerator: u64,
    pub fee_scheduler_mode: u8,
    pub padding_0: [u8; 5],
    pub number_of_period: u16,
    pub period_frequency: u64,
    pub reduction_factor: u64,
    pub padding_1: u64,
}

#[derive(BorshDeserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PoolMetrics {
    pub total_lp_a_fee: u128,
    pub total_lp_b_fee: u128,
    pub total_protocol_a_fee: u64,
    pub total_protocol_b_fee: u64,
    pub total_partner_a_fee: u64,
    pub total_partner_b_fee: u64,
    pub total_position: u64,
    pub padding: u64,
}

#[derive(BorshDeserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DynamicFee {
    pub initialized: u8, // 0, ignore for dynamic fee
    pub padding: [u8; 7],
    pub max_volatility_accumulator: u32,
    pub variable_fee_control: u32,
    pub bin_step: u16,
    pub filter_period: u16,
    pub decay_period: u16,
    pub reduction_factor: u16,
    pub last_update_timestamp: u64,
    pub bin_step_u128: u128,
    pub sqrt_price_reference: u128, // reference sqrt price
    pub volatility_accumulator: u128,
    pub volatility_reference: u128, // decayed volatility accumulator
}

// working
#[derive(BorshDeserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PoolFees {
    pub base_fee: BaseFee,
    pub protocol_fee_percent: u8,
    pub partner_fee_percent: u8,
    pub referral_fee_percent: u8,
    pub padding_0: [u8; 5],
    pub dynamic_fee: DynamicFee,
    pub padding_1: [u64; 2],
}

#[derive(BorshDeserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RewardInfo {
    pub initialized: u8,
    pub reward_token_flag: u8,
    pub _padding_0: [u8; 6],
    pub _padding_1: [u8; 8],
    pub mint: solana_pubkey::Pubkey,
    pub vault: solana_pubkey::Pubkey,
    pub funder: solana_pubkey::Pubkey,
    pub reward_duration: u64,
    pub reward_duration_end: u64,
    pub reward_rate: u128,
    pub reward_per_token_stored: [u8; 32],
    pub last_update_time: u64,
    pub cumulative_seconds_with_empty_liquidity_reward: u64,
}

#[derive(BorshDeserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MeteoraDAMMV2Pool {
    pub pool_fees: PoolFees,
    pub token_a_mint: solana_pubkey::Pubkey,
    pub token_b_mint: solana_pubkey::Pubkey,
    pub token_a_vault: solana_pubkey::Pubkey,
    pub token_b_vault: solana_pubkey::Pubkey,
    pub whitelisted_vault: solana_pubkey::Pubkey,
    pub partner: solana_pubkey::Pubkey,
    pub liquidity: u128,
    pub _padding: u128,
    pub protocol_a_fee: u64,
    pub protocol_b_fee: u64,
    pub partner_a_fee: u64,
    pub partner_b_fee: u64,
    pub sqrt_min_price: u128,
    pub sqrt_max_price: u128,
    pub sqrt_price: u128,
    pub activation_point: u64,
    pub activation_type: u8,
    pub pool_status: u8,
    pub token_a_flag: u8,
    pub token_b_flag: u8,
    pub collect_fee_mode: u8,
    pub pool_type: u8,
    pub _padding_0: [u8; 2],
    pub fee_a_per_liquidity: [u8; 32],
    pub fee_b_per_liquidity: [u8; 32],
    pub permanent_lock_liquidity: u128,
    pub metrics: PoolMetrics,
    pub _padding_1: [u64; 10],
    pub reward_infos: [RewardInfo; 2],
}
