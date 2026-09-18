use std::sync::Arc;

use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey;

use crate::{
    markets::meteora_damm::models::meteora_dynamic_amm_vault_authority::VaultAuthority,
    utils::constants::METEORA_VAULT_PROGRAM,
};
use borsh::BorshDeserialize;

pub static VAULT_PREFIX: &str = "vault";
pub static TOKEN_VAULT_PREFIX: &str = "token_vault";
pub static LP_MINT_PREFIX: &str = "lp_mint";
pub static COLLATERAL_VAULT_PREFIX: &str = "collateral_vault";
pub static FEE_VAULT_PREFIX: &str = "fee_vault";
pub static SOLEND_OBLIGATION_PREFIX: &str = "solend_obligation";
pub static SOLEND_OBLIGATION_OWNER_PREFIX: &str = "solend_obligation_owner";
pub static APRICOT_USER_INFO_SIGNER_PREFIX: &str = "apricot_user_info_signer";
pub static VAULT_BASE_KEY: Pubkey = pubkey!("HWzXGcGHy4tcpYfaRDCyLNzXqBTv3E6BttpCH2vJxArv");

pub fn derive_vault_address(token_mint: Pubkey, base: Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[VAULT_PREFIX.as_ref(), token_mint.as_ref(), base.as_ref()],
        &Pubkey::from_str_const(METEORA_VAULT_PROGRAM),
    )
}

pub fn derive_token_vault_address(vault: Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[TOKEN_VAULT_PREFIX.as_ref(), vault.as_ref()],
        &Pubkey::from_str_const(METEORA_VAULT_PROGRAM),
    )
}

pub fn derive_strategy_address(vault: Pubkey, reserve: Pubkey, index: u8) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[vault.as_ref(), reserve.as_ref(), &[index]],
        &Pubkey::from_str_const(METEORA_VAULT_PROGRAM),
    )
}

pub fn derive_collateral_vault_address(strategy: Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[COLLATERAL_VAULT_PREFIX.as_ref(), strategy.as_ref()],
        &Pubkey::from_str_const(METEORA_VAULT_PROGRAM),
    )
}

pub fn derive_token_lp_mint(vault: Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[LP_MINT_PREFIX.as_ref(), vault.as_ref()],
        &Pubkey::from_str_const(METEORA_VAULT_PROGRAM),
    )
}

pub async fn get_vault_authority_account_data(
    rpc: Arc<RpcClient>,
    vault_address: String,
) -> Option<VaultAuthority> {
    let account = rpc
        .get_account_data(&Pubkey::from_str_const(&vault_address))
        .await;
    let account = match account {
        Ok(account) => account,
        Err(_) => return None,
    };

    if let Ok(vault_authority) = VaultAuthority::deserialize(&mut &account[8..]) {
        return Some(vault_authority);
    }

    None
}
