use crate::utils::constants::METEORA_DYNAMIC_LMM;

pub const BIN_ARRAY: &[u8] = b"bin_array";

pub fn derive_bin_array_pda(
    lb_pair: solana_pubkey::Pubkey,
    bin_array_index: i64,
) -> (solana_pubkey::Pubkey, u8) {
    let meteora_dlmm_program = solana_pubkey::Pubkey::from_str_const(METEORA_DYNAMIC_LMM);

    solana_pubkey::Pubkey::find_program_address(
        &[BIN_ARRAY, lb_pair.as_ref(), &bin_array_index.to_le_bytes()],
        &meteora_dlmm_program,
    )
}

// Helper function to sort token mints
fn _sort_token_mints<'a>(
    token_x: &'a solana_pubkey::Pubkey,
    token_y: &'a solana_pubkey::Pubkey,
) -> (&'a solana_pubkey::Pubkey, &'a solana_pubkey::Pubkey) {
    if token_x.to_bytes() < token_y.to_bytes() {
        (token_x, token_y)
    } else {
        (token_y, token_x)
    }
}
