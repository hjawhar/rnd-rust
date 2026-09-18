use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use solana_commitment_config::CommitmentConfig;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use tokio::sync::RwLock;

use thunder_aggregator::cache;
use thunder_aggregator::loader::PoolLoader;
use thunder_aggregator::price;

use thunder_engine::account_store::AccountStore;
use thunder_engine::api::{self, AppState};
use thunder_engine::cold_start;
use thunder_engine::pool_registry::PoolRegistry;
use thunder_engine::streaming;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let rpc_url = env::var("RPC_URL").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".into());
    let cache_path = PathBuf::from(env::var("CACHE_PATH").unwrap_or_else(|_| "pools.cache".into()));
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);

    println!("Thunder Engine");
    println!("==============");

    // 1. Load pools from cache or RPC.
    let (pool_index, loaded_from_cache) = if cache_path.exists() {
        println!("Loading pools from cache: {}", cache_path.display());
        match cache::load_cache(&cache_path) {
            Ok((index, ts)) => {
                println!(
                    "Loaded {} pools from cache (timestamp {})",
                    index.pool_count(),
                    ts
                );
                (index, true)
            }
            Err(e) => {
                eprintln!("Cache load failed: {e}, falling back to RPC");
                let index = load_from_rpc(&rpc_url).await;
                (index, false)
            }
        }
    } else {
        println!("No cache found at {}, loading from RPC", cache_path.display());
        let index = load_from_rpc(&rpc_url).await;
        (index, false)
    };

    println!("Pool index: {} pools", pool_index.pool_count());

    // 2. Build PoolRegistry and validate immediately from cached vault balances.
    //    No RPC needed — the market objects already have balances from the cache.
    let mut registry = PoolRegistry::from_pool_index(&pool_index);
    registry.validate_from_cache();
    println!(
        "Registry: {} pools, {} swappable, {} unique mints",
        registry.pool_count(),
        registry.swappable_count(),
        registry.unique_mints()
    );

    // 3. Build shared state. Server starts serving quotes immediately using
    //    cached balances. Background task fetches fresh vault data and re-validates.
    let store = Arc::new(AccountStore::new());
    let pool_index = Arc::new(pool_index);
    let rpc = Arc::new(RpcClient::new_with_timeout_and_commitment(
        rpc_url.clone(),
        Duration::from_secs(30),
        CommitmentConfig::confirmed(),
    ));
    let state = Arc::new(AppState {
        store: store.clone(),
        pool_index: pool_index.clone(),
        registry: Arc::new(RwLock::new(registry)),
        rpc: rpc.clone(),
        sol_usd_price: RwLock::new(None),
        recent_blockhash: RwLock::new(None),
        alt: RwLock::new(None),
        start_time: Instant::now(),
    });

    // 4. Start gRPC streaming in background.
    // start_streaming holds a !Send future (Box<dyn Error> without Send bound),
    // so we run it on a dedicated single-threaded runtime.
    let store_bg = store.clone();
    let registry_bg = state.registry.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build streaming runtime");
        rt.block_on(streaming::start_streaming(store_bg, registry_bg));
    });

    // 5. Background: fetch fresh vault data, auxiliary accounts, re-validate.
    //    Vault fetching (4M+ accounts) is the only slow phase.
    //    SKIP_COLD_START=1 serves from cached balances only — used when the
    //    RPC is a lazy mainnet fork (surfpool) that would proxy-fetch every
    //    vault from the upstream datasource.
    let skip_cold_start = env::var("SKIP_COLD_START").is_ok_and(|v| v == "1" || v == "true");
    if skip_cold_start {
        println!("[cold_start] SKIP_COLD_START set: serving from cached vault balances");
    }
    let cold_state = state.clone();
    let cold_rpc_url = rpc_url.clone();
    if !skip_cold_start { tokio::spawn(async move {
        let rpc = RpcClient::new_with_timeout_and_commitment(
            cold_rpc_url,
            std::time::Duration::from_secs(120),
            CommitmentConfig::confirmed(),
        );

        println!("[cold_start] starting background vault fetch");

        // Fetch fresh vault balances (slow, read lock only).
        {
            let reg = cold_state.registry.read().await;
            cold_start::fetch_all_vaults(&rpc, &reg, &cold_state.store).await;

            // Fee configs (CLMM amm_config tiers + pumpswap global config)
            // and DAMM V1 vault-share accounts (vault states, lp mints,
            // per-pool vault-LP token accounts). Without the latter, V1
            // pools are unquotable by design — their vault token balances
            // are shared across pools and are NOT pool reserves.
            cold_start::fetch_fee_configs(&rpc, &cold_state.store).await;
            cold_start::fetch_damm_v1_vault_accounts(&rpc, &reg, &cold_state.store).await;
        }

        // Auxiliary accounts + re-validate (fast, write lock).
        {
            let mut reg = cold_state.registry.write().await;
            cold_start::fetch_bitmap_extensions(&rpc, &mut reg, &cold_state.store).await;
            cold_start::fetch_tick_arrays(&rpc, &mut reg, &cold_state.store).await;
            cold_start::fetch_dlmm_bin_arrays(&rpc, &mut reg, &cold_state.store).await;
            reg.validate_all(&cold_state.store);

            println!("[cold_start] === cold start complete ===");
            println!("[cold_start] total accounts in store: {}", cold_state.store.len());
            println!(
                "[cold_start] swappable pools: {}/{}",
                reg.swappable_count(),
                reg.pool_count()
            );
            for (dex, count) in reg.dex_counts() {
                let swappable = reg
                    .iter_pools()
                    .filter(|(_, info)| info.dex_name == *dex && info.swappable)
                    .count();
                println!("[cold_start]   {dex}: {swappable}/{count} swappable");
            }
        }

        // Save cache if loaded from RPC.
        if !loaded_from_cache {
            println!("Saving pool cache to {}", cache_path.display());
            match cache::save_cache(&pool_index, &cache_path) {
                Ok(n) => println!("Saved {n} pools to cache"),
                Err(e) => eprintln!("Cache save failed: {e}"),
            }
        }
    }); }

    // 6. SOL/USD price refresh every 15s.
    let price_state = state.clone();
    let price_rpc_url = rpc_url.clone();
    tokio::spawn(async move {
        let rpc = RpcClient::new_with_timeout_and_commitment(
            price_rpc_url,
            Duration::from_secs(10),
            CommitmentConfig::confirmed(),
        );
        loop {
            if let Some(p) = price::fetch_sol_usd_onchain(&rpc).await {
                *price_state.sol_usd_price.write().await = Some(p);
            }
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    });

    // 7. Blockhash refresh every 2s.
    let bh_state = state.clone();
    let bh_rpc_url = rpc_url.clone();
    tokio::spawn(async move {
        let rpc = RpcClient::new_with_timeout_and_commitment(
            bh_rpc_url,
            Duration::from_secs(5),
            CommitmentConfig::confirmed(),
        );
        loop {
            if let Ok((bh, last_valid_block_height)) = rpc
                .get_latest_blockhash_with_commitment(CommitmentConfig::confirmed())
                .await
            {
                *bh_state.recent_blockhash.write().await = Some((bh, last_valid_block_height));
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    // 8. Swap ALT: load from SWAP_ALT_ADDRESS on startup, refresh every 10 min.
    if let Ok(alt_str) = env::var("SWAP_ALT_ADDRESS") {
        match alt_str.parse::<solana_pubkey::Pubkey>() {
            Ok(alt_address) => {
                let alt_state = state.clone();
                let alt_rpc_url = rpc_url.clone();
                tokio::spawn(async move {
                    let rpc = RpcClient::new_with_timeout_and_commitment(
                        alt_rpc_url,
                        Duration::from_secs(10),
                        CommitmentConfig::confirmed(),
                    );
                    loop {
                        match thunder_engine::alt::load_alt(&rpc, &alt_address).await {
                            Ok(alt) => {
                                println!("[alt] loaded {} ({} addresses)", alt.key, alt.addresses.len());
                                *alt_state.alt.write().await = Some(alt);
                            }
                            Err(e) => eprintln!("[alt] failed to load {alt_address}: {e}"),
                        }
                        tokio::time::sleep(Duration::from_secs(600)).await;
                    }
                });
            }
            Err(e) => eprintln!("[alt] invalid SWAP_ALT_ADDRESS: {e} (continuing without ALT)"),
        }
    } else {
        println!("[alt] SWAP_ALT_ADDRESS not set; /swap compiles without an ALT");
    }

    // 9. Start HTTP server (blocks main task).
    let app = api::create_router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .expect("failed to bind TCP listener");
    println!("Serving on http://0.0.0.0:{port}");
    thunder_engine::axum::serve(listener, app).await.expect("HTTP server error");
}

async fn load_from_rpc(rpc_url: &str) -> thunder_aggregator::pool_index::PoolIndex {
    let loader = PoolLoader::new(rpc_url);
    let cb: thunder_aggregator::loader::ProgressCallback = Box::new(|progress| {
        println!("[loader] {}: {:?}", progress.dex_name, progress.phase);
    });
    loader
        .load_all(&cb)
        .await
        .expect("failed to load pools from RPC")
}
