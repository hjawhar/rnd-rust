use borsh::BorshDeserialize;

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub struct PoolFees {
    pub trade_fee_numerator: u64,
    pub trade_fee_denominator: u64,
    pub protocol_trade_fee_numerator: u64,
    pub protocol_trade_fee_denominator: u64,
}

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub enum PoolType {
    Permissioned,
    Permissionless,
}

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub struct Bootstrapping {
    pub activation_point: u64,
    pub whitelisted_vault: solana_pubkey::Pubkey,
    pub pool_creator: solana_pubkey::Pubkey,
    pub activation_type: u8,
}

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub struct PartnerInfo {
    pub fee_numerator: u64,
    pub partner_authority: solana_pubkey::Pubkey,
    pub pending_fee_a: u64,
    pub pending_fee_b: u64,
}

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub struct TokenMultiplier {
    pub token_a_multiplier: u64,
    pub token_b_multiplier: u64,
    pub precision_factor: u8,
}

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub enum DepegType {
    None,
    Marinade,
    Lido,
    SplStake,
}

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub struct Depeg {
    pub base_virtual_price: u64,
    pub base_cache_updated: u64,
    pub depeg_type: DepegType,
}

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub enum CurveType {
    ConstantProduct,
    Stable {
        amp: u64,
        token_multiplier: TokenMultiplier,
        depeg: Depeg,
        last_amp_updated_timestamp: u64,
    },
}

#[derive(
    BorshDeserialize, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Clone, Hash,
)]
pub struct Padding {
    pub padding0: [u8; 6],
    pub padding1: [u64; 21],
    pub padding2: [u64; 21],
}

#[derive(BorshDeserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
// #[carbon(discriminator = "0xf19a6d0411b16dbc")]
pub struct MeteoraDAMMPool {
    pub lp_mint: solana_pubkey::Pubkey,
    pub token_a_mint: solana_pubkey::Pubkey,
    pub token_b_mint: solana_pubkey::Pubkey,
    pub a_vault: solana_pubkey::Pubkey,
    pub b_vault: solana_pubkey::Pubkey,
    pub a_vault_lp: solana_pubkey::Pubkey,
    pub b_vault_lp: solana_pubkey::Pubkey,
    pub a_vault_lp_bump: u8,
    pub enabled: bool,
    pub protocol_token_a_fee: solana_pubkey::Pubkey,
    pub protocol_token_b_fee: solana_pubkey::Pubkey,
    pub fee_last_updated_at: u64,
    pub padding0: [u8; 24],
    pub fees: PoolFees,
    pub pool_type: PoolType,
    pub stake: solana_pubkey::Pubkey,
    pub total_locked_lp: u64,
    pub bootstrapping: Bootstrapping,
    pub partner_info: PartnerInfo,
    pub padding: Padding,
    pub curve_type: CurveType,
}

#[derive(BorshDeserialize, Clone, Debug)]
// #[carbon(discriminator = "0x9b0caae01efacc82")]
pub struct Config {
    pub pool_fees: PoolFees,
    pub activation_duration: u64,
    pub vault_config_key: solana_pubkey::Pubkey,
    pub pool_creator_authority: solana_pubkey::Pubkey,
    pub activation_type: u8,
    pub partner_fee_numerator: u64,
    pub padding: [u8; 219],
}
