//! Centralized application state for worker-sol.
//!
//! All in-memory caches (DashMap) and cache operations live here.
//! Redis is kept only for cross-service state (heartbeats, daily volume, prices).

use std::collections::HashMap;
use std::error::Error;
use std::sync::{Arc, OnceLock};

use bigdecimal::Pow;
use dashmap::DashMap;
use vm_data::db::Database;
use vm_data::models::task::ProcessTask;
use vm_data::models::wallet::StoredWallet;
use vm_data::models::wallet_relation::{BalanceType, WalletRelation};
use vm_data::utils::helpers::{big_int_to_f64, get_current_time_ms};
use vm_solana::markets::generic::market_pair::{GenericPoolInfo, MarketEnum, MarketPair};
use vm_solana::markets::generic::pools::{
    get_market_account_raw, get_pool_financials, get_raydium_clmm_configs,
};
use vm_solana::markets::raydium_amm_v4::models::raydium_amm_config::AmmConfig;
use vm_solana::models::token_info::TokenInfo;
use vm_solana::rpc::batched_rpc::get_multiple_accounts_batched;
use vm_solana::rpc::fetch_token_info::fetch_token_info as rpc_fetch_token_info;
use solana_native_token::LAMPORTS_PER_SOL;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use tokio::sync::mpsc;

type GenericError = Box<dyn Error + Send + Sync>;

// ============================================================================
// Global State Access
// ============================================================================

static GLOBAL_STATE: OnceLock<Arc<AppState>> = OnceLock::new();

/// Initialize the global application state. Must be called exactly once at startup.
pub fn init_global_state(state: Arc<AppState>) {
    GLOBAL_STATE
        .set(state)
        .unwrap_or_else(|_| panic!("AppState already initialized"));
}

/// Get a reference to the global application state.
pub fn get_state() -> &'static Arc<AppState> {
    GLOBAL_STATE.get().expect("AppState not initialized — call init_global_state first")
}

// ============================================================================
// State Types
// ============================================================================

/// Cached pool account state (from Geyser or RPC fallback)
#[derive(Debug, Clone)]
pub struct CachedPoolState {
    pub data: Vec<u8>,
    pub slot: u64,
    pub cached_at_ms: u64,
}

/// Cached vault balance (from Geyser or RPC fallback)
#[derive(Debug, Clone)]
pub struct CachedVaultBalance {
    pub balance: u64,
    pub slot: u64,
}

/// Centralized application state — single source of truth for worker-sol.
///
/// In-memory DashMaps replace Redis for all hot-path data.
/// Data is populated on startup via `self_initialize()` and kept current by Geyser events.
pub struct AppState {
    // --- Infrastructure ---
    pub rpc: Arc<RpcClient>,
    pub nats: async_nats::Client,
    pub jetstream: async_nats::jetstream::Context,
    pub db: Database,

    // --- Identity ---
    pub worker_id: String,

    // --- Geyser subscription control ---
    pub geyser_address_tx: mpsc::Sender<Vec<String>>,

    // --- In-memory caches (DashMap = lock-free concurrent HashMap) ---
    pub balances: DashMap<String, f64>,
    pub pool_states: DashMap<String, CachedPoolState>,
    pub vault_balances: DashMap<String, CachedVaultBalance>,
    pub tracked_addresses: DashMap<String, bool>,
    pub tracked_relations: DashMap<String, Vec<WalletRelation>>,
    pub wallets: DashMap<String, StoredWallet>,
    pub pairs: DashMap<String, MarketPair>,
    pub pools: DashMap<String, Vec<GenericPoolInfo>>,
    pub tokens: DashMap<String, TokenInfo>,
    pub clmm_configs: DashMap<String, AmmConfig>,
    pub ata_cache: DashMap<String, bool>,
    pub tasks: DashMap<i32, ProcessTask>,
    pub task_handles: DashMap<i32, tokio::task::AbortHandle>,
    pub token_programs: DashMap<String, Pubkey>,
}

impl AppState {
    pub fn new(
        rpc: Arc<RpcClient>,
        nats: async_nats::Client,
        jetstream: async_nats::jetstream::Context,
        db: Database,
        worker_id: String,
        geyser_address_tx: mpsc::Sender<Vec<String>>,
    ) -> Self {
        Self {
            rpc,
            nats,
            jetstream,
            db,
            worker_id,
            geyser_address_tx,
            balances: DashMap::new(),
            pool_states: DashMap::new(),
            vault_balances: DashMap::new(),
            tracked_addresses: DashMap::new(),
            tracked_relations: DashMap::new(),
            wallets: DashMap::new(),
            pairs: DashMap::new(),
            pools: DashMap::new(),
            tokens: DashMap::new(),
            clmm_configs: DashMap::new(),
            ata_cache: DashMap::new(),
            tasks: DashMap::new(),
            task_handles: DashMap::new(),
            token_programs: DashMap::new(),
        }
    }

    // ========================================================================
    // Balance
    // ========================================================================

    pub fn get_balance(&self, address: &str) -> Option<f64> {
        self.balances.get(address).map(|v| *v)
    }

    pub fn set_balance(&self, address: String, balance: f64) {
        self.balances.insert(address, balance);
    }

    pub fn get_token_balance(&self, mint: &str, owner: &str) -> Option<f64> {
        let token_program = self.get_token_program(mint).unwrap_or(spl_token::ID);
        let ata = get_associated_token_address_with_program_id(
            &Pubkey::from_str_const(owner),
            &Pubkey::from_str_const(mint),
            &token_program,
        );
        self.balances.get(&ata.to_string()).map(|v| *v)
    }

    // ========================================================================
    // Vault
    // ========================================================================

    pub fn get_vault_balance(&self, vault_address: &str) -> Option<u64> {
        self.vault_balances.get(vault_address).map(|v| v.balance)
    }

    pub fn set_vault_balance(&self, address: String, balance: u64, slot: u64) {
        self.vault_balances
            .insert(address, CachedVaultBalance { balance, slot });
    }

    // ========================================================================
    // Pool State
    // ========================================================================

    pub fn get_pool_state(&self, address: &str) -> Option<CachedPoolState> {
        let entry = self.pool_states.get(address)?;
        let state = entry.clone();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        if now_ms - state.cached_at_ms < 60_000 {
            Some(state)
        } else {
            None
        }
    }

    pub fn set_pool_state(&self, address: String, data: Vec<u8>, slot: u64) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.pool_states.insert(
            address,
            CachedPoolState {
                data,
                slot,
                cached_at_ms: now_ms,
            },
        );
    }

    /// Get pool account data — cache first, RPC fallback.
    pub async fn fetch_pool_account_data(&self, address: &str) -> Result<Vec<u8>, GenericError> {
        if let Some(cached) = self.get_pool_state(address) {
            return Ok(cached.data);
        }
        let pubkey = Pubkey::from_str_const(address);
        let account = self.rpc.get_account_data(&pubkey).await?;
        let slot = self.rpc.get_slot().await.unwrap_or(0);
        self.set_pool_state(address.to_string(), account.clone(), slot);
        Ok(account)
    }

    /// Batch get pool account data — cache first, batched RPC for misses.
    pub async fn fetch_pool_accounts_batch(
        &self,
        addresses: &[String],
    ) -> Result<Vec<Option<Vec<u8>>>, GenericError> {
        const MAX_PER_BATCH: usize = 50;
        let mut results: Vec<Option<Vec<u8>>> = vec![None; addresses.len()];
        let mut uncached_indices: Vec<usize> = Vec::new();
        let mut uncached_pubkeys: Vec<Pubkey> = Vec::new();

        for (i, address) in addresses.iter().enumerate() {
            if let Some(cached) = self.get_pool_state(address) {
                results[i] = Some(cached.data);
            } else {
                uncached_indices.push(i);
                uncached_pubkeys.push(Pubkey::from_str_const(address));
            }
        }

        if !uncached_pubkeys.is_empty() {
            let slot = self.rpc.get_slot().await.unwrap_or(0);
            for (chunk_idx, chunk) in uncached_pubkeys.chunks(MAX_PER_BATCH).enumerate() {
                let chunk_start = chunk_idx * MAX_PER_BATCH;
                match self.rpc.get_multiple_accounts(chunk).await {
                    Ok(accounts) => {
                        for (idx, account_opt) in accounts.iter().enumerate() {
                            if let Some(account) = account_opt {
                                let original = uncached_indices[chunk_start + idx];
                                let addr = &addresses[original];
                                self.set_pool_state(addr.clone(), account.data.clone(), slot);
                                results[original] = Some(account.data.clone());
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            "[POOL_CACHE] Batch RPC failed for chunk {}: {}",
                            chunk_idx + 1,
                            e
                        );
                    }
                }
            }
        }

        Ok(results)
    }

    // ========================================================================
    // Tracking
    // ========================================================================

    pub fn is_tracked(&self, address: &str) -> bool {
        self.tracked_addresses.contains_key(address)
    }

    pub fn get_relations(&self, owner: &str) -> Vec<WalletRelation> {
        self.tracked_relations
            .get(owner)
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    pub fn get_tracked_addresses_keys(&self) -> Vec<String> {
        self.tracked_addresses
            .iter()
            .map(|e| e.key().clone())
            .collect()
    }

    pub fn get_tracked_address_relations(
        &self,
        user_id: i32,
        project_id: i32,
        address: &str,
    ) -> Vec<WalletRelation> {
        self.get_relations(address)
            .into_iter()
            .filter(|x| x.user_id == user_id && x.project_id == project_id && x.owner == address)
            .collect()
    }

    pub fn get_flat_tracked_addresses(&self) -> Vec<WalletRelation> {
        let mut result = Vec::new();
        for entry in self.tracked_relations.iter() {
            result.extend(entry.value().clone());
        }
        result
    }

    /// Get all tracked addresses including vault addresses from cached pairs.
    pub fn get_all_tracked_addresses(&self) -> Vec<String> {
        let mut addrs: Vec<String> = Vec::new();

        for entry in self.tracked_relations.iter() {
            for rel in entry.value() {
                if !addrs.contains(&rel.address) {
                    addrs.push(rel.address.clone());
                }
            }
        }

        for entry in self.pairs.iter() {
            let market = entry.value().to_market();
            if let Ok(base_vault) = market.get_base_vault() {
                let addr = base_vault.to_string();
                if !addrs.contains(&addr) {
                    addrs.push(addr);
                }
            }
            if let Ok(quote_vault) = market.get_quote_vault() {
                let addr = quote_vault.to_string();
                if !addrs.contains(&addr) {
                    addrs.push(addr);
                }
            }
        }

        addrs
    }

    pub fn track_address(
        &self,
        user_id: i32,
        project_id: i32,
        address: String,
        mint: String,
        token_program: &Pubkey,
    ) {
        let token_pair = get_associated_token_address_with_program_id(
            &Pubkey::from_str_const(&address),
            &Pubkey::from_str_const(&mint),
            token_program,
        )
        .to_string();

        self.tracked_addresses.insert(address.clone(), true);
        self.tracked_addresses.insert(token_pair.clone(), true);

        let existing = self.get_tracked_address_relations(user_id, project_id, &address);
        if existing.is_empty() {
            let mut all = self.get_relations(&address);

            all.push(WalletRelation {
                user_id,
                project_id,
                owner: address.clone(),
                address: address.clone(),
                balance_type: BalanceType::WSOL,
                mint: mint.clone(),
            });

            all.push(WalletRelation {
                user_id,
                project_id,
                owner: address.clone(),
                address: token_pair,
                balance_type: BalanceType::TOKENS,
                mint,
            });

            self.tracked_relations.insert(address, all);
        }
    }

    pub fn track_token(
        &self,
        user_id: i32,
        project_id: i32,
        mint: String,
        pool: String,
        base_vault: String,
        quote_vault: String,
    ) {
        self.tracked_addresses.insert(base_vault.clone(), true);
        self.tracked_addresses.insert(quote_vault.clone(), true);
        self.tracked_addresses.insert(pool.clone(), true);

        let existing = self.get_tracked_address_relations(user_id, project_id, &mint);
        if existing.is_empty() {
            let mut all = self.get_relations(&mint);

            all.push(WalletRelation {
                user_id,
                project_id,
                owner: mint.clone(),
                address: base_vault,
                balance_type: BalanceType::BASE,
                mint: mint.clone(),
            });

            all.push(WalletRelation {
                user_id,
                project_id,
                owner: mint.clone(),
                address: quote_vault,
                balance_type: BalanceType::QUOTE,
                mint: mint.clone(),
            });

            all.push(WalletRelation {
                user_id,
                project_id,
                owner: mint.clone(),
                address: pool,
                balance_type: BalanceType::POOL,
                mint: mint.clone(),
            });

            self.tracked_relations.insert(mint, all);
        }
    }

    pub fn untrack_address(&self, user_id: i32, project_id: i32, address: String) {
        let current_relations =
            self.get_tracked_address_relations(user_id, project_id, &address);
        let all_relations = self.get_relations(&address);

        let remaining: Vec<WalletRelation> = all_relations
            .into_iter()
            .filter(|x| {
                !(x.user_id == user_id && x.project_id == project_id && x.owner == address)
            })
            .collect();

        if remaining.is_empty() {
            self.tracked_relations.remove(&address);
        } else {
            self.tracked_relations
                .insert(address.clone(), remaining.clone());
        }

        for rel in &current_relations {
            if rel.owner != rel.address {
                self.tracked_addresses.remove(&rel.address);
            }
        }
        if remaining.is_empty() {
            self.tracked_addresses.remove(&address);
        }
    }

    /// Remove all tracked addresses and relations for a given project.
    /// Used when releasing ownership of a project (TASK_STOP) to ensure this
    /// worker no longer subscribes to its Geyser updates.
    pub fn untrack_project(&self, project_id: i32) {
        // Collect owners whose relations include this project.
        // We must collect first to avoid holding DashMap refs across mutations.
        let owners: Vec<String> = self
            .tracked_relations
            .iter()
            .filter(|entry| entry.value().iter().any(|r| r.project_id == project_id))
            .map(|entry| entry.key().clone())
            .collect();

        for owner in owners {
            let remaining: Vec<WalletRelation> = self
                .get_relations(&owner)
                .into_iter()
                .filter(|r| r.project_id != project_id)
                .collect();

            if remaining.is_empty() {
                // No other project uses this owner — remove all its addresses
                if let Some((_, rels)) = self.tracked_relations.remove(&owner) {
                    for rel in &rels {
                        self.tracked_addresses.remove(&rel.address);
                    }
                }
                self.tracked_addresses.remove(&owner);
            } else {
                // Other projects still reference this owner — remove only this project's addresses
                let all = self.get_relations(&owner);
                for rel in &all {
                    if rel.project_id == project_id {
                        // Only remove if no remaining relation uses this address
                        let addr_still_used = remaining.iter().any(|r| r.address == rel.address);
                        if !addr_still_used {
                            self.tracked_addresses.remove(&rel.address);
                        }
                    }
                }
                self.tracked_relations.insert(owner, remaining);
            }
        }

        // Also remove any pairs cached for this project's pool.
        // Pairs are keyed by pool address; relations with BalanceType::POOL
        // reference the pool address, so any pool address we just removed
        // should also be evicted from the pairs cache.
        // (This is best-effort — the pair cache is not project-scoped, so we
        // only remove pools that are no longer tracked by any project.)
        self.pairs.retain(|pool_addr, _| self.tracked_addresses.contains_key(pool_addr));
    }

    // ========================================================================
    // Tasks
    // ========================================================================

    pub fn get_task(&self, project_id: i32) -> Option<ProcessTask> {
        self.tasks.get(&project_id).map(|v| v.clone())
    }

    pub fn set_task(&self, project_id: i32, task: ProcessTask) {
        self.tasks.insert(project_id, task);
    }

    pub fn add_task(&self, project_id: i32, task: ProcessTask) -> ProcessTask {
        self.tasks.insert(project_id, task.clone());
        task
    }

    pub fn get_tasks(&self) -> Vec<ProcessTask> {
        self.tasks.iter().map(|e| e.value().clone()).collect()
    }

    pub fn get_vm_active_tasks(&self) -> Vec<ProcessTask> {
        let current_time = get_current_time_ms();
        self.tasks
            .iter()
            .filter(|e| {
                let a = e.value();
                a.project.trading_strategy == "VOLUME_MAKER"
                    && current_time - a.last_active
                        >= (big_int_to_f64(a.project.trading_interval.clone()) * 1000.0) as u128
            })
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn remove_task(&self, project_id: i32) {
        self.tasks.remove(&project_id);
        self.abort_task_loop(project_id);
    }

    /// Abort any running volume maker loop for this project.
    /// Safe to call even if no loop is running (no-op).
    pub fn abort_task_loop(&self, project_id: i32) {
        if let Some((_, handle)) = self.task_handles.remove(&project_id) {
            handle.abort();
        }
    }

    /// Store the abort handle for a spawned volume maker loop.
    pub fn set_task_handle(&self, project_id: i32, handle: tokio::task::AbortHandle) {
        self.task_handles.insert(project_id, handle);
    }


    // ========================================================================
    // Wallets
    // ========================================================================

    pub fn get_wallet(&self, address: &str) -> Option<StoredWallet> {
        self.wallets.get(address).map(|v| v.clone())
    }

    pub fn add_wallet(&self, stored_wallet: StoredWallet) {
        self.wallets
            .insert(stored_wallet.wallet.address.clone(), stored_wallet);
    }

    pub fn remove_wallet(&self, address: &str) {
        self.wallets.remove(address);
    }

    // ========================================================================
    // Market Pairs
    // ========================================================================

    pub fn get_pair(&self, pool: &str) -> Option<MarketPair> {
        self.pairs.get(pool).map(|v| v.clone())
    }

    pub fn set_pair(&self, pool: String, pair: MarketPair) {
        self.pairs.insert(pool, pair);
    }

    pub fn get_pairs_map(&self) -> HashMap<String, MarketPair> {
        self.pairs
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }

    /// Get pair with RPC fallback on cache miss.
    pub async fn fetch_pair(&self, pool: String) -> Result<Option<MarketPair>, GenericError> {
        if let Some(pair) = self.get_pair(&pool) {
            tracing::trace!(pool = %pool, name = %pair.name, "fetch_pair: cache hit");
            return Ok(Some(pair));
        }

        tracing::debug!(pool = %pool, "fetch_pair: cache miss, fetching from RPC");
        if let Some(pair) = get_market_account_raw(self.rpc.clone(), pool.clone()).await {
            tracing::info!(pool = %pool, name = %pair.name, "fetch_pair: fetched from RPC, caching");
            self.set_pair(pool, pair.clone());
            Ok(Some(pair))
        } else {
            tracing::warn!(pool = %pool, "fetch_pair: not found on RPC");
            Ok(None)
        }
    }

    // ========================================================================
    // Pool Info (per token)
    // ========================================================================

    pub fn get_token_pools(&self, token: &str) -> Vec<GenericPoolInfo> {
        self.pools.get(token).map(|v| v.clone()).unwrap_or_default()
    }

    pub fn set_token_pools(&self, token: String, pools: Vec<GenericPoolInfo>) {
        self.pools.insert(token, pools);
    }

    pub fn get_pools_map(&self) -> HashMap<String, Vec<GenericPoolInfo>> {
        self.pools
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }

    pub fn get_pool_type(&self, token: &str, pool_address: &str) -> Option<String> {
        let pools = self.get_token_pools(token);
        pools
            .iter()
            .find(|pool| pool.pair == pool_address)
            .map(|p| p.name.clone())
    }

    // ========================================================================
    // Token Metadata
    // ========================================================================

    pub fn get_token_info(&self, mint: &str) -> Option<TokenInfo> {
        self.tokens.get(mint).map(|v| v.clone())
    }

    pub fn set_token_info(&self, mint: String, info: TokenInfo) {
        self.tokens.insert(mint, info);
    }

    pub fn get_tokens_map(&self) -> HashMap<String, TokenInfo> {
        self.tokens
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }

    /// Get token info — cache first, RPC fallback.
    pub async fn fetch_token_info(&self, mint: &str) -> Result<TokenInfo, GenericError> {
        if let Some(info) = self.get_token_info(mint) {
            return Ok(info);
        }
        let token_info = rpc_fetch_token_info(self.rpc.clone(), mint.to_string()).await?;
        self.set_token_info(mint.to_string(), token_info.clone());
        Ok(token_info)
    }

    /// Batch fetch token info for multiple mints.
    pub async fn fetch_tokens_batch(
        &self,
        mints: &[String],
    ) -> Result<Vec<(String, TokenInfo)>, GenericError> {
        if mints.is_empty() {
            return Ok(vec![]);
        }
        let mut results = Vec::new();
        for mint in mints {
            match self.fetch_token_info(mint).await {
                Ok(token_info) => results.push((mint.clone(), token_info)),
                Err(e) => {
                    tracing::error!("[TOKEN_CACHE] Failed to fetch token {}: {}", mint, e);
                }
            }
        }
        Ok(results)
    }

    /// Warmup token cache on startup.
    pub async fn warmup_token_cache(&self, mints: &[String]) {
        if mints.is_empty() {
            return;
        }
        if let Err(e) = self.fetch_tokens_batch(mints).await {
            tracing::error!("[CACHE_WARMUP] Failed to warmup token cache: {}", e);
        }
    }

    // ========================================================================
    // CLMM Configs
    // ========================================================================

    pub fn get_clmm_config(&self, address: &str) -> Option<AmmConfig> {
        self.clmm_configs.get(address).map(|v| v.clone())
    }

    pub fn set_clmm_config(&self, address: String, config: AmmConfig) {
        self.clmm_configs.insert(address, config);
    }

    pub fn get_clmm_configs_map(&self) -> HashMap<String, AmmConfig> {
        self.clmm_configs
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect()
    }

    /// Fetch all CLMM configs from RPC and cache them.
    pub async fn fetch_clmm_configs(&self) -> Result<(), GenericError> {
        let clmm_configs = get_raydium_clmm_configs(self.rpc.clone())
            .await
            .unwrap_or_default();
        for (pubkey, config) in clmm_configs {
            self.set_clmm_config(pubkey.to_string(), config);
        }
        Ok(())
    }

    // ========================================================================
    // ATA Cache
    // ========================================================================

    pub fn get_ata_cached(&self, pubkey: &str) -> Option<bool> {
        self.ata_cache.get(pubkey).map(|v| *v)
    }

    pub fn set_ata_cached(&self, pubkey: String, exists: bool) {
        self.ata_cache.insert(pubkey, exists);
    }

    pub fn invalidate_ata(&self, pubkey: &str) {
        self.ata_cache.remove(pubkey);
    }

    /// Check ATA existence — cache first, batched RPC for misses.
    pub async fn check_ata_existence(
        &self,
        ata_pubkeys: &[Pubkey],
    ) -> Result<HashMap<Pubkey, bool>, GenericError> {
        if ata_pubkeys.is_empty() {
            return Ok(HashMap::new());
        }

        let mut result = HashMap::new();
        let mut uncached_pubkeys = Vec::new();

        for pubkey in ata_pubkeys {
            let pubkey_str = pubkey.to_string();
            if let Some(exists) = self.get_ata_cached(&pubkey_str) {
                result.insert(*pubkey, exists);
            } else {
                uncached_pubkeys.push(*pubkey);
            }
        }

        if !uncached_pubkeys.is_empty() {
            let accounts =
                get_multiple_accounts_batched(self.rpc.clone(), &uncached_pubkeys).await?;
            for pubkey in &uncached_pubkeys {
                let exists = accounts
                    .get(pubkey)
                    .and_then(|a| a.as_ref())
                    .map(|account| {
                        let has_lamports = account.lamports > 0;
                        let correct_owner = account.owner == spl_token::ID
                            || account.owner.to_string()
                                == "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
                        // Token-2022 accounts have 165 base bytes + optional extension data
                        let correct_size = account.data.len() >= 165;
                        has_lamports && correct_owner && correct_size
                    })
                    .unwrap_or(false);

                result.insert(*pubkey, exists);
                self.set_ata_cached(pubkey.to_string(), exists);
            }
        }

        Ok(result)
    }

    // ========================================================================
    // Token Program Detection (Token vs Token-2022)
    // ========================================================================

    pub fn get_token_program(&self, mint: &str) -> Option<Pubkey> {
        self.token_programs.get(mint).map(|v| *v)
    }

    pub fn set_token_program(&self, mint: String, program: Pubkey) {
        self.token_programs.insert(mint, program);
    }

    /// Detect which token program a mint uses — cache first, RPC fallback.
    /// Token program is immutable for a given mint, so this is safe to cache indefinitely.
    pub async fn fetch_token_program(&self, mint: &str) -> Result<Pubkey, GenericError> {
        if let Some(program) = self.get_token_program(mint) {
            return Ok(program);
        }

        let token_2022_program = Pubkey::from_str_const(
            vm_solana::utils::constants::TOKEN_PROGRAM_2022,
        );

        let mint_pubkey = Pubkey::from_str_const(mint);
        let result = match self.rpc.get_account(&mint_pubkey).await {
            Ok(account) => {
                if account.owner == token_2022_program {
                    token_2022_program
                } else {
                    spl_token::ID
                }
            }
            Err(_) => spl_token::ID,
        };

        self.set_token_program(mint.to_string(), result);
        Ok(result)
    }

    // ========================================================================
    // Token Price (vault-based calculation)
    // ========================================================================

    /// Get token price from a pool.
    /// CLMM: from sqrt_price_x64. Others: from vault balances (in-memory) or RPC fallback.
    pub async fn get_token_price(
        &self,
        pool: String,
        token_decimals: u8,
    ) -> Result<f64, GenericError> {
        let pair = self.fetch_pair(pool).await?.ok_or("Pair not found")?;
        match pair.market {
            MarketEnum::RaydiumCLMM(ref raydium_clmm_pool) => {
                let sqrt_price_x64 = raydium_clmm_pool.sqrt_price_x64;
                let q64 = 2_u128.pow(64);

                let sqrt_price = sqrt_price_x64 as f64 / q64 as f64;
                let raw_price = sqrt_price.pow(2);

                let decimal_adjustment = 10.pow(
                    raydium_clmm_pool.mint_decimals_0 - raydium_clmm_pool.mint_decimals_1,
                );
                let p = raw_price * decimal_adjustment as f64;
                Ok(1.0 / p)
            }
            MarketEnum::MeteoraDLMM(ref lb_pair) => {
                // DLMM uses bin-based pricing — vault balance ratio is meaningless
                // for concentrated liquidity. Use active bin price directly.
                // bin_price = (1 + bin_step/10000)^active_id  (Y-raw per X-raw)
                // Convert to SOL per whole token: * 10^(token_decimals - 9)
                let bin_step = lb_pair.bin_step as f64;
                let active_id = lb_pair.active_id;
                let raw_price = (1.0 + bin_step / 10000.0).powi(active_id);
                let decimal_adjustment = 10f64.powi(token_decimals as i32 - 9);
                Ok(raw_price * decimal_adjustment)
            }
            MarketEnum::MeteoraDAMM(ref pool) => {
                // Meteora DAMM uses shared Vault Program accounts — vault balances are total
                // across ALL pools sharing the vault, not pool-specific. Compute pool's share
                // via LP token ratio: pool_reserve = total_vault * (pool_lp / lp_supply)
                use vm_solana::markets::meteora_damm::models::meteora_dynamic_amm_vault_authority::VaultAuthority;
                use vm_solana::rpc::batched_rpc::extract_token_balance;
                use borsh::BorshDeserialize;

                // Step 1: Fetch vault authority accounts to get real LP mints and total_amount
                let vault_accounts = get_multiple_accounts_batched(
                    self.rpc.clone(),
                    &[pool.a_vault, pool.b_vault],
                ).await?;

                let parse_vault = |vault_key: &Pubkey| -> Result<VaultAuthority, GenericError> {
                    let acct = vault_accounts.get(vault_key)
                        .and_then(|a| a.as_ref())
                        .ok_or_else(|| format!("Vault {} not found on-chain", vault_key))?;
                    if acct.data.len() <= 8 {
                        return Err(format!("Vault {} data too short: {} bytes", vault_key, acct.data.len()).into());
                    }
                    VaultAuthority::deserialize(&mut &acct.data[8..])
                        .map_err(|e| format!("Failed to parse vault {}: {}", vault_key, e).into())
                };

                let vault_a = parse_vault(&pool.a_vault)?;
                let vault_b = parse_vault(&pool.b_vault)?;

                // Step 2: Fetch pool LP balances + LP mint supplies using real mint addresses
                let lp_accounts = get_multiple_accounts_batched(
                    self.rpc.clone(),
                    &[pool.a_vault_lp, pool.b_vault_lp, vault_a.lp_mint, vault_b.lp_mint],
                ).await?;

                let pool_a_lp = lp_accounts.get(&pool.a_vault_lp)
                    .and_then(|a| a.as_ref())
                    .and_then(extract_token_balance)
                    .ok_or("Failed to get pool a_vault_lp balance")?;

                let pool_b_lp = lp_accounts.get(&pool.b_vault_lp)
                    .and_then(|a| a.as_ref())
                    .and_then(extract_token_balance)
                    .ok_or("Failed to get pool b_vault_lp balance")?;

                // SPL mint layout: supply at bytes 36-44 (u64 LE)
                let extract_supply = |pubkey: &Pubkey| -> Option<u64> {
                    let acct = lp_accounts.get(pubkey)?.as_ref()?;
                    if acct.data.len() >= 44 {
                        Some(u64::from_le_bytes(acct.data[36..44].try_into().ok()?))
                    } else {
                        None
                    }
                };

                let a_lp_supply = extract_supply(&vault_a.lp_mint)
                    .ok_or("Failed to get vault A LP mint supply")?;
                let b_lp_supply = extract_supply(&vault_b.lp_mint)
                    .ok_or("Failed to get vault B LP mint supply")?;

                if a_lp_supply == 0 || b_lp_supply == 0 {
                    return Err("Vault LP supply is zero".into());
                }

                // Pool-specific reserves from vault total_amount and LP share
                let pool_a_reserve = vault_a.total_amount as f64 * (pool_a_lp as f64 / a_lp_supply as f64);
                let pool_b_reserve = vault_b.total_amount as f64 * (pool_b_lp as f64 / b_lp_supply as f64);

                if pool_a_reserve == 0.0 {
                    return Err("Pool has zero base reserve".into());
                }

                // Price = quote / base = B(SOL) / A(token) with decimal adjustment
                let q = pool_b_reserve / LAMPORTS_PER_SOL as f64;
                let b = pool_a_reserve / 10f64.powi(token_decimals as i32);
                let price = q / b;
                tracing::info!(
                    "[DAMM_PRICE] price={:.10} SOL/token | pool_a={:.0} pool_b={:.0} | lp_a={}/{} lp_b={}/{}",
                    price, pool_a_reserve, pool_b_reserve, pool_a_lp, a_lp_supply, pool_b_lp, b_lp_supply,
                );
                Ok(price)
            }
            _ => {
                let generic = pair.generic();
                let q = self.get_vault_balance(&generic.quote_vault);
                let b = self.get_vault_balance(&generic.base_vault);
                if let (Some(q), Some(b)) = (q, b) {
                    let q = q as f64 / LAMPORTS_PER_SOL as f64;
                    let b = b as f64 / 10f64.powi(token_decimals as i32);
                    Ok(q / b)
                } else {
                    let financials = get_pool_financials(self.rpc.clone(), pair).await?;
                    Ok(financials.quote_balance / financials.base_balance)
                }
            }
        }
    }

    // ========================================================================
    // Geyser
    // ========================================================================

    /// Notify the Geyser subscription manager to refresh with current tracked addresses.
    pub async fn refresh_geyser_subscription(&self) {
        let addrs = self.get_all_tracked_addresses();
        if let Err(e) = self.geyser_address_tx.send(addrs).await {
            tracing::error!("[STATE] Failed to send address update to Geyser: {}", e);
        }
    }
}
