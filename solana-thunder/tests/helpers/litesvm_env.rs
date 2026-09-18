//! LiteSVM fixture environment for router integration tests.
//!
//! Loads the account/program fixtures dumped by `thunder-fixtures`
//! (see `bin/fixtures.rs`) into an in-process LiteSVM instance, pins the
//! `Clock` sysvar to the dump slot, and builds the fixture-backed
//! `AccountStore` + `PoolIndex`/`PoolRegistry` that the production
//! `thunder_engine::swap::build_swap_transaction` needs.

use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use borsh::BorshDeserialize;
use litesvm::LiteSVM;
use litesvm::types::TransactionMetadata;
use serde::Deserialize;
use solana_clock::Clock;
use solana_pubkey::Pubkey;
use solana_sdk::account::Account;
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::transaction::VersionedTransaction;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use thunder_aggregator::cache::CachedPool;
use thunder_aggregator::pool_index::PoolIndex;
use thunder_aggregator::types::{PoolEntry, Route, RouteHop};
use thunder_core::{GenericError, SwapDirection, TOKEN_PROGRAM, WSOL};
use thunder_engine::account_store::AccountStore;
use thunder_engine::pool_registry::PoolRegistry;

/// LiteSVM deploy address of the thunder-router program
/// (keypair at crates/router-program/keys/thunder_router-keypair.json).
pub const ROUTER_PROGRAM_ID: &str = "GmVAujPNW7Tysy2Ccmm7ind81XmwD2cXyBtAn5Fk99AV";

const ROUTER_SO: &str = "crates/router-program/target/deploy/thunder_router.so";
const DLMM_PROGRAM: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
const TOKEN_ACCOUNT_RENT: u64 = 2_039_280;

/// Dumped `.so` filename -> mainnet program id (mirror of bin/fixtures.rs).
const PROGRAM_IDS: &[(&str, &str)] = &[
    ("meteora_damm_v2.so", "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG"),
    ("meteora_dlmm.so", "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"),
    ("meteora_damm_v1.so", "Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB"),
    ("meteora_vault.so", meteora_damm::METEORA_VAULT_PROGRAM),
    ("raydium_clmm.so", "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK"),
    ("raydium_amm_v4.so", "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8"),
    ("pumpfun_amm.so", "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA"),
];

#[derive(Deserialize)]
pub struct ManifestPool {
    pub address: String,
    pub dex_name: String,
}

#[derive(Deserialize)]
pub struct Manifest {
    pub slot: u64,
    pub unix_timestamp: i64,
    pub pools: Vec<ManifestPool>,
}

/// Fixture-backed simulation environment.
pub struct FixtureEnv {
    pub svm: LiteSVM,
    pub user: Keypair,
    pub store: AccountStore,
    pub registry: PoolRegistry,
    pub manifest: Manifest,
}

impl FixtureEnv {
    /// Load fixtures into a fresh LiteSVM. Returns `None` (after an
    /// explanatory eprintln) when fixtures are absent, so CI without
    /// fixtures stays green. Panics on malformed fixtures.
    pub fn load() -> Option<Self> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let fixtures = root.join("fixtures");
        let manifest_path = fixtures.join("manifest.json");
        if !manifest_path.exists() {
            eprintln!(
                "skipping: {} not found (run `cargo run --bin thunder-fixtures -- <pool>...`)",
                manifest_path.display()
            );
            return None;
        }
        let router_so = root.join(ROUTER_SO);
        if !router_so.exists() {
            eprintln!(
                "skipping: {} not built (run scripts/build-router.sh)",
                router_so.display()
            );
            return None;
        }
        ensure_router_env();

        let manifest: Manifest = serde_json::from_str(
            &fs::read_to_string(&manifest_path).expect("read manifest.json"),
        )
        .expect("parse manifest.json");

        let mut svm = LiteSVM::new();

        // Pin Clock to the dump slot before loading programs so program
        // cache entries are effective at the pinned slot.
        let mut clock: Clock = svm.get_sysvar();
        clock.slot = manifest.slot;
        clock.unix_timestamp = manifest.unix_timestamp;
        svm.set_sysvar(&clock);

        // Fixture accounts -> LiteSVM + AccountStore.
        let store = AccountStore::new();
        let accounts_dir = fixtures.join("accounts");
        for entry in fs::read_dir(&accounts_dir).expect("read fixtures/accounts") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let (pubkey, account) =
                read_account_json(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            store.upsert(
                pubkey,
                account.data.clone(),
                account.owner,
                account.lamports,
                manifest.slot,
            );
            svm.set_account(pubkey, account).expect("set_account");
        }

        // Dumped DEX programs at their real mainnet ids.
        let programs_dir = fixtures.join("programs");
        let mut loaded_programs = 0usize;
        if programs_dir.exists() {
            for entry in fs::read_dir(&programs_dir).expect("read fixtures/programs") {
                let path = entry.expect("dir entry").path();
                if path.extension().and_then(|e| e.to_str()) != Some("so") {
                    continue;
                }
                let id = program_id_for(&path)
                    .unwrap_or_else(|| panic!("unknown program fixture {}", path.display()));
                svm.add_program_from_file(id, &path)
                    .unwrap_or_else(|e| panic!("load {}: {e:?}", path.display()));
                loaded_programs += 1;
            }
        }
        if loaded_programs == 0 {
            eprintln!("skipping: no .so files in fixtures/programs/");
            return None;
        }

        // The router itself.
        let router_id: Pubkey = ROUTER_PROGRAM_ID.parse().unwrap();
        svm.add_program_from_file(router_id, &router_so)
            .expect("load thunder_router.so");

        // Test user.
        let user = Keypair::new();
        svm.airdrop(&user.pubkey(), 100_000_000_000)
            .expect("airdrop");

        // Fixture-backed PoolIndex -> PoolRegistry.
        let mut index = PoolIndex::new();
        for pool in &manifest.pools {
            let entry = build_pool_entry(pool, &store)
                .unwrap_or_else(|e| panic!("pool {}: {e}", pool.address));
            index.add_pool(pool.address.clone(), entry).expect("add_pool");
        }
        let mut registry = PoolRegistry::from_pool_index(&index);
        registry.validate_all(&store);
        populate_aux_accounts(&mut registry, &manifest, &store);

        let mut env = Self { svm, user, store, registry, manifest };

        // Pre-create user ATAs for every non-native pool mint.
        for mint in env.pool_mints() {
            if mint != Pubkey::from_str_const(WSOL) {
                env.precreate_ata(&mint);
            }
        }

        Some(env)
    }

    /// First manifest pool for the given DEX name.
    pub fn pool_by_dex(&self, dex_name: &str) -> Option<String> {
        self.manifest
            .pools
            .iter()
            .find(|p| p.dex_name == dex_name)
            .map(|p| p.address.clone())
    }

    /// Quote one hop through a fixture pool using the production market math.
    pub fn quote_hop(&self, pool_address: &str, input_mint: Pubkey, amount_in: u64) -> RouteHop {
        let info = self
            .registry
            .get_pool(pool_address)
            .unwrap_or_else(|| panic!("pool {pool_address} not in registry"));
        let (direction, output_mint) = if input_mint == info.quote_mint {
            (SwapDirection::Buy, info.base_mint)
        } else if input_mint == info.base_mint {
            (SwapDirection::Sell, info.quote_mint)
        } else {
            panic!("mint {input_mint} not in pool {pool_address}");
        };

        let pool_pubkey: Pubkey = pool_address.parse().expect("pool address");
        let pool_data = self.store.get_data(&pool_pubkey);
        let quote_bal = self.store.read_token_balance(&info.quote_vault);
        let base_bal = self.store.read_token_balance(&info.base_vault);
        let live = info
            .market
            .calculate_output_live(amount_in, direction, pool_data.as_deref(), quote_bal, base_bal)
            .ok();

        // Cross-check against a spot-price estimate: the CL `_live` quote
        // paths still mix raw amounts with decimal-adjusted prices (fixed by
        // workstream A0), which can be off by 10^(decimal difference). Until
        // then, fall back to the spot estimate whenever live diverges >2x.
        let spot = spot_output_estimate(info, direction, amount_in);
        let output_amount = match (live, spot) {
            (Some(l), Some(s)) if l <= s.saturating_mul(2) && l >= s / 2 => l,
            (live, Some(s)) => {
                eprintln!(
                    "note: {pool_address} live quote {live:?} diverges from spot estimate {s}; using spot (A0 fixes live curve math)"
                );
                s
            }
            (Some(l), None) => l,
            (None, None) => panic!("quote {pool_address}: no live quote and no spot price"),
        };

        RouteHop {
            pool_address: pool_address.to_string(),
            dex_name: info.dex_name.clone(),
            input_mint,
            output_mint,
            input_amount: amount_in,
            output_amount,
            price_impact_bps: 0,
        }
    }

    /// Chain hops through the given pools into a `Route`, quoting each hop
    /// with the fixture data.
    pub fn route(&self, pools: &[&str], input_mint: Pubkey, amount_in: u64) -> Route {
        let mut hops = Vec::with_capacity(pools.len());
        let mut mint = input_mint;
        let mut amount = amount_in;
        for pool_address in pools {
            let hop = self.quote_hop(pool_address, mint, amount);
            mint = hop.output_mint;
            amount = hop.output_amount;
            hops.push(hop);
        }
        Route {
            hops,
            input_mint,
            output_mint: mint,
            input_amount: amount_in,
            output_amount: amount,
            price_impact_bps: 0,
        }
    }

    /// Sign with the test user and execute. On failure, prints the full
    /// transaction logs before returning the error string.
    pub fn sign_and_send(
        &mut self,
        mut tx: VersionedTransaction,
    ) -> Result<TransactionMetadata, String> {
        tx.signatures[0] = self.user.sign_message(&tx.message.serialize());
        self.svm.send_transaction(tx).map_err(|failed| {
            eprintln!("transaction failed: {:?}", failed.err);
            for log in &failed.meta.logs {
                eprintln!("  {log}");
            }
            format!("{:?}", failed.err)
        })
    }

    /// The user's ATA for a mint, using the mint's real owner program.
    pub fn user_ata(&self, mint: &Pubkey) -> Pubkey {
        let owner_prog = self.mint_program(mint);
        get_associated_token_address_with_program_id(&self.user.pubkey(), mint, &owner_prog)
    }

    /// SPL token balance of an account inside the SVM (0 if absent).
    pub fn svm_token_balance(&self, token_account: &Pubkey) -> u64 {
        self.svm
            .get_account(token_account)
            .filter(|a| a.data.len() >= 72)
            .map(|a| u64::from_le_bytes(a.data[64..72].try_into().unwrap()))
            .unwrap_or(0)
    }

    /// All quote/base mints across the fixture pools.
    fn pool_mints(&self) -> Vec<Pubkey> {
        let mut mints = Vec::new();
        for (_, info) in self.registry.iter_pools() {
            for mint in [info.quote_mint, info.base_mint] {
                if !mints.contains(&mint) {
                    mints.push(mint);
                }
            }
        }
        mints
    }

    fn mint_program(&self, mint: &Pubkey) -> Pubkey {
        let tp = Pubkey::from_str_const(TOKEN_PROGRAM);
        self.store.get(mint).map(|a| a.owner).unwrap_or(tp)
    }

    /// Hand-built 165-byte SPL token-account state at the user's ATA.
    fn precreate_ata(&mut self, mint: &Pubkey) {
        let owner_prog = self.mint_program(mint);
        let ata = self.user_ata(mint);
        let mut data = vec![0u8; 165];
        data[0..32].copy_from_slice(mint.as_ref());
        data[32..64].copy_from_slice(self.user.pubkey().as_ref());
        // amount (64..72) = 0, delegate COption (72..108) = None
        data[108] = 1; // state = Initialized
        // is_native (109..121) = None, delegated_amount (121..129) = 0,
        // close_authority (129..165) = None
        self.svm
            .set_account(
                ata,
                Account {
                    lamports: TOKEN_ACCOUNT_RENT,
                    data,
                    owner: owner_prog,
                    executable: false,
                    rent_epoch: u64::MAX,
                },
            )
            .expect("precreate ata");
    }
}

/// Point the production tx builder at the LiteSVM router deploy.
/// `build_swap_transaction` reads ROUTER_PROGRAM_ID once through a OnceLock,
/// so this must run before the first call.
pub fn ensure_router_env() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe { std::env::set_var("ROUTER_PROGRAM_ID", ROUTER_PROGRAM_ID) });
}

// ---------------------------------------------------------------------------
// Fixture parsing
// ---------------------------------------------------------------------------

/// Parse one `fixtures/accounts/<pubkey>.json` file
/// (`solana account --output json` shape).
fn read_account_json(path: &Path) -> Result<(Pubkey, Account), GenericError> {
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    let pubkey: Pubkey = v["pubkey"].as_str().ok_or("missing pubkey")?.parse()?;
    let acc = &v["account"];
    let data = BASE64.decode(acc["data"][0].as_str().ok_or("missing data")?)?;
    Ok((
        pubkey,
        Account {
            lamports: acc["lamports"].as_u64().ok_or("missing lamports")?,
            data,
            owner: acc["owner"].as_str().ok_or("missing owner")?.parse()?,
            executable: acc["executable"].as_bool().unwrap_or(false),
            rent_epoch: acc["rentEpoch"].as_u64().unwrap_or(u64::MAX),
        },
    ))
}

/// Program id for a dumped `.so`: known DEX names first, then a bare
/// `<pubkey>.so` fallback.
fn program_id_for(path: &Path) -> Option<Pubkey> {
    let file_name = path.file_name()?.to_str()?;
    if let Some((_, id)) = PROGRAM_IDS.iter().find(|(name, _)| *name == file_name) {
        return id.parse().ok();
    }
    path.file_stem()?.to_str()?.parse().ok()
}

/// Rebuild a `PoolEntry` (Market trait object) from raw fixture pool bytes,
/// mirroring the per-DEX builders in `crates/aggregator/src/loader.rs`.
fn build_pool_entry(pool: &ManifestPool, store: &AccountStore) -> Result<PoolEntry, GenericError> {
    let pool_pubkey: Pubkey = pool.address.parse()?;
    let data = store
        .get_data(&pool_pubkey)
        .ok_or("pool account missing from fixtures/accounts")?;
    let addr = pool.address.clone();

    let cached = match pool.dex_name.as_str() {
        "Meteora DAMM V2" => {
            let p: meteora_damm::MeteoraDAMMV2Pool = deser_anchor(&data)?;
            let a_bal = store.read_token_balance(&p.token_a_vault);
            let b_bal = store.read_token_balance(&p.token_b_vault);
            CachedPool::MeteoraDAMMV2 { addr, pool: p, a_bal, b_bal }
        }
        "Meteora DLMM" => {
            let p: meteora_dlmm::MeteoraDLMMPool = deser_anchor(&data)?;
            let rx_bal = store.read_token_balance(&p.reserve_x);
            let ry_bal = store.read_token_balance(&p.reserve_y);
            CachedPool::MeteoraDLMM { addr, pool: p, rx_bal, ry_bal }
        }
        "Meteora DAMM V1" => {
            let p: meteora_damm::MeteoraDAMMPool = deser_anchor(&data)?;
            let a_bal =
                store.read_token_balance(&meteora_damm::derive_token_vault_address(p.a_vault).0);
            let b_bal =
                store.read_token_balance(&meteora_damm::derive_token_vault_address(p.b_vault).0);
            CachedPool::MeteoraDAMMV1 { addr, pool: p, a_bal, b_bal }
        }
        "Raydium CLMM" => {
            let p: raydium_clmm::RaydiumCLMMPool = deser_anchor(&data)?;
            let v0_bal = store.read_token_balance(&p.token_vault_0);
            let v1_bal = store.read_token_balance(&p.token_vault_1);
            CachedPool::RaydiumClmm { addr, pool: p, v0_bal, v1_bal }
        }
        "Raydium AMM V4" => {
            let p = raydium_amm_v4::RaydiumAMMV4::try_from_slice(&data)?;
            let quote_bal = store.read_token_balance(&p.quote_vault);
            let base_bal = store.read_token_balance(&p.base_vault);
            CachedPool::RaydiumV4 { addr, pool: p, quote_bal, base_bal }
        }
        "Pumpfun AMM" => {
            let mut p: pumpfun_amm::PumpfunAmmPool = deser_anchor(&data)?;
            // Pumpswap AMM pools are plain constant-product on the real vault
            // balances; synthesize a bonding-curve view from them so the
            // Market math has reserves to work with (A0 replaces this with
            // real curve fidelity).
            let base_bal = store.read_token_balance(&p.pool_base_token_account);
            let quote_bal = store.read_token_balance(&p.pool_quote_token_account);
            p.bonding_curve = Some(pumpfun_amm::PumpfunBondingCurve {
                virtual_token_reserves: base_bal,
                virtual_sol_reserves: quote_bal,
                real_token_reserves: base_bal,
                real_sol_reserves: quote_bal,
                token_total_supply: 0,
                complete: false,
                creator: p.creator,
            });
            CachedPool::PumpfunAmm { addr, pool: p }
        }
        other => return Err(format!("unsupported DEX in manifest: {other}").into()),
    };

    Ok(cached.into_pool_entry().1)
}

/// Fill the DEX-specific auxiliary accounts on each registry `PoolInfo`
/// (normally populated by cold_start): DLMM bitmap extension + active bin
/// array, CLMM tick arrays.
fn populate_aux_accounts(registry: &mut PoolRegistry, manifest: &Manifest, store: &AccountStore) {
    for pool in &manifest.pools {
        let Some(info) = registry.get_pool_mut(&pool.address) else { continue };
        let pool_pubkey: Pubkey = pool.address.parse().expect("pool address");
        match pool.dex_name.as_str() {
            "Meteora DLMM" => {
                let dlmm = Pubkey::from_str_const(DLMM_PROGRAM);
                let (bitmap_ext, _) =
                    Pubkey::find_program_address(&[b"bitmap", pool_pubkey.as_ref()], &dlmm);
                if store.contains(&bitmap_ext) {
                    info.bitmap_ext = Some(bitmap_ext);
                }
                info.bin_array = thunder_aggregator::cache::extract_dlmm_bin_pda(&info.cached_data)
                    .map(|(_, pda)| pda);
            }
            "Raydium CLMM" => {
                if let Some((_, pdas)) =
                    thunder_aggregator::cache::extract_clmm_tick_pdas(&info.cached_data)
                {
                    info.tick_arrays =
                        pdas.into_iter().filter(|pda| store.contains(pda)).collect();
                }
            }
            _ => {}
        }
    }
}

/// Fee-less output estimate from the pool's spot price
/// (`Market::current_price` = quote per base, human units).
fn spot_output_estimate(
    info: &thunder_engine::pool_registry::PoolInfo,
    direction: SwapDirection,
    amount_in: u64,
) -> Option<u64> {
    let price = info.market.current_price().ok()?;
    if !price.is_finite() || price <= 0.0 {
        return None;
    }
    let fin = info.market.financials().ok()?;
    let (out_dec, out_human) = match direction {
        // Buy: quote in, base out.
        SwapDirection::Buy => {
            let in_human = amount_in as f64 / 10f64.powi(fin.quote_decimals as i32);
            (fin.base_decimals, in_human / price)
        }
        // Sell: base in, quote out.
        SwapDirection::Sell => {
            let in_human = amount_in as f64 / 10f64.powi(fin.base_decimals as i32);
            (fin.quote_decimals, in_human * price)
        }
    };
    let raw = out_human * 10f64.powi(out_dec as i32);
    if raw.is_finite() && raw >= 1.0 { Some(raw as u64) } else { None }
}

/// Deserialize an Anchor account, skipping the 8-byte discriminator.
/// Tolerates trailing bytes.
fn deser_anchor<T: BorshDeserialize>(data: &[u8]) -> Result<T, GenericError> {
    if data.len() < 8 {
        return Err("account data too short".into());
    }
    let mut slice = &data[8..];
    T::deserialize(&mut slice).map_err(|e| e.into())
}
