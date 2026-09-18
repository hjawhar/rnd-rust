use borsh::BorshDeserialize;

#[derive(BorshDeserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AmmConfig {
    pub bump_seed: u8,
    pub index: u16,
    pub owner: solana_pubkey::Pubkey,
    pub protocol_fee_rate: u32,
    pub trade_fee_rate: u32,
    pub tick_spacing: u16,
    pub fund_fee_rate: u32,
    pub padding_u32: u32,
    pub fund_owner: solana_pubkey::Pubkey,
    pub padding: [u64; 3],
}
