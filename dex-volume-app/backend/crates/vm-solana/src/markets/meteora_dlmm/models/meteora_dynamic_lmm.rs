use borsh::BorshDeserialize;

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub struct StaticParameters {
    pub base_factor: u16,
    pub filter_period: u16,
    pub decay_period: u16,
    pub reduction_factor: u16,
    pub variable_fee_control: u32,
    pub max_volatility_accumulator: u32,
    pub min_bin_id: i32,
    pub max_bin_id: i32,
    pub protocol_share: u16,
    pub base_fee_power_factor: u8,
    pub padding: [u8; 5],
}

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub struct VariableParameters {
    pub volatility_accumulator: u32,
    pub volatility_reference: u32,
    pub index_reference: i32,
    pub padding: [u8; 4],
    pub last_update_timestamp: i64,
    pub padding1: [u8; 8],
}

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub struct ProtocolFee {
    pub amount_x: u64,
    pub amount_y: u64,
}

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub struct RewardInfo {
    pub mint: solana_pubkey::Pubkey,
    pub vault: solana_pubkey::Pubkey,
    pub funder: solana_pubkey::Pubkey,
    pub reward_duration: u64,
    pub reward_duration_end: u64,
    pub reward_rate: u128,
    pub last_update_time: u64,
    pub cumulative_seconds_with_empty_liquidity_reward: u64,
}

#[derive(BorshDeserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
// #[carbon(discriminator = "0x210b3162b565b10d")]
pub struct MeteoraDLMMPool {
    pub parameters: StaticParameters,
    pub v_parameters: VariableParameters,
    pub bump_seed: [u8; 1],
    pub bin_step_seed: [u8; 2],
    pub pair_type: u8,
    pub active_id: i32,
    pub bin_step: u16,
    pub status: u8,
    pub require_base_factor_seed: u8,
    pub base_factor_seed: [u8; 2],
    pub activation_type: u8,
    pub creator_pool_on_off_control: u8,
    pub token_x_mint: solana_pubkey::Pubkey,
    pub token_y_mint: solana_pubkey::Pubkey,
    pub reserve_x: solana_pubkey::Pubkey,
    pub reserve_y: solana_pubkey::Pubkey,
    pub protocol_fee: ProtocolFee,
    pub padding1: [u8; 32],
    pub reward_infos: [RewardInfo; 2],
    pub oracle: solana_pubkey::Pubkey,
    pub bin_array_bitmap: [u64; 16],
    pub last_updated_at: i64,
    pub padding2: [u8; 32],
    pub pre_activation_swap_address: solana_pubkey::Pubkey,
    pub base_key: solana_pubkey::Pubkey,
    pub activation_point: u64,
    pub pre_activation_duration: u64,
    pub padding3: [u8; 8],
    pub padding4: u64,
    pub creator: solana_pubkey::Pubkey,
    pub token_mint_x_program_flag: u8,
    pub token_mint_y_program_flag: u8,
    pub reserved: [u8; 22],
}
