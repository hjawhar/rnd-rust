use solana_program::{
    account_info::AccountInfo,
    instruction::{AccountMeta, Instruction},
    msg,
    program::invoke,
    program_error::ProgramError,
};

/// swap2 discriminator.
const SWAP2_DISC: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];

/// Adapter accounts layout (16 fixed + 1..=3 bin arrays, no sentinels):
///  [0]  dex_program_id
///  [1]  swap_authority_pubkey
///  [2]  swap_source_token
///  [3]  swap_destination_token
///  [4]  lb_pair
///  [5]  bin_array_bitmap_extension (writable when present; dex program id = None)
///  [6]  reserve_x
///  [7]  reserve_y
///  [8]  token_x_mint
///  [9]  token_y_mint
///  [10] oracle
///  [11] host_fee_in
///  [12] token_x_program
///  [13] token_y_program
///  [14] memo_program
///  [15] event_authority
///  [16..] bin arrays in swap order (1..=3, all writable)
const FIXED_ACCOUNTS: usize = 16;

pub fn swap(accounts: &[AccountInfo], amount_in: u64) -> Result<(), ProgramError> {
    if accounts.len() < FIXED_ACCOUNTS + 1 {
        msg!(
            "DLMM: expected at least {} accounts, got {}",
            FIXED_ACCOUNTS + 1,
            accounts.len()
        );
        return Err(ProgramError::NotEnoughAccountKeys);
    }

    let dex_program = accounts[0].key;

    // Data: disc(8) + amount_in(8) + min_out=1(8) + 0u32(4) = 28 bytes
    let mut data = Vec::with_capacity(28);
    data.extend_from_slice(&SWAP2_DISC);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&1u64.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());

    // Bitmap extension: swap2 declares it `mut`, so it must be writable when
    // present. The dex program id doubles as Anchor's None marker (readonly).
    let bitmap_ext = if accounts[5].key == dex_program {
        AccountMeta::new_readonly(*accounts[5].key, false)
    } else {
        AccountMeta::new(*accounts[5].key, false)
    };

    // CPI metas — fixed prefix (16), then variable bin arrays
    let mut metas = Vec::with_capacity(accounts.len());
    metas.push(AccountMeta::new(*accounts[4].key, false));            //  0 lb_pair
    metas.push(bitmap_ext);                                           //  1 bin_array_bitmap_ext
    metas.push(AccountMeta::new(*accounts[6].key, false));            //  2 reserve_x
    metas.push(AccountMeta::new(*accounts[7].key, false));            //  3 reserve_y
    metas.push(AccountMeta::new(*accounts[2].key, false));            //  4 swap_source_token
    metas.push(AccountMeta::new(*accounts[3].key, false));            //  5 swap_destination_token
    metas.push(AccountMeta::new_readonly(*accounts[8].key, false));   //  6 token_x_mint
    metas.push(AccountMeta::new_readonly(*accounts[9].key, false));   //  7 token_y_mint
    metas.push(AccountMeta::new(*accounts[10].key, false));           //  8 oracle
    metas.push(AccountMeta::new(*accounts[11].key, false));           //  9 host_fee_in
    metas.push(AccountMeta::new_readonly(*accounts[1].key, true));    // 10 swap_authority (readonly, signer)
    metas.push(AccountMeta::new_readonly(*accounts[12].key, false));  // 11 token_x_program
    metas.push(AccountMeta::new_readonly(*accounts[13].key, false));  // 12 token_y_program
    metas.push(AccountMeta::new_readonly(*accounts[14].key, false));  // 13 memo_program
    metas.push(AccountMeta::new_readonly(*accounts[15].key, false));  // 14 event_authority
    metas.push(AccountMeta::new_readonly(*dex_program, false));       // 15 program (Anchor self-ref)

    // All trailing accounts are bin arrays, forwarded writable in order.
    for bin_array in &accounts[FIXED_ACCOUNTS..] {
        metas.push(AccountMeta::new(*bin_array.key, false));
    }

    let ix = Instruction { program_id: *dex_program, accounts: metas, data };
    invoke(&ix, accounts)?;
    Ok(())
}
