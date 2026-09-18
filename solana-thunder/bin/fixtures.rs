//! thunder-fixtures: dump every account a swap through the given pools would
//! touch, so LiteSVM tests can replay swaps offline.
//!
//! Usage:
//!   cargo run --bin thunder-fixtures -- <pool_address> [<pool_address>...]
//!
//! For each pool this identifies the DEX by account owner, enumerates the swap
//! account set (reusing the engine's per-DEX collectors with a dummy user,
//! plus tick/bin arrays +-3 around the current position), fetches everything
//! via getMultipleAccounts, and writes:
//!   fixtures/accounts/<pubkey>.json   (`solana account --output json` shape)
//!   fixtures/manifest.json            ({ slot, unix_timestamp, pools })
//! It then prints — and attempts to run — the `solana program dump` commands
//! for the DEX programs the dumped pools need.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use borsh::BorshDeserialize;
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::account::Account;
use thunder_core::{GenericError, TOKEN_PROGRAM};
use thunder_engine::swap::{
    collect_clmm_accounts, collect_damm_v1_accounts, collect_damm_v2_accounts,
    collect_dlmm_accounts, collect_pumpfun_accounts, collect_ray_v4_accounts,
};

/// (program_id, dex_name, dumped .so filename). dex_name matches
/// `RouteHop::dex_name` / `PoolInfo::dex_name` conventions.
const DEX_PROGRAMS: &[(&str, &str, &str)] = &[
    ("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG", "Meteora DAMM V2", "meteora_damm_v2.so"),
    ("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo", "Meteora DLMM", "meteora_dlmm.so"),
    ("Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB", "Meteora DAMM V1", "meteora_damm_v1.so"),
    ("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK", "Raydium CLMM", "raydium_clmm.so"),
    ("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8", "Raydium AMM V4", "raydium_amm_v4.so"),
    ("pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA", "Pumpfun AMM", "pumpfun_amm.so"),
];

/// Non-DEX programs the swaps CPI into, keyed by program id.
const AUX_PROGRAMS: &[(&str, &str)] = &[
    (meteora_damm::METEORA_VAULT_PROGRAM, "meteora_vault.so"),
];

/// Programs LiteSVM provides natively — never dumped.
const NATIVE_PROGRAMS: &[&str] = &[
    "11111111111111111111111111111111",
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
    "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr",
    "Memo1UhkJRfHyvLMcVucJwxXeuD728EqVDDwQDxFMNo",
    "ComputeBudget111111111111111111111111111111",
    "AddressLookupTab1e1111111111111111111111111",
];

const DLMM_PROGRAM: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
const SOLANA_CLI: &str = ".local/share/solana/install/active_release/bin/solana";

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), GenericError> {
    dotenvy::dotenv().ok();

    let pool_args: Vec<Pubkey> = env::args()
        .skip(1)
        .map(|s| s.parse::<Pubkey>().map_err(|e| format!("invalid pool address {s}: {e}")))
        .collect::<Result<_, _>>()?;
    if pool_args.is_empty() {
        return Err("usage: thunder-fixtures <pool_address> [<pool_address>...]".into());
    }

    let rpc_url = env::var("RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".into());
    let rpc = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());
    println!("RPC: {rpc_url}");

    // --- Fetch pool accounts and identify DEXes -----------------------------
    let (pool_accounts, mut slot) = fetch_multiple(&rpc, &pool_args)?;

    let mut pools_meta: Vec<(Pubkey, &'static str)> = Vec::new();
    let mut candidates: BTreeSet<Pubkey> = BTreeSet::new();
    // Dummy user + ATAs passed to the collectors; never dumped.
    let dummy_user = Pubkey::new_unique();
    let dummy_in = Pubkey::new_unique();
    let dummy_out = Pubkey::new_unique();
    let tp = Pubkey::from_str_const(TOKEN_PROGRAM);

    for (pool, account) in pool_args.iter().zip(&pool_accounts) {
        let account = account
            .as_ref()
            .ok_or_else(|| format!("pool account {pool} not found on-chain"))?;
        let dex_name = DEX_PROGRAMS
            .iter()
            .find(|(id, _, _)| account.owner == Pubkey::from_str_const(id))
            .map(|(_, name, _)| *name)
            .ok_or_else(|| format!("pool {pool}: unknown owner {}", account.owner))?;
        println!("{pool}: {dex_name} ({} bytes)", account.data.len());

        let data = &account.data;
        let mut extra: Vec<Pubkey> = Vec::new();
        let metas = match dex_name {
            "Meteora DAMM V2" => collect_damm_v2_accounts(
                data, *pool, dummy_user, dummy_in, dummy_out, &Pubkey::default(),
            ),
            "Meteora DLMM" => {
                let lb: meteora_dlmm::MeteoraDLMMPool = deser_anchor(data)?;
                let dlmm = Pubkey::from_str_const(DLMM_PROGRAM);
                let (bitmap_ext, _) = Pubkey::find_program_address(
                    &[b"bitmap", pool.as_ref()], &dlmm,
                );
                // Active bin array +-3 in both directions.
                let active_index = (lb.active_id as i64).div_euclid(70);
                for i in (active_index - 3)..=(active_index + 3) {
                    let (pda, _) = Pubkey::find_program_address(
                        &[b"bin_array", pool.as_ref(), &i.to_le_bytes()], &dlmm,
                    );
                    extra.push(pda);
                }
                // Both swap directions' bin array windows.
                let mut metas = collect_dlmm_accounts(
                    data, *pool, dummy_user, dummy_in, dummy_out,
                    &lb.token_x_mint, tp, tp, Some(bitmap_ext),
                )?;
                metas.extend(collect_dlmm_accounts(
                    data, *pool, dummy_user, dummy_in, dummy_out,
                    &lb.token_y_mint, tp, tp, Some(bitmap_ext),
                )?);
                metas
            }
            "Meteora DAMM V1" => {
                let p: meteora_damm::MeteoraDAMMPool = deser_anchor(data)?;
                extra.push(p.token_a_mint);
                extra.push(p.token_b_mint);
                // The vault state accounts carry token_vault/lp_mint.
                let (vault_accounts, _) = fetch_multiple(&rpc, &[p.a_vault, p.b_vault])?;
                let a_vault_data = vault_accounts[0].as_ref()
                    .map(|a| a.data.clone())
                    .ok_or("DAMM V1 a_vault account not found on-chain")?;
                let b_vault_data = vault_accounts[1].as_ref()
                    .map(|a| a.data.clone())
                    .ok_or("DAMM V1 b_vault account not found on-chain")?;
                // Both directions (the protocol fee account differs).
                let mut metas = collect_damm_v1_accounts(
                    data, *pool, dummy_user, dummy_in, dummy_out, &p.token_a_mint,
                    &a_vault_data, &b_vault_data,
                )?;
                metas.extend(collect_damm_v1_accounts(
                    data, *pool, dummy_user, dummy_in, dummy_out, &p.token_b_mint,
                    &a_vault_data, &b_vault_data,
                )?);
                metas
            }
            "Raydium CLMM" => {
                let p: raydium_clmm::RaydiumCLMMPool = deser_anchor(data)?;
                // Initialized tick arrays +-3 in both directions from the
                // current tick, straight from the in-pool bitmap.
                extra.extend(raydium_clmm::tick_arrays::derive_pool_tick_array_pdas(&p, pool));
                // The bitmap extension account data drives swap-ordered tick
                // array selection; fetch it up front.
                let ext_pda = thunder_engine::swap::clmm_bitmap_extension_pda(pool);
                let (ext_accounts, _) = fetch_multiple(&rpc, &[ext_pda])?;
                let ext_data = ext_accounts[0].as_ref().map(|a| a.data.clone());
                // Both swap directions' tick array windows.
                let mut metas = collect_clmm_accounts(
                    data, *pool, dummy_user, dummy_in, dummy_out,
                    &p.token_mint_0, ext_data.as_deref(),
                )?;
                metas.extend(collect_clmm_accounts(
                    data, *pool, dummy_user, dummy_in, dummy_out,
                    &p.token_mint_1, ext_data.as_deref(),
                )?);
                metas
            }
            "Raydium AMM V4" => {
                let p: raydium_amm_v4::RaydiumAMMV4 =
                    borsh::BorshDeserialize::try_from_slice(data)
                        .map_err(|e| format!("V4 pool {pool}: {e}"))?;
                extra.push(p.base_mint);
                extra.push(p.quote_mint);
                collect_ray_v4_accounts(data, *pool, dummy_user, dummy_in, dummy_out)
            }
            "Pumpfun AMM" => {
                let base_mint = pubkey_at(data, 43);
                let quote_mint = pubkey_at(data, 75);
                extra.push(base_mint);
                extra.push(quote_mint);
                // The GlobalConfig account carries the fee recipients; fetch
                // it up front so the collector can read them.
                let config_pda = thunder_engine::swap::pumpfun_global_config_pda();
                let (config_accounts, _) = fetch_multiple(&rpc, &[config_pda])?;
                let config_data = config_accounts[0]
                    .as_ref()
                    .map(|a| a.data.clone())
                    .ok_or("pumpfun global config account not found on-chain")?;
                // Both directions (buy = quote in, sell = base in) so the
                // volume-accumulator PDAs of each layout get dumped.
                let mut metas = collect_pumpfun_accounts(
                    data, *pool, dummy_user, dummy_in, dummy_out,
                    &quote_mint, tp, tp, &config_data,
                )?;
                metas.extend(collect_pumpfun_accounts(
                    data, *pool, dummy_user, dummy_in, dummy_out,
                    &base_mint, tp, tp, &config_data,
                )?);
                metas
            }
            _ => unreachable!(),
        };

        candidates.insert(*pool);
        candidates.extend(metas.iter().map(|m| m.pubkey));
        candidates.extend(extra);
        pools_meta.push((*pool, dex_name));
    }

    // Drop dummies and sentinels.
    for pk in [dummy_user, dummy_in, dummy_out, Pubkey::default()] {
        candidates.remove(&pk);
    }

    // --- Fetch the full account set -----------------------------------------
    let all_keys: Vec<Pubkey> = candidates.into_iter().collect();
    println!("\nFetching {} candidate accounts...", all_keys.len());
    let (fetched, fetch_slot) = fetch_multiple(&rpc, &all_keys)?;
    slot = slot.max(fetch_slot);

    let accounts_dir = PathBuf::from("fixtures/accounts");
    fs::create_dir_all(&accounts_dir)?;
    fs::create_dir_all("fixtures/programs")?;

    let mut programs_needed: BTreeMap<Pubkey, String> = BTreeMap::new();
    let mut written = 0usize;
    for (pk, maybe) in all_keys.iter().zip(&fetched) {
        let Some(acc) = maybe else {
            println!("  (absent)  {pk}");
            continue;
        };
        if acc.executable {
            if NATIVE_PROGRAMS.iter().any(|id| *pk == Pubkey::from_str_const(id)) {
                continue;
            }
            programs_needed.insert(*pk, program_so_name(pk));
            continue;
        }
        write_account_json(&accounts_dir, pk, acc)?;
        written += 1;
    }
    // The DEX programs themselves (metas include them at index 0, but be
    // explicit in case a collector ever drops the program meta).
    for (pool, dex_name) in &pools_meta {
        let (id, _, so) = DEX_PROGRAMS
            .iter()
            .find(|(_, name, _)| name == dex_name)
            .expect("dex_name from DEX_PROGRAMS");
        let _ = pool; // pool id already dumped above
        programs_needed.insert(Pubkey::from_str_const(id), so.to_string());
    }
    println!("Wrote {written} account fixtures to fixtures/accounts/");

    // --- Manifest ------------------------------------------------------------
    let unix_timestamp = block_time_or_now(&rpc, slot);
    let manifest = serde_json::json!({
        "slot": slot,
        "unix_timestamp": unix_timestamp,
        "pools": pools_meta
            .iter()
            .map(|(pool, dex_name)| serde_json::json!({
                "address": pool.to_string(),
                "dex_name": dex_name,
            }))
            .collect::<Vec<_>>(),
    });
    fs::write("fixtures/manifest.json", serde_json::to_string_pretty(&manifest)?)?;
    println!("Wrote fixtures/manifest.json (slot {slot}, unix_timestamp {unix_timestamp})");

    // --- Program dumps --------------------------------------------------------
    println!("\nProgram dumps needed:");
    for (id, so_name) in &programs_needed {
        println!("  solana program dump {id} fixtures/programs/{so_name} -u {rpc_url}");
    }
    let solana_cli = PathBuf::from(env::var("HOME").unwrap_or_default()).join(SOLANA_CLI);
    if solana_cli.exists() {
        for (id, so_name) in &programs_needed {
            let out_path = format!("fixtures/programs/{so_name}");
            dump_program(&solana_cli, id, &out_path, &rpc_url);
        }
    } else {
        println!("(solana CLI not found at {}; run the commands above manually)", solana_cli.display());
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// RPC helpers
// ---------------------------------------------------------------------------

/// getMultipleAccounts in batches of 100, with pacing and 429/backoff retries.
/// Returns the accounts plus the highest context slot observed.
fn fetch_multiple(
    rpc: &RpcClient,
    keys: &[Pubkey],
) -> Result<(Vec<Option<Account>>, u64), GenericError> {
    let mut out = Vec::with_capacity(keys.len());
    let mut max_slot = 0u64;
    for chunk in keys.chunks(100) {
        let mut delay = Duration::from_millis(500);
        let mut attempts = 0;
        let resp = loop {
            match rpc.get_multiple_accounts_with_commitment(chunk, CommitmentConfig::confirmed()) {
                Ok(resp) => break resp,
                Err(e) => {
                    attempts += 1;
                    if attempts >= 5 {
                        return Err(format!("getMultipleAccounts failed after {attempts} tries: {e}").into());
                    }
                    eprintln!("  rpc error ({e}); retrying in {delay:?}");
                    sleep(delay);
                    delay *= 2;
                }
            }
        };
        max_slot = max_slot.max(resp.context.slot);
        out.extend(resp.value);
        sleep(Duration::from_millis(200));
    }
    Ok((out, max_slot))
}

fn block_time_or_now(rpc: &RpcClient, slot: u64) -> i64 {
    for probe in [slot, slot.saturating_sub(20)] {
        if let Ok(ts) = rpc.get_block_time(probe) {
            return ts;
        }
        sleep(Duration::from_millis(200));
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn dump_program(solana_cli: &Path, id: &Pubkey, out_path: &str, rpc_url: &str) {
    for attempt in 1..=3 {
        let result = Command::new(solana_cli)
            .args(["program", "dump", &id.to_string(), out_path, "-u", rpc_url])
            .output();
        match result {
            Ok(o) if o.status.success() => {
                println!("  dumped {id} -> {out_path}");
                return;
            }
            Ok(o) => {
                eprintln!(
                    "  dump {id} attempt {attempt} failed: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                );
            }
            Err(e) => eprintln!("  dump {id} attempt {attempt} failed to spawn: {e}"),
        }
        sleep(Duration::from_secs(2));
    }
    eprintln!("  giving up on {id}; run the printed command manually");
}

// ---------------------------------------------------------------------------
// Misc helpers
// ---------------------------------------------------------------------------

/// Deserialize an Anchor account, skipping the 8-byte discriminator.
/// Tolerates trailing bytes.
fn deser_anchor<T: BorshDeserialize>(data: &[u8]) -> Result<T, GenericError> {
    if data.len() < 8 {
        return Err("account data too short".into());
    }
    let mut slice = &data[8..];
    T::deserialize(&mut slice).map_err(|e| e.into())
}

/// Extract a 32-byte Pubkey from raw account data at the given byte offset.
fn pubkey_at(data: &[u8], offset: usize) -> Pubkey {
    Pubkey::new_from_array(data[offset..offset + 32].try_into().unwrap())
}

fn program_so_name(id: &Pubkey) -> String {
    DEX_PROGRAMS
        .iter()
        .map(|(pid, _, so)| (*pid, *so))
        .chain(AUX_PROGRAMS.iter().copied())
        .find(|(pid, _)| *id == Pubkey::from_str_const(pid))
        .map(|(_, so)| so.to_string())
        .unwrap_or_else(|| format!("{id}.so"))
}

/// Write one account in the `solana account --output json` shape.
fn write_account_json(
    dir: &Path,
    pk: &Pubkey,
    acc: &Account,
) -> Result<(), GenericError> {
    let obj = serde_json::json!({
        "pubkey": pk.to_string(),
        "account": {
            "lamports": acc.lamports,
            "data": [BASE64.encode(&acc.data), "base64"],
            "owner": acc.owner.to_string(),
            "executable": acc.executable,
            "rentEpoch": acc.rent_epoch,
            "space": acc.data.len(),
        },
    });
    fs::write(
        dir.join(format!("{pk}.json")),
        serde_json::to_string_pretty(&obj)?,
    )?;
    Ok(())
}

