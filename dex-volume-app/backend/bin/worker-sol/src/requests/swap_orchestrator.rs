//! Swap Orchestrator - Cache-First Instruction Building
//!
//! This module provides a high-level abstraction for building swap instructions
//! with optimized data fetching. It separates I/O concerns (RPC, cache) from
//! pure instruction building logic.
//!
//! **Architecture**:
//! ```text
//! SwapOrchestrator (worker-sol)
//!   │
//!   ├─> Ask Market: "What data do you need?"
//!   ├─> Fetch all data in parallel (cache-first)
//!   └─> Call Market: "Build instructions" (pure function)
//! ```
//!
//! **Performance**:
//! - Cache hits: <5ms total (Redis lookup only)
//! - Cache misses: <150ms (batched RPC calls)
//! - Parallel data fetching (all I/O concurrent)
//!
//! **Usage**:
//! ```rust,no_run
//! let orchestrator = SwapOrchestrator::new(rpc);
//! let instructions = orchestrator.build_swap(
//!     &market_pair,
//!     user_pubkey,
//!     swap_args,
//!     SwapDirection::Buy,
//! ).await?;
//! ```

use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;

use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::instruction::Instruction;
use spl_associated_token_account::get_associated_token_address_with_program_id;

use vm_solana::markets::generic::market_pair::MarketPair;
use vm_solana::markets::traits::{
    Market, SwapArgs, SwapDirection, SwapContext, RequiredAccounts,
};
use vm_solana::rpc::batched_rpc::get_multiple_accounts_batched;


type GenericError = Box<dyn Error + Send + Sync>;


// ============================================================================
// Swap Orchestrator
// ============================================================================

/// High-level swap instruction builder with optimized I/O
///
/// Responsibilities:
/// - Fetch all required data (cache-first strategy)
/// - Batch RPC calls for cache misses
/// - Parallelize independent I/O operations
/// - Prepare SwapContext for pure instruction building
pub struct SwapOrchestrator {
    rpc: Arc<RpcClient>,
}

impl SwapOrchestrator {
    /// Create new orchestrator with RPC client
    pub fn new(rpc: Arc<RpcClient>) -> Self {
        Self { rpc }
    }

    /// Build swap instructions with full optimization
    ///
    /// **Flow**:
    /// 1. Ask market what data it needs (`required_accounts`)
    /// 2. Fetch all data in parallel (cache-first)
    /// 3. Build pure SwapContext
    /// 4. Call market's `build_swap_instruction_pure`
    ///
    /// **Performance**:
    /// - All cache hits: <5ms
    /// - Some cache misses: <150ms (batched RPC)
    /// - Parallel data fetching maximizes throughput
    pub async fn build_swap(
        &self,
        market_pair: &MarketPair,
        user: Pubkey,
        swap_args: SwapArgs,
        direction: SwapDirection,
    ) -> Result<Vec<Instruction>, GenericError> {
        // Step 1: Get market instance (zero-cost)
        let market = market_pair.to_market();

        // Step 2: Ask DEX what data it needs
        let required = market.required_accounts(user, direction)?;

        // Step 3: Fetch all required data (async, parallel)
        let context = self
            .prepare_context(&market, user, direction, required)
            .await?;

        // Step 4: Build instructions (pure, synchronous)
        let instructions = market.build_swap_instruction_pure(
            context.clone(),
            swap_args.clone(),
            direction,
        )?;

        Ok(instructions)
    }

    /// Fetch all required data in parallel (cache-first strategy)
    ///
    /// **Optimizations**:
    /// - Token program detected first (needed for correct ATA derivation)
    /// - Remaining I/O operations run concurrently via `tokio::join!`
    /// - Cache checks happen first (Redis ~1ms)
    /// - RPC calls batched for cache misses (~100ms)
    /// - Pool data from Geyser cache (60s TTL, real-time)
    async fn prepare_context(
        &self,
        market: &Box<dyn Market>,
        user: Pubkey,
        _direction: SwapDirection,
        required: RequiredAccounts,
    ) -> Result<SwapContext, GenericError> {
        // Phase 1: Detect token program (needed for correct ATA address derivation)
        // Token2022 mints produce different ATA addresses than standard Token mints
        let token_program_id = self.detect_token_program(market).await?;

        // Compute ATA addresses using detected token program
        let (destination_ata, source_ata) = self.compute_ata_addresses(user, &required, token_program_id);

        // Phase 2: Parallel fetch all remaining data
        // tokio::join! runs all futures concurrently
        let (dest_ata_result, source_ata_result, pool_data_result, extra_accounts_result) = tokio::join!(
            self.check_ata_cached(destination_ata),
            self.check_ata_cached(source_ata),
            self.fetch_pool_data_if_needed(market, required.needs_pool_data),
            self.fetch_account_data_batch(required.account_data),
        );

        let destination_ata_exists = dest_ata_result?;
        let source_ata_exists = source_ata_result?;
        let pool_data = pool_data_result?;
        let extra_accounts = extra_accounts_result?;

        Ok(SwapContext {
            user,
            destination_ata,
            destination_ata_exists,
            source_ata,
            source_ata_exists,
            token_program_id,
            pool_data,
            extra_accounts,
        })
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Compute ATA addresses for source and destination
    ///
    /// Pure computation (no I/O), uses SPL token program derivation.
    /// Uses the detected token program for the non-SOL token (Token2022 mints
    /// produce different ATA addresses than standard Token mints).
    /// WSOL always uses standard Token program.
    fn compute_ata_addresses(
        &self,
        user: Pubkey,
        required: &RequiredAccounts,
        base_token_program: Pubkey,
    ) -> (Pubkey, Pubkey) {
        use vm_solana::utils::constants::WSOL;

        let wsol = Pubkey::from_str_const(WSOL);

        // WSOL always uses standard Token program; non-SOL token uses detected program
        let dest_program = if required.destination_mint == wsol { spl_token::ID } else { base_token_program };
        let src_program = if required.source_mint == wsol { spl_token::ID } else { base_token_program };

        let destination_ata = get_associated_token_address_with_program_id(&user, &required.destination_mint, &dest_program);
        let source_ata = get_associated_token_address_with_program_id(&user, &required.source_mint, &src_program);

        (destination_ata, source_ata)
    }

    /// Check ATA existence (cache-first, 1h TTL)
    ///
    /// **Performance**:
    /// - Cache hit: ~1ms (Redis GET)
    /// - Cache miss: ~100ms (RPC call + cache write)
    /// - Expected hit rate: 80%+ after warmup
    async fn check_ata_cached(&self, ata: Pubkey) -> Result<bool, GenericError> {
        let state = crate::state::get_state();
        let result = state.check_ata_existence(&[ata]).await?;
        Ok(result.get(&ata).copied().unwrap_or(false))
    }

    /// Fetch pool account data from Geyser cache (60s TTL)
    ///
    /// **Performance**:
    /// - Cache hit: ~1ms (Redis GET, Geyser-fed)
    /// - Cache miss: ~100ms (RPC fallback)
    /// - Expected hit rate: 95%+ for active pools
    async fn fetch_pool_data_if_needed(
        &self,
        market: &Box<dyn Market>,
        needs_data: bool,
    ) -> Result<Option<Vec<u8>>, GenericError> {
        if !needs_data {
            return Ok(None);
        }

        let state = crate::state::get_state();
        let metadata = market.metadata()?;
        let pool_data = state.fetch_pool_account_data(&metadata.address).await?;
        Ok(Some(pool_data))
    }

    /// Batch fetch extra account data (e.g., tick arrays, bin arrays)
    ///
    /// **Performance**:
    /// - Uses batched RPC (50 accounts per call)
    /// - Parallel processing of chunks
    /// - ~100-200ms for 1-100 accounts
    async fn fetch_account_data_batch(
        &self,
        pubkeys: Vec<Pubkey>,
    ) -> Result<HashMap<String, Vec<u8>>, GenericError> {
        if pubkeys.is_empty() {
            return Ok(HashMap::new());
        }

        let accounts = get_multiple_accounts_batched(self.rpc.clone(), &pubkeys).await?;

        let mut result = HashMap::new();
        for (pubkey, account) in accounts {
            if let Some(acc) = account {
                result.insert(pubkey.to_string(), acc.data);
            }
        }

        Ok(result)
    }

    /// Detect which token program the non-SOL token uses (TOKEN_PROGRAM or TOKEN_2022_PROGRAM)
    ///
    /// Delegates to AppState's in-memory token_programs cache (populated on first access via RPC).
    /// Handles flipped pools where base_mint = WSOL by picking whichever mint is NOT WSOL.
    async fn detect_token_program(
        &self,
        market: &Box<dyn Market>,
    ) -> Result<Pubkey, GenericError> {
        use vm_solana::utils::constants::WSOL;

        let wsol = Pubkey::from_str_const(WSOL);
        let base_mint = market.get_base_mint()?;
        let quote_mint = market.get_quote_mint()?;

        // Pick the non-WSOL mint (handles flipped pools where base_mint = WSOL)
        let token_mint = if base_mint == wsol { quote_mint } else { base_mint };

        let state = crate::state::get_state();
        state.fetch_token_program(&token_mint.to_string()).await
    }

}

// ============================================================================
// Convenience Functions
// ============================================================================

/// Build buy swap instructions (convenience wrapper)
///
/// **Usage**:
/// ```rust,no_run
/// let instructions = build_buy_swap(
///     rpc,
///     market_pair,
///     user_pubkey,
///     amount_in_lamports,
///     min_amount_out_tokens,
/// ).await?;
/// ```
pub async fn build_buy_swap(
    rpc: Arc<RpcClient>,
    market_pair: &MarketPair,
    user: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<Vec<Instruction>, GenericError> {
    let orchestrator = SwapOrchestrator::new(rpc);
    let swap_args = SwapArgs::new(amount_in, min_amount_out);

    orchestrator
        .build_swap(market_pair, user, swap_args, SwapDirection::Buy)
        .await
}

/// Build sell swap instructions (convenience wrapper)
///
/// **Usage**:
/// ```rust,no_run
/// let instructions = build_sell_swap(
///     rpc,
///     market_pair,
///     user_pubkey,
///     amount_in_tokens,
///     min_amount_out_lamports,
/// ).await?;
/// ```
pub async fn build_sell_swap(
    rpc: Arc<RpcClient>,
    market_pair: &MarketPair,
    user: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<Vec<Instruction>, GenericError> {
    let orchestrator = SwapOrchestrator::new(rpc);
    let swap_args = SwapArgs::new(amount_in, min_amount_out);

    orchestrator
        .build_swap(market_pair, user, swap_args, SwapDirection::Sell)
        .await
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swap_context_creation() {
        let user = Pubkey::new_unique();
        let destination_ata = Pubkey::new_unique();
        let source_ata = Pubkey::new_unique();

        let context = SwapContext {
            user,
            destination_ata,
            destination_ata_exists: true,
            source_ata,
            source_ata_exists: true,
            token_program_id: spl_token::ID,
            pool_data: None,
            extra_accounts: HashMap::new(),
        };

        assert_eq!(context.user, user);
        assert_eq!(context.destination_ata_exists, true);
        assert!(context.pool_data.is_none());
        assert!(context.extra_accounts.is_empty());
    }

    #[test]
    fn test_required_accounts_creation() {
        let source_mint = Pubkey::new_unique();
        let destination_mint = Pubkey::new_unique();

        let required = RequiredAccounts {
            source_mint,
            destination_mint,
            ata_checks: vec![Pubkey::new_unique()],
            account_data: vec![],
            needs_pool_data: false,
        };

        assert_eq!(required.source_mint, source_mint);
        assert_eq!(required.destination_mint, destination_mint);
        assert_eq!(required.ata_checks.len(), 1);
        assert_eq!(required.account_data.len(), 0);
        assert_eq!(required.needs_pool_data, false);
    }
}
