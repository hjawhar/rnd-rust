//! Dynamic multi-hop swap on Surfpool via thunder-router.
//!
//! Exploratory fresh-state test: requires a running surfpool fork
//! (`surfpool start`) and a router deploy on it, so it is `#[ignore]`d.
//!
//! Run:
//!   ROUTER_PROGRAM_ID=<surfpool router deploy> \
//!   INPUT=SOL OUTPUT=6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN \
//!   AMOUNT=0.1 MAX_HOPS=2 \
//!   cargo test --release --test surfpool_swap -- --ignored --nocapture
//!
//! Transaction assembly lives in the production bundle path
//! (`thunder_engine::swap::build_swap_instructions` +
//! `compile_to_transaction` with the curated swap ALT); this test only
//! supplies live account data (AccountStore), pool auxiliaries
//! (PoolRegistry), and the ALT (loaded from SWAP_ALT_ADDRESS or created on
//! the fork via `thunder_engine::alt::create_swap_alt`).

use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;
use std::str::FromStr;

use solana_account_decoder_client_types::UiAccountEncoding;
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_rpc_client_api::filter::{Memcmp, RpcFilterType};
use solana_sdk::{
    message::AddressLookupTableAccount,
    signature::{Keypair, Signer},
};
use thunder_aggregator::{
    cache::{self, CachedPool},
    loader,
    pool_index::PoolIndex,
    router::Router,
    types::Route,
};
use thunder_core::{calculate_min_amount_out, infer_mint_decimals, WSOL};
use thunder_engine::account_store::AccountStore;
use thunder_engine::alt;
use thunder_engine::pool_registry::{PoolInfo, PoolRegistry};
use thunder_engine::swap::{build_swap_instructions, compile_to_transaction, SwapOptions};

const DLMM_PROGRAM: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
const CLMM_PROGRAM: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";

/// Supported DEX names for the router.
const SUPPORTED_DEXES: &[&str] = &[
    "Meteora DLMM",
    "Raydium CLMM",
    "Meteora DAMM V1",
    "Meteora DAMM V2",
    "Raydium AMM V4",
    "Pumpfun AMM",
];

// =========================================================================
// AccountStore / PoolRegistry assembly for the production builder
// =========================================================================

/// Fetch every account the production builder reads (pool accounts, mints,
/// and DEX auxiliaries: DAMM V1 vault states, CLMM bitmap extensions,
/// pumpfun global config) into an AccountStore. Mirrors the engine's
/// `ensure_route_accounts`.
async fn populate_store(rpc: &RpcClient, routes: &[&Route], store: &AccountStore) {
    let fetch = |keys: Vec<Pubkey>| async move {
        for chunk in keys.chunks(100) {
            if let Ok(accounts) = rpc.get_multiple_accounts(chunk).await {
                for (pk, maybe) in chunk.iter().zip(accounts) {
                    if let Some(acc) = maybe {
                        store.upsert(*pk, acc.data, acc.owner, acc.lamports, 0);
                    }
                }
            }
        }
    };

    // Phase 1: pools + mints.
    let mut keys: Vec<Pubkey> = Vec::new();
    for route in routes {
        for hop in &route.hops {
            if let Ok(pk) = Pubkey::from_str(&hop.pool_address) {
                if !keys.contains(&pk) {
                    keys.push(pk);
                }
            }
            for mint in [hop.input_mint, hop.output_mint] {
                if !keys.contains(&mint) {
                    keys.push(mint);
                }
            }
        }
    }
    fetch(keys).await;

    // Phase 2: DEX auxiliaries read from the (now fetched) pool bytes.
    let mut aux: Vec<Pubkey> = Vec::new();
    for route in routes {
        for hop in &route.hops {
            let Ok(pool_pk) = Pubkey::from_str(&hop.pool_address) else { continue };
            match hop.dex_name.as_str() {
                "Meteora DAMM V1" => {
                    if let Some(pool_data) = store.get_data(&pool_pk) {
                        if pool_data.len() >= 168 {
                            aux.push(Pubkey::new_from_array(pool_data[104..136].try_into().unwrap()));
                            aux.push(Pubkey::new_from_array(pool_data[136..168].try_into().unwrap()));
                        }
                    }
                }
                "Raydium CLMM" => {
                    aux.push(thunder_engine::swap::clmm_bitmap_extension_pda(&pool_pk));
                }
                "Pumpfun AMM" => {
                    aux.push(thunder_engine::swap::pumpfun_global_config_pda());
                }
                _ => {}
            }
        }
    }
    aux.dedup();
    fetch(aux).await;
}

/// Minimal PoolRegistry covering the given routes, rebuilt from the index's
/// cached pool bytes with the DEX auxiliaries (bitmap extensions, tick
/// arrays) the swap collectors need.
fn build_registry_for_routes(
    index: &PoolIndex,
    routes: &[&Route],
    bitmap_map: &HashMap<Pubkey, Pubkey>,
    tick_array_map: &HashMap<Pubkey, Vec<Pubkey>>,
) -> PoolRegistry {
    let mut registry = PoolRegistry::new();
    let mut seen: HashSet<String> = HashSet::new();
    for route in routes {
        for hop in &route.hops {
            if !seen.insert(hop.pool_address.clone()) {
                continue;
            }
            let Some(entry) = index.get_pool(&hop.pool_address) else { continue };
            if entry.cached_data.is_empty() {
                continue;
            }
            let Ok(cached) = bincode::deserialize::<CachedPool>(&entry.cached_data) else {
                continue;
            };
            let (address, owned) = cached.into_pool_entry();
            let Ok(meta) = owned.market.metadata() else { continue };
            let pool_pubkey = Pubkey::from_str(&address).unwrap_or_default();
            let info = PoolInfo {
                address: address.clone(),
                dex_name: owned.dex_name,
                market: owned.market,
                swappable: true,
                quote_mint: owned.quote_mint,
                base_mint: owned.base_mint,
                quote_vault: meta.quote_vault,
                base_vault: meta.base_vault,
                tick_arrays: tick_array_map.get(&pool_pubkey).cloned().unwrap_or_default(),
                bitmap_ext: bitmap_map.get(&pool_pubkey).copied(),
                bin_array: None,
                cached_data: owned.cached_data,
            };
            registry.add_pool(address, info);
        }
    }
    registry
}

// =========================================================================
// Address Lookup Table — production helpers live in thunder_engine::alt;
// the test either loads SWAP_ALT_ADDRESS or creates the curated ALT on the
// fork with the same code path `thunder-alt create` uses.
// =========================================================================

async fn obtain_swap_alt(rpc: &RpcClient, payer: &Keypair) -> Option<AddressLookupTableAccount> {
    if let Ok(addr_str) = env::var("SWAP_ALT_ADDRESS") {
        let addr = Pubkey::from_str(&addr_str).expect("invalid SWAP_ALT_ADDRESS");
        match alt::load_alt(rpc, &addr).await {
            Ok(table) => {
                println!("Loaded swap ALT {} ({} addresses)", table.key, table.addresses.len());
                return Some(table);
            }
            Err(e) => println!("Failed to load SWAP_ALT_ADDRESS ({e}); creating a fresh one"),
        }
    }
    match alt::create_swap_alt(rpc, payer).await {
        Ok(table) => {
            println!(
                "Created swap ALT {} ({} addresses) — export SWAP_ALT_ADDRESS={} to reuse",
                table.key,
                table.addresses.len(),
                table.key
            );
            Some(table)
        }
        Err(e) => {
            println!("ALT creation failed ({e}); continuing without ALT");
            None
        }
    }
}

// =========================================================================
// Test
// =========================================================================

#[tokio::test]
#[ignore = "requires a running surfpool fork + router deploy (set ROUTER_PROGRAM_ID); run manually"]
async fn test_dynamic_swap() {
    dotenvy::dotenv().ok();

    if env::var("ROUTER_PROGRAM_ID").is_err() {
        eprintln!("ROUTER_PROGRAM_ID not set: deploy thunder_router.so to surfpool and export its id");
        return;
    }

    let input_str = env::var("INPUT").unwrap_or_else(|_| "SOL".into());
    let output_str = env::var("OUTPUT").unwrap_or_else(|_| "6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN".into());
    let amount_str = env::var("AMOUNT").unwrap_or_else(|_| "0.1".into());
    let max_hops: usize = env::var("MAX_HOPS").ok().and_then(|s| s.parse().ok()).unwrap_or(2);
    let slippage_bps: u64 = env::var("SLIPPAGE").ok().and_then(|s| s.parse().ok()).unwrap_or(500);

    let input_mint = parse_mint(&input_str);
    let output_mint = parse_mint(&output_str);
    let input_decimals = infer_mint_decimals(&input_mint);
    let amount_in = (amount_str.parse::<f64>().expect("Invalid AMOUNT")
        * 10f64.powi(input_decimals as i32)) as u64;

    let keypair = Keypair::from_base58_string(&env::var("PRIVATE_KEY").expect("PRIVATE_KEY"));
    let user = keypair.pubkey();
    let surfpool_url = env::var("SURFPOOL_URL").unwrap_or_else(|_| "http://127.0.0.1:8899".into());
    // Signing safety: this test signs and sends with PRIVATE_KEY; it must
    // only ever talk to a local fork.
    assert!(
        !surfpool_url.to_ascii_lowercase().contains("mainnet"),
        "SURFPOOL_URL looks like a mainnet endpoint; refusing to sign/send"
    );
    let rpc_url = env::var("RPC_URL").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".into());

    let rpc = RpcClient::new_with_commitment(surfpool_url, CommitmentConfig::confirmed());
    let mainnet = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());

    println!("Thunder - Dynamic Surfpool Swap (Router)");
    println!("========================================\n");
    println!("Wallet:  {user}");
    println!("Input:   {input_str}");
    println!("Output:  {}", &output_mint.to_string()[..12]);
    println!("Amount:  {amount_str} ({amount_in} raw)");
    println!("Hops:    {max_hops}\n");

    let sol_before = rpc.get_balance(&user).await.unwrap_or(0);
    println!("SOL balance: {:.4}\n", sol_before as f64 / 1e9);

    // Check if we already have the output token
    let out_prog_check = if let Ok(acc) = mainnet.get_account(&output_mint).await {
        acc.owner
    } else {
        Pubkey::from_str_const(thunder_core::TOKEN_PROGRAM)
    };
    let out_ata_check = spl_associated_token_account::get_associated_token_address_with_program_id(
        &user, &output_mint, &out_prog_check,
    );
    let output_before = rpc.get_account(&out_ata_check).await.ok()
        .filter(|a| a.data.len() >= 72)
        .map(|a| u64::from_le_bytes(a.data[64..72].try_into().unwrap()))
        .unwrap_or(0);

    // ── Load pools ──────────────────────────────────────────────────────
    let cache_path = PathBuf::from(env::var("CACHE_PATH").unwrap_or_else(|_| "pools.cache".into()));
    let index = match cache::load_cache(&cache_path) {
        Ok((idx, _)) => { println!("Loaded {} pools from cache", idx.pool_count()); idx }
        Err(_) => {
            println!("Loading from RPC...");
            let cb: loader::ProgressCallback = Box::new(|_| {});
            let idx = loader::PoolLoader::new(&rpc_url).load_all(&cb).await.expect("Load failed");
            let _ = cache::save_cache(&idx, &cache_path);
            println!("Loaded {} pools", idx.pool_count()); idx
        }
    };

    // ── Pre-fetch all DLMM bitmap extensions (53 on-chain) ──────────────
    let bitmap_map = fetch_all_bitmap_extensions(&mainnet).await;
    println!("Bitmap extensions: {}\n", bitmap_map.len());

    // ── Find route ──────────────────────────────────────────────────────
    println!("Finding route (max {max_hops} hops)...\n");
    let router = Router::new(&index, max_hops);
    let quote = router.find_routes(input_mint, output_mint, amount_in, 200).expect("Route failed");

    if quote.routes.is_empty() {
        println!("No routes found.");
        return;
    }

    for (i, route) in quote.routes.iter().take(5).enumerate() {
        print_route(i + 1, route, if i == 0 { " <- best" } else { "" });
    }
    if quote.routes.len() > 5 {
        println!("  ... and {} more routes\n", quote.routes.len() - 5);
    }

    // Pre-fetch CLMM tick arrays in parallel for all unique CLMM pools.
    let mut clmm_pools: Vec<(Pubkey, i32, u16)> = Vec::new();
    for route in &quote.routes {
        for hop in &route.hops {
            if hop.dex_name == "Raydium CLMM" {
                let pk = Pubkey::from_str(&hop.pool_address).unwrap();
                if !clmm_pools.iter().any(|(p, _, _)| *p == pk) {
                    if let Ok(acc) = rpc.get_account(&pk).await {
                        if acc.data.len() >= 273 {
                            let ts = u16::from_le_bytes(acc.data[235..237].try_into().unwrap());
                            let tc = i32::from_le_bytes(acc.data[269..273].try_into().unwrap());
                            clmm_pools.push((pk, tc, ts));
                        }
                    }
                }
            }
        }
    }
    println!("Fetching tick arrays for {} CLMM pools (parallel)...", clmm_pools.len());
    let tick_futures: Vec<_> = clmm_pools.iter()
        .map(|(pk, tc, ts)| fetch_clmm_tick_arrays(&mainnet, pk, *tc, *ts))
        .collect();
    let tick_results = futures::future::join_all(tick_futures).await;
    let mut tick_array_map: HashMap<Pubkey, Vec<Pubkey>> = HashMap::new();
    for ((pk, _, _), tas) in clmm_pools.iter().zip(tick_results) {
        if !tas.is_empty() {
            tick_array_map.insert(*pk, tas);
        }
    }
    println!("Found tick arrays for {}/{} pools", tick_array_map.len(), clmm_pools.len());

    // Pre-filter routes: skip routes with unsupported DEXes or missing tick arrays.
    let viable_routes: Vec<&Route> = quote.routes.iter().filter(|route| {
        route.hops.iter().all(|hop| {
            if !SUPPORTED_DEXES.contains(&hop.dex_name.as_str()) {
                return false;
            }
            match hop.dex_name.as_str() {
                "Raydium CLMM" => {
                    let pk = Pubkey::from_str(&hop.pool_address).unwrap();
                    tick_array_map.contains_key(&pk)
                }
                _ => true,
            }
        })
    }).collect();
    println!("{} viable routes (filtered from {})\n", viable_routes.len(), quote.routes.len());

    // ── Assemble the builder inputs (live surfpool state) ──────────────
    let store = AccountStore::new();
    populate_store(&rpc, &viable_routes, &store).await;
    let registry = build_registry_for_routes(&index, &viable_routes, &bitmap_map, &tick_array_map);
    println!("AccountStore: {} accounts, registry: {} pools\n", store.len(), registry.pool_count());

    // ── Swap ALT: load SWAP_ALT_ADDRESS or create the curated table ─────
    let alts: Vec<AddressLookupTableAccount> =
        obtain_swap_alt(&rpc, &keypair).await.into_iter().collect();

    // ── Try each viable route through the production bundle path ───────
    for (ri, route) in viable_routes.iter().enumerate() {
        let hops_desc: Vec<String> = route.hops.iter().map(|h| format!("{}({})", &h.pool_address[..6], h.dex_name.chars().take(4).collect::<String>())).collect();
        println!("--- Route {} [{}] ---", ri + 1, hops_desc.join(" -> "));

        let min_out = calculate_min_amount_out(route.output_amount, slippage_bps);
        let bundle = match build_swap_instructions(
            route, &user, amount_in, min_out, &store, &registry, &SwapOptions::default(),
        ) {
            Ok(bundle) => bundle,
            Err(e) => {
                println!("  Builder failed: {e}");
                continue;
            }
        };

        let blockhash = rpc.get_latest_blockhash().await.expect("Blockhash");

        // Size comparison: no-ALT compile may legitimately exceed 1232.
        let no_alt_desc = match compile_to_transaction(&bundle, &user, &[], blockhash) {
            Ok(tx) => format!("{} bytes", bincode::serialize(&tx).map(|b| b.len()).unwrap_or(0)),
            Err(e) => format!("unusable ({e})"),
        };
        let mut tx = match compile_to_transaction(&bundle, &user, &alts, blockhash) {
            Ok(tx) => tx,
            Err(e) => {
                println!("  Compile with ALT failed: {e} (no-ALT: {no_alt_desc})");
                continue;
            }
        };
        tx.signatures[0] = keypair.sign_message(tx.message.serialize().as_slice());

        let tx_size = bincode::serialize(&tx).map(|b| b.len()).unwrap_or(0);
        println!(
            "  tx size: {tx_size} bytes with ALT ({} table{}) | without ALT: {no_alt_desc} | cu_limit={}",
            alts.len(),
            if alts.len() == 1 { "" } else { "s" },
            bundle.compute_unit_limit,
        );

        // Simulate first to get full logs on failure
        if ri < 3 {
            let sim_config = solana_rpc_client_api::config::RpcSimulateTransactionConfig {
                sig_verify: false,
                commitment: Some(CommitmentConfig::confirmed()),
                replace_recent_blockhash: false,
                ..Default::default()
            };
            if let Ok(sim) = rpc.simulate_transaction_with_config(&tx, sim_config).await {
                if let Some(err) = &sim.value.err {
                    println!("  SIM ERROR: {err:?}");
                }
                if let Some(logs) = &sim.value.logs {
                    for (li, log) in logs.iter().enumerate() {
                        if log.contains("failed") || log.contains("Error") || log.contains("unauthorized") || log.contains("invoke") || log.contains("Program log") {
                            println!("  LOG[{li:>2}]: {log}");
                        }
                    }
                }
            }
        }

        match rpc.send_and_confirm_transaction(&tx).await {
            Ok(sig) => {
                println!("\n  SWAP SUCCEEDED!");
                println!("  Signature: {sig}\n");

                // Show before/after diff
                let sol_after = rpc.get_balance(&user).await.unwrap_or(0);
                let output_after = rpc.get_account(&out_ata_check).await.ok()
                    .filter(|a| a.data.len() >= 72)
                    .map(|a| u64::from_le_bytes(a.data[64..72].try_into().unwrap()))
                    .unwrap_or(0);

                let out_dec = infer_mint_decimals(&output_mint);
                let out_diff = output_after.saturating_sub(output_before);
                let sol_diff = sol_before.saturating_sub(sol_after);

                println!("  ┌─────────────────────────────────────────┐");
                println!("  │  SOL   : -{:.6} ({:.4} -> {:.4})", sol_diff as f64 / 1e9, sol_before as f64 / 1e9, sol_after as f64 / 1e9);
                println!("  │  Token : +{:.6} ({:.6} -> {:.6})",
                    out_diff as f64 / 10f64.powi(out_dec as i32),
                    output_before as f64 / 10f64.powi(out_dec as i32),
                    output_after as f64 / 10f64.powi(out_dec as i32),
                );
                println!("  └─────────────────────────────────────────┘");

                // wSOL unwrap check: whenever SOL was on either side, the
                // WSOL ATA must be closed post-swap.
                let wsol = Pubkey::from_str_const(WSOL);
                if input_mint == wsol || output_mint == wsol {
                    let wsol_ata =
                        spl_associated_token_account::get_associated_token_address(&user, &wsol);
                    match rpc.get_account(&wsol_ata).await {
                        Ok(acc) if acc.lamports > 0 => {
                            println!("  WARNING: wSOL ATA still open ({} lamports)", acc.lamports)
                        }
                        _ => println!("  wSOL ATA closed (unwrap OK)"),
                    }
                }
                return;
            }
            Err(e) => {
                let msg = e.to_string();
                // Extract log messages count and key error info
                let log_count = msg.matches("Program log:").count()
                    + msg.matches("Program ").count();
                if let Some(pos) = msg.find("custom program error") {
                    let end = (pos + 50).min(msg.len());
                    println!("  Failed: {}: {} log messages", &msg[pos..end], log_count);
                    // Print first few log lines for debugging
                    for line in msg.lines().filter(|l| l.contains("Program log:") || l.contains("Error")).take(5) {
                        println!("    {}", line.trim());
                    }
                } else {
                    println!("  Failed: {}", &msg[..msg.len().min(200)]);
                }
            }
        }
    }
    println!("\nAll {} viable routes exhausted ({} total found).", viable_routes.len(), quote.routes.len());
}

// =========================================================================
// Helpers (unchanged)
// =========================================================================

async fn fetch_all_bitmap_extensions(rpc: &RpcClient) -> HashMap<Pubkey, Pubkey> {
    let dlmm = Pubkey::from_str_const(DLMM_PROGRAM);
    let mut map = HashMap::new();

    #[allow(deprecated)]
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::DataSize(12488)]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            data_slice: Some(solana_account_decoder_client_types::UiDataSliceConfig { offset: 8, length: 32 }),
            ..Default::default()
        },
        ..Default::default()
    };

    #[allow(deprecated)]
    if let Ok(accounts) = rpc.get_program_accounts_with_config(&dlmm, config).await {
        for (ext_pubkey, account) in accounts {
            if account.data.len() >= 32 {
                if let Ok(pool_pubkey) = Pubkey::try_from(&account.data[..32]) {
                    map.insert(pool_pubkey, ext_pubkey);
                }
            }
        }
    }

    map
}

/// Fetch initialized tick arrays for a CLMM pool from mainnet.
/// Returns the 3 tick arrays closest to the current tick (sorted by proximity).
async fn fetch_clmm_tick_arrays(
    rpc: &RpcClient,
    pool: &Pubkey,
    tick_current: i32,
    tick_spacing: u16,
) -> Vec<Pubkey> {
    let clmm = Pubkey::from_str_const(CLMM_PROGRAM);

    #[allow(deprecated)]
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![
            RpcFilterType::Memcmp(Memcmp::new_raw_bytes(8, pool.to_bytes().to_vec())),
        ]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            data_slice: Some(solana_account_decoder_client_types::UiDataSliceConfig {
                offset: 8,
                length: 36,
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    #[allow(deprecated)]
    let accounts = match rpc.get_program_accounts_with_config(&clmm, config).await {
        Ok(a) => a,
        Err(_) => return vec![],
    };

    let tick_per_array = tick_spacing as i32 * 60;
    let mut tick_array_info: Vec<(Pubkey, i32)> = accounts.iter().filter_map(|(pubkey, acc)| {
        if acc.data.len() >= 36 {
            let start = i32::from_le_bytes(acc.data[32..36].try_into().ok()?);
            Some((*pubkey, start))
        } else {
            None
        }
    }).collect();

    let current_start = tick_current.div_euclid(tick_per_array) * tick_per_array;
    tick_array_info.sort_by_key(|(_, start)| ((*start - current_start) as i64).abs());

    tick_array_info.into_iter().take(3).map(|(pk, _)| pk).collect()
}

fn parse_mint(s: &str) -> Pubkey {
    if s.eq_ignore_ascii_case("SOL") || s.eq_ignore_ascii_case("WSOL") {
        Pubkey::from_str_const(WSOL)
    } else {
        Pubkey::from_str(s).unwrap_or_else(|_| panic!("Invalid mint: {s}"))
    }
}

fn print_route(num: usize, route: &Route, tag: &str) {
    let n = route.hops.len();
    println!("  Route {num} ({n} hop{}):{tag}", if n == 1 { "" } else { "s" });
    for (j, hop) in route.hops.iter().enumerate() {
        let id = infer_mint_decimals(&hop.input_mint);
        let od = infer_mint_decimals(&hop.output_mint);
        println!(
            "    {}: {:.6} {} -> {:.6} {} via {}..{} ({})",
            j + 1,
            hop.input_amount as f64 / 10f64.powi(id as i32),
            &hop.input_mint.to_string()[..6],
            hop.output_amount as f64 / 10f64.powi(od as i32),
            &hop.output_mint.to_string()[..6],
            &hop.pool_address[..6], &hop.pool_address[hop.pool_address.len()-4..],
            hop.dex_name,
        );
    }
    let od = infer_mint_decimals(&route.output_mint);
    println!("    -> {:.6}\n", route.output_amount as f64 / 10f64.powi(od as i32));
}
