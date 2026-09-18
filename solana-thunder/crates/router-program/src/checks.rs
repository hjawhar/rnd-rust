//! Hardening checks around each hop's CPI (ported from the OKX router
//! pattern). All checks return `ProgramError::Custom` codes defined in
//! `lib.rs` so failures are distinguishable in transaction logs.

use solana_program::{
    account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey,
};

use crate::{DexType, ERR_BAD_AUTHORITY, ERR_CHAIN_MISMATCH, ERR_DEX_PROGRAM_MISMATCH};

// Mainnet DEX program ids, one per DexType.
const DAMM_V1_PROGRAM: Pubkey =
    Pubkey::from_str_const("Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB");
const DAMM_V2_PROGRAM: Pubkey =
    Pubkey::from_str_const("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG");
const DLMM_PROGRAM: Pubkey =
    Pubkey::from_str_const("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
const CLMM_PROGRAM: Pubkey =
    Pubkey::from_str_const("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK");
const RAY_V4_PROGRAM: Pubkey =
    Pubkey::from_str_const("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");
const PUMPFUN_PROGRAM: Pubkey =
    Pubkey::from_str_const("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA");

const TOKEN_PROGRAM: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const TOKEN_2022_PROGRAM: Pubkey =
    Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// SPL token account minimum length (mint 0..32, owner 32..64, amount 64..72).
const TOKEN_ACCOUNT_MIN_LEN: usize = 165;

/// The hardcoded DEX program each DexType is allowed to CPI into.
pub fn expected_dex_program(dex_type: DexType) -> Pubkey {
    match dex_type {
        DexType::MeteoraDAMMV1 => DAMM_V1_PROGRAM,
        DexType::MeteoraDAMMV2 => DAMM_V2_PROGRAM,
        DexType::MeteoraDLMM => DLMM_PROGRAM,
        DexType::RaydiumCLMM => CLMM_PROGRAM,
        DexType::RaydiumAMMV4 => RAY_V4_PROGRAM,
        DexType::PumpfunBuy | DexType::PumpfunSell => PUMPFUN_PROGRAM,
    }
}

/// Read the SPL mint field (bytes 0..32) of a token account.
fn token_mint(account: &AccountInfo) -> Option<[u8; 32]> {
    let data = account.try_borrow_data().ok()?;
    if data.len() < TOKEN_ACCOUNT_MIN_LEN {
        return None;
    }
    data[0..32].try_into().ok()
}

/// Read the SPL owner field (bytes 32..64) of a token account.
fn token_owner(account: &AccountInfo) -> Option<[u8; 32]> {
    let data = account.try_borrow_data().ok()?;
    if data.len() < TOKEN_ACCOUNT_MIN_LEN {
        return None;
    }
    data[32..64].try_into().ok()
}

/// Pre-CPI validation of one hop's account slice (uniform prefix:
/// [0] dex_program, [1] authority, [2] source token, [3] dest token).
///
/// Returns the dest token account's mint for the next hop's chaining check.
pub fn before_hop(
    hop_accounts: &[AccountInfo],
    dex_type: DexType,
    prev_dest_mint: Option<&[u8; 32]>,
) -> Result<[u8; 32], ProgramError> {
    if hop_accounts.len() < 4 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let dex_program = &hop_accounts[0];
    let authority = &hop_accounts[1];
    let source = &hop_accounts[2];
    let dest = &hop_accounts[3];

    // (a) accounts[0] must be the hardcoded program for this DexType.
    if *dex_program.key != expected_dex_program(dex_type) {
        return Err(ProgramError::Custom(ERR_DEX_PROGRAM_MISMATCH));
    }

    // (b) The swap authority must actually sign, and the source token account
    // must belong to it.
    if !authority.is_signer {
        return Err(ProgramError::Custom(ERR_BAD_AUTHORITY));
    }
    let authority_bytes = authority.key.to_bytes();
    match token_owner(source) {
        Some(owner) if owner == authority_bytes => {}
        _ => return Err(ProgramError::Custom(ERR_BAD_AUTHORITY)),
    }

    // (b) Hop N's source mint must equal hop N-1's dest mint.
    let source_mint = token_mint(source).ok_or(ProgramError::Custom(ERR_BAD_AUTHORITY))?;
    if let Some(prev) = prev_dest_mint {
        if source_mint != *prev {
            return Err(ProgramError::Custom(ERR_CHAIN_MISMATCH));
        }
    }

    // (d) No other writable token account owned by the authority may be
    // smuggled into the hop slice (an adapter bug or a malicious account list
    // could otherwise let a DEX drain it).
    for (i, account) in hop_accounts.iter().enumerate() {
        if i == 1 || i == 2 || i == 3 || !account.is_writable {
            continue;
        }
        if *account.owner != TOKEN_PROGRAM && *account.owner != TOKEN_2022_PROGRAM {
            continue;
        }
        if let Some(owner) = token_owner(account) {
            if owner == authority_bytes {
                return Err(ProgramError::Custom(ERR_BAD_AUTHORITY));
            }
        }
    }

    // Dest mint feeds the next hop's chaining check. The dest ATA always
    // exists at this point (the transaction creates it idempotently before
    // the router instruction).
    token_mint(dest).ok_or(ProgramError::Custom(ERR_CHAIN_MISMATCH))
}
