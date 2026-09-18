use solana_program::{
    account_info::AccountInfo,
    instruction::{AccountMeta, Instruction},
    msg,
    program::invoke,
    program_error::ProgramError,
};

/// Raydium CLMM swap_v2 discriminator.
const SWAP_V2_DISC: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];

/// Adapter accounts layout (15 fixed + 1..=3 tick arrays, all real accounts —
/// no sentinels; verified against mainnet swap_v2 tx
/// 36cjxY7fd7adG3SppazEjNH3gwwGAFxdhSTyVzh5RbGdTJb4mbuwwhsjP3NnS81BWENzeAvS9puCawUnr3GWPrVo):
///  [0]  dex_program_id
///  [1]  swap_authority_pubkey
///  [2]  swap_source_token
///  [3]  swap_destination_token
///  [4]  amm_config_id
///  [5]  pool_id
///  [6]  input_vault
///  [7]  output_vault
///  [8]  observation_id
///  [9]  token_program
///  [10] token_program_2022
///  [11] memo_program
///  [12] input_vault_mint
///  [13] output_vault_mint
///  [14] ex_bitmap (PDA ["pool_tick_array_bitmap_extension", pool])
///  [15..] tick arrays in swap order (1..=3, all writable)
const FIXED_ACCOUNTS: usize = 15;

pub fn swap(accounts: &[AccountInfo], amount_in: u64) -> Result<(), ProgramError> {
    if accounts.len() < FIXED_ACCOUNTS + 1 {
        msg!(
            "CLMM: expected at least {} accounts, got {}",
            FIXED_ACCOUNTS + 1,
            accounts.len()
        );
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let dex_program = accounts[0].key;

    // Data: disc(8) + amount_in(8) + threshold=1(8) + sqrt_price_limit=0(16) + is_base_input=1(1) = 41 bytes
    let mut data = Vec::with_capacity(41);
    data.extend_from_slice(&SWAP_V2_DISC);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&1u64.to_le_bytes());        // threshold (min_out)
    data.extend_from_slice(&0u128.to_le_bytes());       // sqrt_price_limit_x64 = 0
    data.push(1u8);                                      // is_base_input = true

    // CPI metas — swap_v2 ordering, then variable trailing tick arrays.
    let mut metas = Vec::with_capacity(accounts.len() - 1);
    metas.push(AccountMeta::new(*accounts[1].key, true));             //  0 swap_authority (writable, signer)
    metas.push(AccountMeta::new_readonly(*accounts[4].key, false));   //  1 amm_config_id
    metas.push(AccountMeta::new(*accounts[5].key, false));            //  2 pool_id
    metas.push(AccountMeta::new(*accounts[2].key, false));            //  3 swap_source_token
    metas.push(AccountMeta::new(*accounts[3].key, false));            //  4 swap_destination_token
    metas.push(AccountMeta::new(*accounts[6].key, false));            //  5 input_vault
    metas.push(AccountMeta::new(*accounts[7].key, false));            //  6 output_vault
    metas.push(AccountMeta::new(*accounts[8].key, false));            //  7 observation_id
    metas.push(AccountMeta::new_readonly(*accounts[9].key, false));   //  8 token_program
    metas.push(AccountMeta::new_readonly(*accounts[10].key, false));  //  9 token_program_2022
    metas.push(AccountMeta::new_readonly(*accounts[11].key, false));  // 10 memo_program
    metas.push(AccountMeta::new_readonly(*accounts[12].key, false));  // 11 input_vault_mint
    metas.push(AccountMeta::new_readonly(*accounts[13].key, false));  // 12 output_vault_mint
    metas.push(AccountMeta::new(*accounts[14].key, false));           // 13 ex_bitmap

    // All trailing accounts are tick arrays, forwarded writable in order.
    for tick_array in &accounts[FIXED_ACCOUNTS..] {
        metas.push(AccountMeta::new(*tick_array.key, false));
    }

    let ix = Instruction { program_id: *dex_program, accounts: metas, data };
    invoke(&ix, accounts)?;
    Ok(())
}
