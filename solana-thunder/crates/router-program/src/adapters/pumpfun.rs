use solana_program::{
    account_info::AccountInfo,
    instruction::{AccountMeta, Instruction},
    msg,
    program::invoke,
    program_error::ProgramError,
};

/// `buy_exact_quote_in` discriminator (from the on-chain pAMMBay IDL).
///
/// "Given a budget of spendable_quote_in, buy at least min_base_amount_out.
/// Fees will be deducted from spendable_quote_in." A true exact-in buy, so the
/// router needs no on-chain output precomputation; the final route-level
/// min_amount_out enforces slippage.
const PUMPFUN_BUY_EXACT_QUOTE_IN_DISC: [u8; 8] = [198, 46, 21, 82, 180, 217, 232, 112];
/// `sell` discriminator (exact-in by construction).
const PUMPFUN_SELL_DISC: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];

/// Buy adapter accounts layout (25 total; verified against mainnet buy tx
/// 2GzrE7HHWCZo8hDYXgrNrrEAU1BRdEi5Z7UFa47JKhNy6KaYQAwybtU5QYgUEFd41JrU8SkDj3VZBM53JHazV1m1):
///  [0]  dex_program_id
///  [1]  swap_authority_pubkey (user)
///  [2]  swap_source_token   (user quote ATA)
///  [3]  swap_destination_token (user base ATA)
///  [4]  pool
///  [5]  global_config
///  [6]  base_mint
///  [7]  quote_mint
///  [8]  pool_base_token_account
///  [9]  pool_quote_token_account
///  [10] protocol_fee_recipient
///  [11] protocol_fee_recipient_token_account
///  [12] base_token_program
///  [13] quote_token_program
///  [14] system_program
///  [15] associated_token_program
///  [16] event_authority
///  [17] coin_creator_vault_ata
///  [18] coin_creator_vault_authority
///  [19] global_volume_accumulator
///  [20] user_volume_accumulator
///  [21] fee_config
///  [22] fee_program
///  [23] buyback_fee_recipient (mandatory remaining account since 2025-09-01)
///  [24] buyback_fee_recipient_token_account
const BUY_ACCOUNTS_LEN: usize = 25;

/// Sell adapter accounts layout (23 total; the same list minus the two volume
/// accumulators, verified against mainnet sell tx
/// JqutDu6AMs18UacQCz653hXJZBGUrqrSdstz2rN2ncxKsT3NyMrWeo23kAtgN6ALqKSVs5zb9PimMZF7MNz79Kn):
///  [0..18] identical to buy layout [0..18], with [2] = user base ATA and
///          [3] = user quote ATA (direction flips source/dest)
///  [19] fee_config
///  [20] fee_program
///  [21] buyback_fee_recipient
///  [22] buyback_fee_recipient_token_account
const SELL_ACCOUNTS_LEN: usize = 23;

/// Buy: source = user quote ATA, dest = user base ATA.
/// Data: disc(8) + spendable_quote_in(8) + min_base_amount_out=1(8) + track_volume=false(1)
pub fn buy(accounts: &[AccountInfo], amount_in: u64) -> Result<(), ProgramError> {
    if accounts.len() < BUY_ACCOUNTS_LEN {
        msg!("Pumpfun buy: expected {} accounts, got {}", BUY_ACCOUNTS_LEN, accounts.len());
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let dex_program = accounts[0].key;

    let mut data = Vec::with_capacity(25);
    data.extend_from_slice(&PUMPFUN_BUY_EXACT_QUOTE_IN_DISC);
    data.extend_from_slice(&amount_in.to_le_bytes()); // spendable_quote_in
    data.extend_from_slice(&1u64.to_le_bytes());      // min_base_amount_out
    data.push(0u8);                                   // track_volume = false

    // CPI metas in IDL order (23) + 2 buyback remaining accounts.
    let metas = vec![
        AccountMeta::new(*accounts[4].key, false),           //  0 pool
        AccountMeta::new(*accounts[1].key, true),            //  1 user (writable, signer)
        AccountMeta::new_readonly(*accounts[5].key, false),  //  2 global_config
        AccountMeta::new_readonly(*accounts[6].key, false),  //  3 base_mint
        AccountMeta::new_readonly(*accounts[7].key, false),  //  4 quote_mint
        AccountMeta::new(*accounts[3].key, false),           //  5 user_base_token_account (dest)
        AccountMeta::new(*accounts[2].key, false),           //  6 user_quote_token_account (source)
        AccountMeta::new(*accounts[8].key, false),           //  7 pool_base_token_account
        AccountMeta::new(*accounts[9].key, false),           //  8 pool_quote_token_account
        AccountMeta::new_readonly(*accounts[10].key, false), //  9 protocol_fee_recipient
        AccountMeta::new(*accounts[11].key, false),          // 10 protocol_fee_recipient_token_account
        AccountMeta::new_readonly(*accounts[12].key, false), // 11 base_token_program
        AccountMeta::new_readonly(*accounts[13].key, false), // 12 quote_token_program
        AccountMeta::new_readonly(*accounts[14].key, false), // 13 system_program
        AccountMeta::new_readonly(*accounts[15].key, false), // 14 associated_token_program
        AccountMeta::new_readonly(*accounts[16].key, false), // 15 event_authority
        AccountMeta::new_readonly(*dex_program, false),      // 16 program (Anchor self-ref)
        AccountMeta::new(*accounts[17].key, false),          // 17 coin_creator_vault_ata
        AccountMeta::new_readonly(*accounts[18].key, false), // 18 coin_creator_vault_authority
        AccountMeta::new_readonly(*accounts[19].key, false), // 19 global_volume_accumulator
        AccountMeta::new(*accounts[20].key, false),          // 20 user_volume_accumulator
        AccountMeta::new_readonly(*accounts[21].key, false), // 21 fee_config
        AccountMeta::new_readonly(*accounts[22].key, false), // 22 fee_program
        AccountMeta::new_readonly(*accounts[23].key, false), // 23 buyback_fee_recipient
        AccountMeta::new(*accounts[24].key, false),          // 24 buyback_fee_recipient_token_account
    ];

    let ix = Instruction { program_id: *dex_program, accounts: metas, data };
    invoke(&ix, accounts)?;
    Ok(())
}

/// Sell: source = user base ATA, dest = user quote ATA.
/// Data: disc(8) + base_amount_in(8) + min_quote_amount_out=1(8)
pub fn sell(accounts: &[AccountInfo], amount_in: u64) -> Result<(), ProgramError> {
    if accounts.len() < SELL_ACCOUNTS_LEN {
        msg!("Pumpfun sell: expected {} accounts, got {}", SELL_ACCOUNTS_LEN, accounts.len());
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let dex_program = accounts[0].key;

    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&PUMPFUN_SELL_DISC);
    data.extend_from_slice(&amount_in.to_le_bytes()); // base_amount_in
    data.extend_from_slice(&1u64.to_le_bytes());      // min_quote_amount_out

    // CPI metas in IDL order (21) + 2 buyback remaining accounts.
    let metas = vec![
        AccountMeta::new(*accounts[4].key, false),           //  0 pool
        AccountMeta::new(*accounts[1].key, true),            //  1 user (writable, signer)
        AccountMeta::new_readonly(*accounts[5].key, false),  //  2 global_config
        AccountMeta::new_readonly(*accounts[6].key, false),  //  3 base_mint
        AccountMeta::new_readonly(*accounts[7].key, false),  //  4 quote_mint
        AccountMeta::new(*accounts[2].key, false),           //  5 user_base_token_account (source)
        AccountMeta::new(*accounts[3].key, false),           //  6 user_quote_token_account (dest)
        AccountMeta::new(*accounts[8].key, false),           //  7 pool_base_token_account
        AccountMeta::new(*accounts[9].key, false),           //  8 pool_quote_token_account
        AccountMeta::new_readonly(*accounts[10].key, false), //  9 protocol_fee_recipient
        AccountMeta::new(*accounts[11].key, false),          // 10 protocol_fee_recipient_token_account
        AccountMeta::new_readonly(*accounts[12].key, false), // 11 base_token_program
        AccountMeta::new_readonly(*accounts[13].key, false), // 12 quote_token_program
        AccountMeta::new_readonly(*accounts[14].key, false), // 13 system_program
        AccountMeta::new_readonly(*accounts[15].key, false), // 14 associated_token_program
        AccountMeta::new_readonly(*accounts[16].key, false), // 15 event_authority
        AccountMeta::new_readonly(*dex_program, false),      // 16 program (Anchor self-ref)
        AccountMeta::new(*accounts[17].key, false),          // 17 coin_creator_vault_ata
        AccountMeta::new_readonly(*accounts[18].key, false), // 18 coin_creator_vault_authority
        AccountMeta::new_readonly(*accounts[19].key, false), // 19 fee_config
        AccountMeta::new_readonly(*accounts[20].key, false), // 20 fee_program
        AccountMeta::new_readonly(*accounts[21].key, false), // 21 buyback_fee_recipient
        AccountMeta::new(*accounts[22].key, false),          // 22 buyback_fee_recipient_token_account
    ];

    let ix = Instruction { program_id: *dex_program, accounts: metas, data };
    invoke(&ix, accounts)?;
    Ok(())
}
