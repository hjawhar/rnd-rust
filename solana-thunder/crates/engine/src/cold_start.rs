#![allow(deprecated)]

use std::collections::HashMap;
use std::str::FromStr;

use futures::future::join_all;
use solana_account_decoder_client_types::UiAccountEncoding;
use solana_commitment_config::CommitmentConfig;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_rpc_client_api::filter::RpcFilterType;

use crate::account_store::AccountStore;
use crate::pool_registry::PoolRegistry;

const BATCH_SIZE: usize = 100;
const BATCH_CONCURRENCY: usize = 100;

const DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
const CLMM_PROGRAM_ID: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
const PUMPFUN_AMM_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
/// Raydium CLMM AmmConfig account size (disc 8 + bump 1 + index 2 + owner 32
/// + protocol_fee_rate 4 + trade_fee_rate 4 + tick_spacing 2 + fund_fee_rate
/// 4 + padding). Verified against mainnet dumps.
const CLMM_AMM_CONFIG_SIZE: u64 = 117;

/// Fetch all vault accounts from the registry and store them in the AccountStore.
pub async fn fetch_all_vaults(
    rpc: &RpcClient,
    registry: &PoolRegistry,
    store: &AccountStore,
) {
    let vault_keys: Vec<Pubkey> = registry
        .iter_pools()
        .flat_map(|(_, info)| [info.quote_vault, info.base_vault])
        .collect();

    let total = vault_keys.len();
    println!("[cold_start] fetching {} vault accounts", total);

    let chunks: Vec<(usize, &[Pubkey])> = vault_keys.chunks(BATCH_SIZE).enumerate().collect();
    let mut fetched = 0usize;
    let mut last_print = 0usize;

    for window in chunks.chunks(BATCH_CONCURRENCY) {
        let futures: Vec<_> = window
            .iter()
            .map(|(_, chunk)| rpc.get_multiple_accounts(chunk))
            .collect();
        let results = join_all(futures).await;

        for ((_, chunk), result) in window.iter().zip(results) {
            match result {
                Ok(accounts) => {
                    for (pubkey, maybe_account) in chunk.iter().zip(accounts) {
                        if let Some(account) = maybe_account {
                            store.upsert(
                                *pubkey,
                                account.data,
                                account.owner,
                                account.lamports,
                                0,
                            );
                            fetched += 1;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[cold_start] vault batch error: {e}");
                }
            }
        }

        // Print progress every ~500k accounts.
        if fetched - last_print >= 500_000 {
            println!("[cold_start] vaults: {fetched}/{total}");
            last_print = fetched;
        }
    }

    println!("[cold_start] vaults done: {fetched}/{total} stored");
}

/// Fetch CLMM tick arrays by deriving PDAs from each pool's bitmap, then
/// batch-fetching with getMultipleAccounts. Replaces the previous single-GPA
/// approach which failed on large response payloads.
pub async fn fetch_tick_arrays(
    rpc: &RpcClient,
    registry: &mut PoolRegistry,
    store: &AccountStore,
) {
    // Collect CLMM pools sorted by vault balance descending.
    let mut clmm_pools: Vec<(&str, u64)> = registry
        .iter_pools()
        .filter(|(_, info)| info.dex_name == "Raydium CLMM")
        .map(|(addr, info)| {
            let balance = store.read_token_balance(&info.quote_vault)
                + store.read_token_balance(&info.base_vault);
            (addr, balance)
        })
        .collect();

    clmm_pools.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    clmm_pools.truncate(10_000);

    if clmm_pools.is_empty() {
        println!("[cold_start] no CLMM pools found, skipping tick arrays");
        return;
    }

    // Derive tick array PDAs from each pool's cached data (bitmap + current tick).
    let pool_addrs: Vec<String> = clmm_pools.iter().map(|(a, _)| a.to_string()).collect();
    let mut pool_tick_map: HashMap<String, Vec<Pubkey>> = HashMap::new();
    let mut all_pdas: Vec<Pubkey> = Vec::new();

    for addr in &pool_addrs {
        let cached_data = match registry.get_pool(addr).map(|i| &i.cached_data) {
            Some(d) if !d.is_empty() => d,
            _ => continue,
        };
        if let Some((_pool_id, pdas)) = thunder_aggregator::cache::extract_clmm_tick_pdas(cached_data) {
            pool_tick_map.insert(addr.clone(), pdas.clone());
            all_pdas.extend(pdas);
        }
    }

    // Deduplicate.
    all_pdas.sort();
    all_pdas.dedup();

    println!(
        "[cold_start] fetching {} tick array accounts for {} CLMM pools",
        all_pdas.len(),
        pool_tick_map.len()
    );

    // Batch fetch with getMultipleAccounts.
    let mut fetched = 0usize;
    let chunks: Vec<&[Pubkey]> = all_pdas.chunks(BATCH_SIZE).collect();

    for window in chunks.chunks(BATCH_CONCURRENCY) {
        let futures: Vec<_> = window
            .iter()
            .map(|chunk| rpc.get_multiple_accounts(chunk))
            .collect();
        let results = join_all(futures).await;

        for (chunk, result) in window.iter().zip(results) {
            match result {
                Ok(accounts) => {
                    for (pubkey, maybe_account) in chunk.iter().zip(accounts) {
                        if let Some(account) = maybe_account {
                            store.upsert(
                                *pubkey,
                                account.data,
                                account.owner,
                                account.lamports,
                                0,
                            );
                            fetched += 1;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[cold_start] tick array batch error: {e}");
                }
            }
        }
    }

    // Assign tick arrays to pool infos, keeping only those that exist on-chain.
    for (addr, pdas) in &pool_tick_map {
        if let Some(pool_info) = registry.get_pool_mut(addr) {
            pool_info.tick_arrays = pdas
                .iter()
                .filter(|pk| store.contains(pk))
                .cloned()
                .collect();
        }
    }

    println!(
        "[cold_start] tick arrays: {fetched} stored across {} pools",
        pool_tick_map.len()
    );
}

/// Fetch DLMM bin array accounts for the active bin of each pool.
/// Derives bin array PDAs from cached pool data, batch-fetches them,
/// and records the PDA on each PoolInfo so the swappable check can
/// verify existence in the store without re-deriving.
pub async fn fetch_dlmm_bin_arrays(
    rpc: &RpcClient,
    registry: &mut PoolRegistry,
    store: &AccountStore,
) {
    // Derive bin array PDAs for all DLMM pools.
    let mut pda_map: HashMap<String, Pubkey> = HashMap::new(); // pool_addr -> bin_array PDA
    let mut all_pdas: Vec<Pubkey> = Vec::new();

    for (addr, info) in registry.iter_pools() {
        if info.dex_name != "Meteora DLMM" {
            continue;
        }
        // Skip pools that already have a bitmap extension (they pass the
        // swappable check without a bin array).
        if info.bitmap_ext.is_some() {
            continue;
        }
        if let Some((_pool_pk, pda)) = thunder_aggregator::cache::extract_dlmm_bin_pda(&info.cached_data) {
            pda_map.insert(addr.to_string(), pda);
            all_pdas.push(pda);
        }
    }

    all_pdas.sort();
    all_pdas.dedup();

    if all_pdas.is_empty() {
        return;
    }

    println!(
        "[cold_start] fetching {} DLMM bin array accounts for {} pools",
        all_pdas.len(),
        pda_map.len()
    );

    let mut fetched = 0usize;
    let chunks: Vec<&[Pubkey]> = all_pdas.chunks(BATCH_SIZE).collect();

    for window in chunks.chunks(BATCH_CONCURRENCY) {
        let futures: Vec<_> = window
            .iter()
            .map(|chunk| rpc.get_multiple_accounts(chunk))
            .collect();
        let results = join_all(futures).await;

        for (chunk, result) in window.iter().zip(results) {
            match result {
                Ok(accounts) => {
                    for (pubkey, maybe_account) in chunk.iter().zip(accounts) {
                        if let Some(account) = maybe_account {
                            store.upsert(
                                *pubkey,
                                account.data,
                                account.owner,
                                account.lamports,
                                0,
                            );
                            fetched += 1;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[cold_start] DLMM bin array batch error: {e}");
                }
            }
        }
    }

    // Record the bin array PDA on each pool info.
    for (addr, pda) in &pda_map {
        if let Some(pool_info) = registry.get_pool_mut(addr) {
            pool_info.bin_array = Some(*pda);
        }
    }

    println!("[cold_start] DLMM bin arrays: {fetched} stored for {} pools", pda_map.len());
}

/// Fetch all DLMM bitmap extension accounts (dataSize=12488) and assign to pools.
pub async fn fetch_bitmap_extensions(
    rpc: &RpcClient,
    registry: &mut PoolRegistry,
    store: &AccountStore,
) {
    let dlmm_program = match Pubkey::from_str(DLMM_PROGRAM_ID) {
        Ok(pk) => pk,
        Err(_) => return,
    };

    let config = RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::DataSize(12488)]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            commitment: Some(CommitmentConfig::confirmed()),
            ..Default::default()
        },
        ..Default::default()
    };

    let accounts = match rpc
        .get_program_accounts_with_config(&dlmm_program, config)
        .await
    {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[cold_start] bitmap extension GPA error: {e}");
            return;
        }
    };

    println!(
        "[cold_start] fetched {} bitmap extension accounts",
        accounts.len()
    );

    // Build a reverse lookup: pool_pubkey -> pool address string.
    let pool_lookup: HashMap<Pubkey, String> = registry
        .iter_pools()
        .filter(|(_, info)| info.dex_name == "Meteora DLMM")
        .filter_map(|(addr, _)| Pubkey::from_str(addr).ok().map(|pk| (pk, addr.to_string())))
        .collect();

    let mut matched = 0usize;

    for (pubkey, account) in accounts {
        if account.data.len() < 40 {
            continue;
        }

        // lb_pair (pool address) at offset 8, 32 bytes.
        let lb_pair = Pubkey::try_from(&account.data[8..40]).unwrap();

        store.upsert(pubkey, account.data, account.owner, account.lamports, 0);

        if let Some(pool_addr) = pool_lookup.get(&lb_pair) {
            if let Some(pool_info) = registry.get_pool_mut(pool_addr) {
                pool_info.bitmap_ext = Some(pubkey);
                matched += 1;
            }
        }
    }

    println!("[cold_start] bitmap extensions: {matched} matched to pools");
}

/// Fetch DEX fee-config accounts into the store so the live quote math can
/// use real fees:
/// - Raydium CLMM AmmConfig accounts (GPA dataSize=117, a few dozen): the
///   pool's `amm_config` pubkey resolves against these for `trade_fee_rate`.
/// - Pumpswap GlobalConfig (single account): lp/protocol/coin_creator fee bps.
///
/// Additive and non-critical: quote math falls back to defaults when absent.
pub async fn fetch_fee_configs(rpc: &RpcClient, store: &AccountStore) {
    // Raydium CLMM amm_config accounts.
    if let Ok(clmm_program) = Pubkey::from_str(CLMM_PROGRAM_ID) {
        let config = RpcProgramAccountsConfig {
            filters: Some(vec![RpcFilterType::DataSize(CLMM_AMM_CONFIG_SIZE)]),
            account_config: RpcAccountInfoConfig {
                encoding: Some(UiAccountEncoding::Base64),
                commitment: Some(CommitmentConfig::confirmed()),
                ..Default::default()
            },
            ..Default::default()
        };
        match rpc.get_program_accounts_with_config(&clmm_program, config).await {
            Ok(accounts) => {
                let n = accounts.len();
                for (pubkey, account) in accounts {
                    store.upsert(pubkey, account.data, account.owner, account.lamports, 0);
                }
                println!("[cold_start] CLMM amm_config accounts: {n} stored");
            }
            Err(e) => eprintln!("[cold_start] CLMM amm_config GPA error: {e}"),
        }
    }

    // Pumpswap global config.
    if let Ok(pump_program) = Pubkey::from_str(PUMPFUN_AMM_PROGRAM_ID) {
        let (global_config, _) =
            Pubkey::find_program_address(&[b"global_config"], &pump_program);
        match rpc.get_account(&global_config).await {
            Ok(account) => {
                store.upsert(global_config, account.data, account.owner, account.lamports, 0);
                println!("[cold_start] pumpswap global_config stored");
            }
            Err(e) => eprintln!("[cold_start] pumpswap global_config fetch error: {e}"),
        }
    }
}

/// Fetch the accounts needed for Meteora DAMM V1 vault-share reserve math:
/// unique vault state accounts (shared per token, small set), each vault's
/// LP mint (pubkey read from the vault state at offset 115), and every
/// pool's a_vault_lp/b_vault_lp token accounts (~2 per V1 pool).
///
/// Without these accounts V1 pools are unquotable (calculate_output_live_ex
/// returns Err and the router skips them) — intentional: the vault token
/// account balances are shared across all pools of a token and are NOT this
/// pool's reserves (using them inflated quotes by 100-400x).
pub async fn fetch_damm_v1_vault_accounts(
    rpc: &RpcClient,
    registry: &PoolRegistry,
    store: &AccountStore,
) {
    let mut vault_set: Vec<Pubkey> = Vec::new();
    let mut lp_accounts: Vec<Pubkey> = Vec::new();
    for (_, info) in registry.iter_pools() {
        if info.dex_name != "Meteora DAMM V1" || info.cached_data.is_empty() {
            continue;
        }
        if let Some((a_vault, b_vault, a_lp, b_lp)) =
            thunder_aggregator::cache::extract_damm_v1_vault_keys(&info.cached_data)
        {
            vault_set.push(a_vault);
            vault_set.push(b_vault);
            lp_accounts.push(a_lp);
            lp_accounts.push(b_lp);
        }
    }
    if vault_set.is_empty() {
        return;
    }
    vault_set.sort();
    vault_set.dedup();
    lp_accounts.sort();
    lp_accounts.dedup();

    println!(
        "[cold_start] fetching DAMM V1 vault-share accounts: {} vaults, {} pool lp accounts",
        vault_set.len(),
        lp_accounts.len()
    );

    // 1. Vault state accounts (unique, shared across pools) — also collect
    //    each vault's lp_mint pubkey for the second phase.
    let mut lp_mints: Vec<Pubkey> = Vec::new();
    let mut fetched_vaults = 0usize;
    for chunk in vault_set.chunks(BATCH_SIZE) {
        match rpc.get_multiple_accounts(chunk).await {
            Ok(accounts) => {
                for (pubkey, maybe_account) in chunk.iter().zip(accounts) {
                    if let Some(account) = maybe_account {
                        if account.data.len() >= 147 {
                            if let Ok(pk) = Pubkey::try_from(&account.data[115..147]) {
                                lp_mints.push(pk);
                            }
                        }
                        store.upsert(*pubkey, account.data, account.owner, account.lamports, 0);
                        fetched_vaults += 1;
                    }
                }
            }
            Err(e) => eprintln!("[cold_start] V1 vault batch error: {e}"),
        }
    }
    lp_mints.sort();
    lp_mints.dedup();

    // 2. Vault LP mints + per-pool vault-LP token accounts.
    let mut rest: Vec<Pubkey> = lp_mints;
    rest.extend(lp_accounts);
    let mut fetched_rest = 0usize;
    let chunks: Vec<&[Pubkey]> = rest.chunks(BATCH_SIZE).collect();
    for window in chunks.chunks(BATCH_CONCURRENCY) {
        let futures: Vec<_> = window
            .iter()
            .map(|chunk| rpc.get_multiple_accounts(chunk))
            .collect();
        let results = join_all(futures).await;
        for (chunk, result) in window.iter().zip(results) {
            match result {
                Ok(accounts) => {
                    for (pubkey, maybe_account) in chunk.iter().zip(accounts) {
                        if let Some(account) = maybe_account {
                            store.upsert(
                                *pubkey,
                                account.data,
                                account.owner,
                                account.lamports,
                                0,
                            );
                            fetched_rest += 1;
                        }
                    }
                }
                Err(e) => eprintln!("[cold_start] V1 lp accounts batch error: {e}"),
            }
        }
    }

    println!(
        "[cold_start] DAMM V1 vault-share accounts: {fetched_vaults} vaults + {fetched_rest} lp mints/accounts stored"
    );
}

/// Cold-start orchestrator: fetch all on-chain data and validate pools.
pub async fn cold_start(
    rpc: &RpcClient,
    registry: &mut PoolRegistry,
    store: &AccountStore,
) {
    println!("[cold_start] starting cold start sequence");

    // 1. Vault balances first — everything else depends on these.
    fetch_all_vaults(rpc, registry, store).await;

    // 2. Bitmap extensions (fast, ~43 accounts).
    fetch_bitmap_extensions(rpc, registry, store).await;

    // 2b. Fee configs (fast: one GPA + one account) — real fees for quotes.
    fetch_fee_configs(rpc, store).await;

    // 2c. DAMM V1 vault-share accounts (vault states, lp mints, pool lp
    //     token accounts) — required for V1 quoting.
    fetch_damm_v1_vault_accounts(rpc, registry, store).await;

    // 3. Tick arrays (depends on vault balances for sorting).
    fetch_tick_arrays(rpc, registry, store).await;

    // 4. DLMM bin arrays (depends on bitmap extensions for skip logic).
    fetch_dlmm_bin_arrays(rpc, registry, store).await;

    // 5. Validate all pools against the now-populated store.
    registry.validate_all(store);

    // 6. Summary.
    println!("[cold_start] === cold start complete ===");
    println!("[cold_start] total accounts in store: {}", store.len());
    println!(
        "[cold_start] swappable pools: {}/{}",
        registry.swappable_count(),
        registry.pool_count()
    );

    for (dex, count) in registry.dex_counts() {
        // Count swappable per DEX.
        let swappable = registry
            .iter_pools()
            .filter(|(_, info)| info.dex_name == *dex && info.swappable)
            .count();
        println!("[cold_start]   {dex}: {swappable}/{count} swappable");
    }
}
