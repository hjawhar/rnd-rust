//! Address Lookup Table (ALT) support.
//!
//! One static, ops-owned ALT holds every hot constant the swap builder
//! references (DEX programs, authorities, event authorities, fee PDAs,
//! common mints) so multi-hop v0 transactions fit in 1232 bytes.
//!
//! The table is created per environment with the `thunder-alt` binary
//! (`bin/alt.rs`); the engine loads it from the `SWAP_ALT_ADDRESS` env var
//! and refreshes it every 10 minutes (see `bin/engine.rs`).

use std::env;
use std::str::FromStr;

use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    message::{AddressLookupTableAccount, Message, VersionedMessage},
    signature::{Keypair, Signer},
    transaction::VersionedTransaction,
};
use spl_associated_token_account::get_associated_token_address;
use thunder_core::{GenericError, USDC, USDT, WSOL};

use crate::swap::{
    pumpfun_global_config_pda, ATA_PROGRAM, CLMM_PROGRAM, DAMM_V1_PROGRAM, DAMM_V2_POOL_AUTHORITY,
    DAMM_V2_PROGRAM, DLMM_EVENT_AUTHORITY, DLMM_PROGRAM, MEMO_PROGRAM, PUMPFUN_PROGRAM,
    RAY_V4_AUTHORITY, RAY_V4_PROGRAM, SYSTEM_PROGRAM, VAULT_PROGRAM,
};

/// The on-chain Address Lookup Table program.
pub const ALT_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("AddressLookupTab1e1111111111111111111111111");

/// Compute Budget program (referenced by every swap transaction).
pub const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";

/// Size of the serialized `LookupTableMeta` prefix in an ALT account;
/// stored addresses start at this byte offset.
pub const LOOKUP_TABLE_META_SIZE: usize = 56;

// ---------------------------------------------------------------------------
// Address derivation + instruction builders
// ---------------------------------------------------------------------------

/// Derive the ALT address from authority + recent slot (mirrors the on-chain
/// derivation).
pub fn derive_lookup_table_address(authority: &Pubkey, recent_slot: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[authority.as_ref(), &recent_slot.to_le_bytes()],
        &ALT_PROGRAM_ID,
    )
}

/// Build a CreateLookupTable instruction (bincode-encoded, variant 0).
/// Returns the instruction and the derived table address.
pub fn build_create_alt_ix(
    authority: Pubkey,
    payer: Pubkey,
    recent_slot: u64,
) -> (Instruction, Pubkey) {
    let (alt_address, bump_seed) = derive_lookup_table_address(&authority, recent_slot);
    // bincode: variant 0 (u32 LE) + recent_slot (u64 LE) + bump_seed (u8)
    let mut data = Vec::with_capacity(13);
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&recent_slot.to_le_bytes());
    data.push(bump_seed);
    let ix = Instruction {
        program_id: ALT_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(alt_address, false),
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(Pubkey::from_str_const(SYSTEM_PROGRAM), false),
        ],
        data,
    };
    (ix, alt_address)
}

/// Build an ExtendLookupTable instruction (bincode-encoded, variant 2).
pub fn build_extend_alt_ix(
    alt_address: Pubkey,
    authority: Pubkey,
    payer: Pubkey,
    new_addresses: &[Pubkey],
) -> Instruction {
    // bincode: variant 2 (u32 LE) + vec_len (u64 LE) + pubkeys (32 bytes each)
    let mut data = Vec::with_capacity(4 + 8 + new_addresses.len() * 32);
    data.extend_from_slice(&2u32.to_le_bytes());
    data.extend_from_slice(&(new_addresses.len() as u64).to_le_bytes());
    for addr in new_addresses {
        data.extend_from_slice(addr.as_ref());
    }
    Instruction {
        program_id: ALT_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(alt_address, false),
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(Pubkey::from_str_const(SYSTEM_PROGRAM), false),
        ],
        data,
    }
}

// ---------------------------------------------------------------------------
// Account parsing / loading
// ---------------------------------------------------------------------------

/// Parse the address list out of a raw ALT account's data.
///
/// Layout: 56-byte `LookupTableMeta` (u32 type discriminant = 1, then
/// deactivation/extension slots, authority, padding) followed by the stored
/// addresses in 32-byte chunks.
pub fn parse_alt_account(data: &[u8]) -> Result<Vec<Pubkey>, GenericError> {
    if data.len() < LOOKUP_TABLE_META_SIZE {
        return Err(format!(
            "ALT account too short: {} bytes < {LOOKUP_TABLE_META_SIZE}",
            data.len()
        )
        .into());
    }
    let discriminant = u32::from_le_bytes(data[0..4].try_into().unwrap());
    if discriminant != 1 {
        return Err(format!("not an initialized lookup table (type {discriminant})").into());
    }
    let addr_bytes = &data[LOOKUP_TABLE_META_SIZE..];
    if addr_bytes.len() % 32 != 0 {
        return Err(format!(
            "ALT address region not 32-byte aligned ({} bytes)",
            addr_bytes.len()
        )
        .into());
    }
    Ok(addr_bytes
        .chunks_exact(32)
        .map(|c| Pubkey::new_from_array(c.try_into().unwrap()))
        .collect())
}

/// Fetch and parse an ALT account into an `AddressLookupTableAccount` ready
/// for `v0::Message::try_compile`.
pub async fn load_alt(
    rpc: &RpcClient,
    address: &Pubkey,
) -> Result<AddressLookupTableAccount, GenericError> {
    let account = rpc
        .get_account(address)
        .await
        .map_err(|e| format!("fetch ALT {address}: {e}"))?;
    if account.owner != ALT_PROGRAM_ID {
        return Err(format!(
            "account {address} is not owned by the lookup-table program (owner {})",
            account.owner
        )
        .into());
    }
    let addresses = parse_alt_account(&account.data)?;
    Ok(AddressLookupTableAccount {
        key: *address,
        addresses,
    })
}

// ---------------------------------------------------------------------------
// Curated swap-ALT address list
// ---------------------------------------------------------------------------

/// Static (RPC-free) portion of the curated swap ALT: every hot constant the
/// per-DEX collectors in `swap.rs` reference. The router program id is taken
/// from the `ROUTER_PROGRAM_ID` env var when set (each environment has its
/// own deploy).
pub fn static_alt_addresses() -> Vec<Pubkey> {
    let damm_v2 = Pubkey::from_str_const(DAMM_V2_PROGRAM);
    let pumpfun = Pubkey::from_str_const(PUMPFUN_PROGRAM);
    let (damm_v2_event_auth, _) = Pubkey::find_program_address(&[b"__event_authority"], &damm_v2);
    let (pumpfun_event_auth, _) = Pubkey::find_program_address(&[b"__event_authority"], &pumpfun);

    let mut addrs = vec![
        // DEX programs
        Pubkey::from_str_const(DLMM_PROGRAM),
        Pubkey::from_str_const(CLMM_PROGRAM),
        Pubkey::from_str_const(DAMM_V1_PROGRAM),
        damm_v2,
        Pubkey::from_str_const(RAY_V4_PROGRAM),
        pumpfun,
        // Meteora dynamic vault program (DAMM V1 funds live in these vaults)
        Pubkey::from_str_const(VAULT_PROGRAM),
        // Infrastructure programs
        Pubkey::from_str_const(thunder_core::TOKEN_PROGRAM),
        Pubkey::from_str_const(thunder_core::TOKEN_PROGRAM_2022),
        Pubkey::from_str_const(ATA_PROGRAM),
        Pubkey::from_str_const(SYSTEM_PROGRAM),
        Pubkey::from_str_const(MEMO_PROGRAM),
        Pubkey::from_str_const(COMPUTE_BUDGET_PROGRAM),
        // Static authorities
        Pubkey::from_str_const(RAY_V4_AUTHORITY),
        Pubkey::from_str_const(DAMM_V2_POOL_AUTHORITY),
        // Event authorities
        Pubkey::from_str_const(DLMM_EVENT_AUTHORITY),
        damm_v2_event_auth,
        pumpfun_event_auth,
        // Pumpfun static PDAs + fee program
        pumpfun_global_config_pda(),
        pumpfun_amm::pda::get_pumpfun_config_pda(),
        Pubkey::from_str_const(pumpfun_amm::PUMPFUN_FEE_PROGRAM),
        pumpfun_amm::pda::get_global_volume_accumulator_pda(),
        // Common mints
        Pubkey::from_str_const(WSOL),
        Pubkey::from_str_const(USDC),
        Pubkey::from_str_const(USDT),
    ];
    // The router deploy for this environment.
    if let Ok(router) = env::var("ROUTER_PROGRAM_ID") {
        if let Ok(pk) = Pubkey::from_str(&router) {
            addrs.push(pk);
        }
    }
    addrs
}

/// Pumpfun fee-recipient addresses read from the live GlobalConfig account:
/// protocol fee recipient, reserved (mayhem-mode) fee recipient, and buyback
/// fee recipient, each with its WSOL ATA (pumpfun pools quote in WSOL).
///
/// GlobalConfig layout (after the 8-byte discriminator):
/// protocol_fee_recipients[0] @57, reserved_fee_recipient @385,
/// buyback_fee_recipients[0] @643 (mirrors `swap::collect_pumpfun_accounts`).
pub async fn pumpfun_fee_recipient_addresses(
    rpc: &RpcClient,
) -> Result<Vec<Pubkey>, GenericError> {
    let config_pda = pumpfun_global_config_pda();
    let account = rpc
        .get_account(&config_pda)
        .await
        .map_err(|e| format!("fetch pumpfun global config {config_pda}: {e}"))?;
    if account.data.len() < 907 {
        return Err(format!(
            "pumpfun global config: expected >= 907 bytes, got {}",
            account.data.len()
        )
        .into());
    }
    let wsol = Pubkey::from_str_const(WSOL);
    let mut out = Vec::with_capacity(6);
    for offset in [57usize, 385, 643] {
        let recipient =
            Pubkey::new_from_array(account.data[offset..offset + 32].try_into().unwrap());
        if recipient == Pubkey::default() {
            continue;
        }
        out.push(recipient);
        out.push(get_associated_token_address(&recipient, &wsol));
    }
    Ok(out)
}

/// Full curated swap-ALT address list: the static constants plus the live
/// pumpfun fee recipients read via RPC. Deduplicated, order-stable.
pub async fn curated_alt_addresses(rpc: &RpcClient) -> Result<Vec<Pubkey>, GenericError> {
    let mut addrs = static_alt_addresses();
    match pumpfun_fee_recipient_addresses(rpc).await {
        Ok(recipients) => addrs.extend(recipients),
        Err(e) => eprintln!("warning: skipping pumpfun fee recipients in ALT: {e}"),
    }
    let mut deduped: Vec<Pubkey> = Vec::with_capacity(addrs.len());
    for a in addrs {
        if !deduped.contains(&a) {
            deduped.push(a);
        }
    }
    Ok(deduped)
}

// ---------------------------------------------------------------------------
// Creation / extension (used by bin/alt.rs and the surfpool test)
// ---------------------------------------------------------------------------

/// Sign and send a single-instruction legacy transaction.
async fn send_ix(rpc: &RpcClient, payer: &Keypair, ix: Instruction) -> Result<(), GenericError> {
    let blockhash = rpc.get_latest_blockhash().await?;
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[payer])?;
    rpc.send_and_confirm_transaction(&tx).await?;
    Ok(())
}

/// Create a new ALT owned by `payer` and extend it with the curated swap
/// address list. Returns the ready-to-use `AddressLookupTableAccount`.
pub async fn create_swap_alt(
    rpc: &RpcClient,
    payer: &Keypair,
) -> Result<AddressLookupTableAccount, GenericError> {
    let addresses = curated_alt_addresses(rpc).await?;

    let recent_slot = rpc.get_slot().await?;
    let (create_ix, alt_address) =
        build_create_alt_ix(payer.pubkey(), payer.pubkey(), recent_slot);
    send_ix(rpc, payer, create_ix).await?;

    extend_alt(rpc, payer, &alt_address, &addresses).await?;

    // The table only becomes usable one slot after its last extension.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    Ok(AddressLookupTableAccount {
        key: alt_address,
        addresses,
    })
}

/// Extend an existing ALT with `addresses` (chunked to stay within tx size).
pub async fn extend_alt(
    rpc: &RpcClient,
    payer: &Keypair,
    alt_address: &Pubkey,
    addresses: &[Pubkey],
) -> Result<(), GenericError> {
    for chunk in addresses.chunks(20) {
        let ix = build_extend_alt_ix(*alt_address, payer.pubkey(), payer.pubkey(), chunk);
        send_ix(rpc, payer, ix).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_alt_round_trip() {
        let addresses = vec![Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique()];
        let mut data = vec![0u8; LOOKUP_TABLE_META_SIZE];
        data[0..4].copy_from_slice(&1u32.to_le_bytes()); // type = LookupTable
        for a in &addresses {
            data.extend_from_slice(a.as_ref());
        }
        assert_eq!(parse_alt_account(&data).unwrap(), addresses);
    }

    #[test]
    fn parse_alt_rejects_uninitialized() {
        let data = vec![0u8; LOOKUP_TABLE_META_SIZE];
        assert!(parse_alt_account(&data).is_err());
    }

    #[test]
    fn static_list_is_deduplicated_and_sized() {
        let addrs = static_alt_addresses();
        let mut seen = std::collections::HashSet::new();
        for a in &addrs {
            assert!(seen.insert(*a), "duplicate address in static ALT list: {a}");
        }
        assert!(addrs.len() >= 25, "static list unexpectedly small: {}", addrs.len());
    }
}
