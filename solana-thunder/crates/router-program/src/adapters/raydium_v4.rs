use solana_program::{
    account_info::AccountInfo,
    instruction::{AccountMeta, Instruction},
    msg,
    program::invoke,
    program_error::ProgramError,
};

/// Raydium AMM V4 `SwapBaseInV2` discriminator (single byte = 16).
///
/// V2 is the orderbook-free swap added when Raydium removed the OpenBook
/// integration. It takes only 8 accounts (no market accounts at all) and is
/// what Jupiter routes through on mainnet today (e.g. tx
/// 5VakSBagfcCh1pGBaAE8zVNPbrXpb6b9nmyCPCotNB9dPxyyaCW2WYUDgwbz9MirV4yMn7uVjevJmCEEJYYnSXRA).
/// It requires `AmmStatus::orderbook_permission() == false`, which is exactly
/// the `status == 6` (SwapOnly) set our engine treats as active.
const SWAP_BASE_IN_V2_DISC: u8 = 16;

/// Adapter accounts layout (9 total):
///  [0]  dex_program_id
///  [1]  swap_authority_pubkey
///  [2]  swap_source_token
///  [3]  swap_destination_token
///  [4]  token_program
///  [5]  amm_id
///  [6]  amm_authority
///  [7]  pool_coin_token_account
///  [8]  pool_pc_token_account
const ACCOUNTS_LEN: usize = 9;

pub fn swap(accounts: &[AccountInfo], amount_in: u64) -> Result<(), ProgramError> {
    if accounts.len() < ACCOUNTS_LEN {
        msg!("Raydium V4: expected {} accounts, got {}", ACCOUNTS_LEN, accounts.len());
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let dex_program = accounts[0].key;

    // Data: disc=16u8(1) + amount_in(8) + min_out=1(8) = 17 bytes
    let mut data = Vec::with_capacity(17);
    data.push(SWAP_BASE_IN_V2_DISC);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&1u64.to_le_bytes());

    // CPI metas — SwapBaseInV2 ordering (8 metas)
    let metas = vec![
        AccountMeta::new_readonly(*accounts[4].key, false),  //  0 token_program
        AccountMeta::new(*accounts[5].key, false),           //  1 amm_id
        AccountMeta::new_readonly(*accounts[6].key, false),  //  2 amm_authority
        AccountMeta::new(*accounts[7].key, false),           //  3 pool_coin_token_account
        AccountMeta::new(*accounts[8].key, false),           //  4 pool_pc_token_account
        AccountMeta::new(*accounts[2].key, false),           //  5 swap_source_token
        AccountMeta::new(*accounts[3].key, false),           //  6 swap_destination_token
        AccountMeta::new_readonly(*accounts[1].key, true),   //  7 swap_authority (signer)
    ];

    let ix = Instruction { program_id: *dex_program, accounts: metas, data };
    invoke(&ix, accounts)?;
    Ok(())
}
