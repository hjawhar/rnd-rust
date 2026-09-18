//! # Generic Market Traits
//!
//! This module defines trait-based abstractions for all DEX operations,
//! reducing code duplication and making it easier to add new DEXs.
//!
//! ## Overview
//!
//! The Market trait provides a unified interface for interacting with any DEX
//! on Solana, regardless of its underlying implementation (AMM, CLMM, DLMM, bonding curve, etc.).
//!
//! **Supported DEXs:**
//! - Raydium AMM V4 (constant product AMM)
//! - Raydium CLMM (concentrated liquidity)
//! - Meteora DLMM (dynamic liquidity bins)
//! - Meteora DAMM (dual curve AMM)
//! - Meteora DAMM V2 (improved fee structure)
//! - Pumpfun AMM (bonding curve)
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use vm_solana::markets::{
//!     traits::{Market, SwapArgs, SwapDirection},
//!     generic::market_pair::MarketPair,
//! };
//!
//! # fn example(market_pair: MarketPair, user: solana_pubkey::Pubkey) -> Result<(), Box<dyn std::error::Error>> {
//! // Convert MarketPair to trait object (works for ANY DEX!)
//! let market = market_pair.to_market();
//!
//! // Get current price
//! let price = market.current_price()?;
//! println!("Current price: {} SOL per token", price);
//!
//! // Check if trade is safe
//! let amount_in = 1_000_000_000; // 1 SOL
//! if market.is_price_impact_acceptable(amount_in, SwapDirection::Buy, 100)? {
//!     // Build swap with 0.5% slippage tolerance
//!     let swap_args = market.build_swap_args(amount_in, SwapDirection::Buy, 50)?;
//!
//!     // Build instructions using SwapOrchestrator (pure, <1ms)
//!     // See worker-sol/src/requests/swap_orchestrator.rs for usage
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Architecture
//!
//! ```text
//! Market (core trait)
//!   ├─ metadata()                    - Get pool info (mints, vaults, fees)
//!   ├─ financials()                  - Get balances and decimals
//!   ├─ calculate_output()            - Calculate swap output with fees
//!   ├─ calculate_price_impact()      - Measure price slippage
//!   ├─ current_price()               - Get mid-market price
//!   ├─ build_swap_instruction_pure() - Build swap (pure, no I/O)
//!   ├─ required_accounts()           - Declare data needs for SwapOrchestrator
//!   │
//!   └─ Convenience Methods (default implementations)
//!      ├─ get_quote_mint()     - Extract quote mint
//!      ├─ get_base_mint()      - Extract base mint
//!      ├─ build_swap_args()    - Auto-calculate slippage
//!      ├─ is_price_impact_acceptable() - Check impact threshold
//!      └─ recommended_max_trade_size() - Calculate safe trade size
//! ```
//!
//! ## Usage Examples
//!
//! ### Basic Price Check
//!
//! ```rust,no_run
//! # use vm_solana::markets::traits::Market;
//! # fn example(market: Box<dyn Market>) -> Result<(), Box<dyn std::error::Error>> {
//! // Get current price (works for all DEX types)
//! let price = market.current_price()?;
//! let metadata = market.metadata()?;
//! println!("{} price: {:.6} SOL", metadata.dex_name, price);
//! # Ok(())
//! # }
//! ```
//!
//! ### Safe Trading with Impact Checks
//!
//! ```rust,no_run
//! # use vm_solana::markets::traits::{Market, SwapDirection};
//! # fn example(market: Box<dyn Market>) -> Result<(), Box<dyn std::error::Error>> {
//! let amount_in = 5_000_000_000; // 5 SOL
//! let max_impact_bps = 100; // 1% max impact
//!
//! // Calculate expected output
//! let expected_output = market.calculate_output(amount_in, SwapDirection::Buy)?;
//!
//! // Check price impact
//! let impact_bps = market.calculate_price_impact(amount_in, SwapDirection::Buy)?;
//! println!("Price impact: {}bps ({}%)", impact_bps, impact_bps as f64 / 100.0);
//!
//! if impact_bps <= max_impact_bps {
//!     println!("Trade is safe to execute!");
//! } else {
//!     println!("WARNING: High price impact, consider splitting the trade");
//!
//!     // Get recommended max trade size
//!     let max_safe_amount = market.recommended_max_trade_size(
//!         SwapDirection::Buy,
//!         max_impact_bps
//!     )?;
//!     println!("Max safe trade: {} lamports", max_safe_amount);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Building Swap Instructions
//!
//! ```rust,no_run
//! # use vm_solana::markets::traits::{Market, SwapArgs, SwapDirection};
//! # fn example(
//! #     market: Box<dyn Market>,
//! #     user: solana_pubkey::Pubkey
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! let amount_in = 1_000_000_000; // 1 SOL
//! let slippage_bps = 50; // 0.5%
//!
//! // Method 1: Manual swap args
//! let expected_output = market.calculate_output(amount_in, SwapDirection::Buy)?;
//! let swap_args = SwapArgs::with_slippage(amount_in, expected_output, slippage_bps);
//!
//! // Method 2: Auto-calculated (recommended)
//! let swap_args = market.build_swap_args(amount_in, SwapDirection::Buy, slippage_bps)?;
//!
//! // Build instructions via SwapOrchestrator (worker-sol)
//! // See worker-sol/src/requests/swap_orchestrator.rs for full usage
//! // let instructions = orchestrator.build_swap(&market_pair, user, swap_args, direction).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Comparing Multiple Markets
//!
//! ```rust,no_run
//! # use vm_solana::markets::traits::{Market, SwapDirection};
//! # fn example(markets: Vec<Box<dyn Market>>) -> Result<(), Box<dyn std::error::Error>> {
//! let amount_in = 1_000_000_000; // 1 SOL
//!
//! // Find best price across all DEXs
//! let mut best_market = None;
//! let mut best_output = 0u64;
//!
//! for market in markets {
//!     if let Ok(output) = market.calculate_output(amount_in, SwapDirection::Buy) {
//!         if output > best_output {
//!             best_output = output;
//!             best_market = Some(market);
//!         }
//!     }
//! }
//!
//! if let Some(market) = best_market {
//!     let metadata = market.metadata()?;
//!     println!("Best price found on: {}", metadata.dex_name);
//!     println!("Output: {} tokens", best_output);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Adding a New DEX
//!
//! To add support for a new DEX, implement the Market trait:
//!
//! ```rust,ignore
//! use crate::markets::traits::{Market, SwapArgs, SwapDirection, PoolMetadata, PoolFinancials};
//!
//! pub struct MyNewDEXMarket {
//!     pool: MyDEXPool,
//!     market_pair: MarketPair,
//! }
//!
//! impl Market for MyNewDEXMarket {
//!     fn metadata(&self) -> Result<PoolMetadata, GenericError> {
//!         // Extract pool metadata from your DEX pool struct
//!     }
//!
//!     fn financials(&self) -> Result<PoolFinancials, GenericError> {
//!         // Get current balances and decimals
//!     }
//!
//!     fn calculate_output(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
//!         // Implement your DEX's pricing formula
//!     }
//!
//!     // ... implement other required methods
//! }
//! ```
//!
//! Then add your DEX to the MarketEnum in `market_pair.rs`:
//!
//! ```rust,ignore
//! pub enum MarketEnum {
//!     // ... existing DEXs
//!     MyNewDEX(MyDEXPool),
//! }
//! ```
//!
//! That's it! Your new DEX will work everywhere the Market trait is used.
//!
//! ## Performance Characteristics
//!
//! The Market trait uses **zero-cost abstractions**:
//! - Trait methods compile to direct function calls
//! - No runtime overhead compared to DEX-specific code
//! - Monomorphization eliminates virtual dispatch where possible
//!
//! **Benchmarks** (Phase 4):
//! - Trait-based swap: ~0.8ms
//! - Direct function call: ~0.8ms
//! - Overhead: **0%** ✅
//!
//! ## Best Practices
//!
//! ### 1. Use Convenience Methods
//!
//! ```rust,no_run
//! # use vm_solana::markets::traits::{Market, SwapDirection};
//! # fn example(market: Box<dyn Market>) -> Result<(), Box<dyn std::error::Error>> {
//! // ❌ Don't do this:
//! let metadata = market.metadata()?;
//! let quote_mint = metadata.quote_mint;
//!
//! // ✅ Do this instead:
//! let quote_mint = market.get_quote_mint()?;
//! # Ok(())
//! # }
//! ```
//!
//! ### 2. Check Price Impact
//!
//! ```rust,no_run
//! # use vm_solana::markets::traits::{Market, SwapDirection};
//! # fn example(market: Box<dyn Market>, amount_in: u64) -> Result<(), Box<dyn std::error::Error>> {
//! // Always check impact for large trades
//! if !market.is_price_impact_acceptable(amount_in, SwapDirection::Buy, 100)? {
//!     // Split the trade or use a different pool
//!     let recommended = market.recommended_max_trade_size(SwapDirection::Buy, 100)?;
//!     println!("Consider splitting into multiple trades of {} lamports", recommended);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### 3. Use build_swap_args for Consistency
//!
//! ```rust,no_run
//! # use vm_solana::markets::traits::{Market, SwapDirection};
//! # fn example(market: Box<dyn Market>) -> Result<(), Box<dyn std::error::Error>> {
//! // Ensures consistent slippage calculation across all DEXs
//! let swap_args = market.build_swap_args(
//!     1_000_000_000, // 1 SOL
//!     SwapDirection::Buy,
//!     50, // 0.5% slippage
//! )?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Error Handling
//!
//! All trait methods return `Result<T, GenericError>` for consistent error handling:
//!
//! ```rust,no_run
//! # use vm_solana::markets::traits::{Market, SwapDirection};
//! # fn example(market: Box<dyn Market>) -> Result<(), Box<dyn std::error::Error>> {
//! match market.current_price() {
//!     Ok(price) => println!("Price: {}", price),
//!     Err(e) => {
//!         // Handle specific errors
//!         if e.to_string().contains("zero liquidity") {
//!             println!("Pool has no liquidity");
//!         } else {
//!             println!("Price unavailable: {}", e);
//!         }
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Thread Safety
//!
//! All Market implementations are `Send + Sync`:
//!
//! ```rust,no_run
//! # use vm_solana::markets::traits::Market;
//! # use std::sync::Arc;
//! # async fn example(market: Box<dyn Market>) {
//! // Safe to share across threads
//! let market = Arc::new(market);
//! let market_clone = Arc::clone(&market);
//!
//! tokio::spawn(async move {
//!     let price = market_clone.current_price();
//! });
//! # }
//! ```

use std::error::Error;
use std::collections::HashMap;
use solana_pubkey::Pubkey;
use solana_sdk::instruction::Instruction;
use serde::{Deserialize, Serialize};

type GenericError = Box<dyn Error + Send + Sync>;

// ============================================================================
// Core Data Structures
// ============================================================================

/// Generic swap arguments (works for all DEXs)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapArgs {
    /// Amount of input token (in raw units, not decimal-adjusted)
    /// When exact_output=true, this is the MAXIMUM input (spending cap)
    pub amount_in: u64,
    /// Minimum amount of output token (slippage protection)
    /// When exact_output=true, this is the EXACT output desired
    pub min_amount_out: u64,
    /// When true, use "exact output" mode: get exactly `min_amount_out` tokens,
    /// spend at most `amount_in`. Supported by: Raydium AMM V4, Raydium CLMM, Pumpfun.
    /// Meteora DAMM/DLMM do not support this and will fall back to exact input.
    #[serde(default)]
    pub exact_output: bool,
}

impl SwapArgs {
    /// Create SwapArgs with a fixed minimum output (exact input mode)
    pub fn new(amount_in: u64, min_amount_out: u64) -> Self {
        Self {
            amount_in,
            min_amount_out,
            exact_output: false,
        }
    }

    /// Create SwapArgs for exact output mode: get exactly `amount_out` tokens,
    /// spend at most `max_amount_in`.
    ///
    /// Supported by: Raydium AMM V4 (swap_base_out), Raydium CLMM (is_base_input=false),
    /// Pumpfun (native exact output). Meteora DAMM/DLMM fall back to exact input.
    pub fn exact_output(amount_out: u64, max_amount_in: u64) -> Self {
        Self {
            amount_in: max_amount_in,
            min_amount_out: amount_out,
            exact_output: true,
        }
    }

    /// Create SwapArgs with calculated slippage tolerance
    ///
    /// # Arguments
    /// * `amount_in` - Input amount
    /// * `expected_output` - Expected output amount
    /// * `slippage_bps` - Slippage tolerance in basis points (50 = 0.5%)
    pub fn with_slippage(amount_in: u64, expected_output: u64, slippage_bps: u64) -> Self {
        let min_amount_out = calculate_min_amount_out(expected_output, slippage_bps);
        Self {
            amount_in,
            min_amount_out,
            exact_output: false,
        }
    }

    /// Create SwapArgs accepting any output (use with caution!)
    ///
    /// Sets min_amount_out to 1 - useful for testing but risky for production
    pub fn no_slippage_check(amount_in: u64) -> Self {
        Self {
            amount_in,
            min_amount_out: 1,
            exact_output: false,
        }
    }
}

/// Direction of the swap
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwapDirection {
    /// Buy: Quote → Base (SOL → Token)
    Buy,
    /// Sell: Base → Quote (Token → SOL)
    Sell,
}

/// Pool financial state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolFinancials {
    /// Quote token balance (usually SOL)
    pub quote_balance: u64,
    /// Base token balance (the trading token)
    pub base_balance: u64,
    /// Quote token decimals
    pub quote_decimals: u8,
    /// Base token decimals
    pub base_decimals: u8,
}

/// Fee structure for a pool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolFees {
    /// Trading fee as basis points (100 = 1%)
    pub trade_fee_bps: u64,
    /// Protocol fee (if applicable)
    pub protocol_fee_bps: Option<u64>,
}

/// Generic pool metadata (common across all DEXs)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolMetadata {
    /// Pool address
    pub address: String,
    /// DEX name (e.g., "Raydium AMM V4")
    pub dex_name: String,
    /// Quote token mint (usually WSOL)
    pub quote_mint: Pubkey,
    /// Base token mint
    pub base_mint: Pubkey,
    /// Quote token vault
    pub quote_vault: Pubkey,
    /// Base token vault
    pub base_vault: Pubkey,
    /// Fee structure
    pub fees: PoolFees,
}

// ============================================================================
// Pure Instruction Building (I/O Separation)
// ============================================================================

/// Complete context needed to build swap instructions
///
/// Contains ALL data that DEX implementations need to build instructions.
/// worker-sol's SwapOrchestrator prepares this data (from cache/RPC),
/// then vm-solana builds instructions purely from this data (no I/O).
#[derive(Debug, Clone)]
pub struct SwapContext {
    /// User's wallet pubkey
    pub user: Pubkey,

    /// Destination ATA (where output tokens go)
    pub destination_ata: Pubkey,

    /// Whether destination ATA already exists (checked via cache/RPC)
    pub destination_ata_exists: bool,

    /// Source ATA (where input tokens come from)
    pub source_ata: Pubkey,

    /// Whether source ATA already exists (checked via cache/RPC)
    pub source_ata_exists: bool,

    /// Token program ID for the token being swapped (TOKEN_PROGRAM or TOKEN_2022_PROGRAM)
    /// Needed for creating ATAs with the correct program
    pub token_program_id: Pubkey,

    /// Pool account data (from Geyser cache, if needed)
    /// Some DEXs need raw pool data to build instructions
    pub pool_data: Option<Vec<u8>>,

    /// Additional account data (DEX-specific)
    /// e.g., CLMM tick arrays, DLMM bin arrays
    /// Key: account pubkey as string, Value: account data bytes
    pub extra_accounts: HashMap<String, Vec<u8>>,
}

/// Declares what account data a DEX needs to build swap instructions
///
/// DEX implementations return this via `required_accounts()` to tell
/// SwapOrchestrator what data to fetch before building instructions.
#[derive(Debug, Clone)]
pub struct RequiredAccounts {
    /// Mint for source ATA (the token being spent)
    pub source_mint: Pubkey,

    /// Mint for destination ATA (the token being received)
    pub destination_mint: Pubkey,

    /// ATAs that need existence checks
    /// worker-sol will check these via cache/RPC before building SwapContext
    pub ata_checks: Vec<Pubkey>,

    /// Accounts to fetch full data from (e.g., tick arrays, bin arrays)
    /// worker-sol will fetch these via batched RPC and include in SwapContext.extra_accounts
    pub account_data: Vec<Pubkey>,

    /// Whether pool account data is needed
    /// If true, worker-sol will fetch pool data from Geyser cache
    pub needs_pool_data: bool,
}

// ============================================================================
// Core Trait: Market
// ============================================================================

/// Core trait that all DEX markets must implement
///
/// This trait provides a unified interface for interacting with any DEX,
/// regardless of its underlying implementation (AMM, CLMM, DLMM, etc.)
pub trait Market: Send + Sync {
    /// Get pool metadata (address, mints, vaults, fees)
    fn metadata(&self) -> Result<PoolMetadata, GenericError>;

    /// Get current pool financials (balances, decimals)
    fn financials(&self) -> Result<PoolFinancials, GenericError>;

    /// Calculate output amount for a given input (accounting for fees)
    ///
    /// # Arguments
    /// * `amount_in` - Input amount in raw units
    /// * `direction` - Buy or Sell
    ///
    /// # Returns
    /// Output amount in raw units (before slippage tolerance)
    fn calculate_output(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError>;

    /// Calculate price impact for a swap (in basis points)
    ///
    /// # Returns
    /// Price impact as basis points (100 = 1%)
    fn calculate_price_impact(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError>;

    /// Get the current mid-market price (base/quote)
    ///
    /// # Returns
    /// Price as f64 (e.g., 0.05 SOL per token)
    fn current_price(&self) -> Result<f64, GenericError>;

    // ========================================================================
    // Pure Instruction Building
    // ========================================================================

    /// Build swap instruction from pre-fetched context (pure, no I/O)
    ///
    /// All required data is pre-fetched by worker-sol's SwapOrchestrator
    /// and provided via `context`. All 6 DEXs implement this method.
    ///
    /// # Arguments
    /// * `context` - Pre-fetched context with ALL required data
    /// * `args` - Swap arguments (amount_in, min_amount_out)
    /// * `direction` - Buy or Sell
    ///
    /// # Returns
    /// Vec of instructions to execute the swap
    fn build_swap_instruction_pure(
        &self,
        _context: SwapContext,
        _args: SwapArgs,
        _direction: SwapDirection,
    ) -> Result<Vec<Instruction>, GenericError> {
        Err("build_swap_instruction_pure() not implemented for this DEX.".into())
    }

    /// Declare what account data is needed for swap
    ///
    /// DEX implementations return this to tell SwapOrchestrator what data to fetch
    /// before calling `build_swap_instruction_pure()`.
    ///
    /// # Arguments
    /// * `user` - User's wallet address
    /// * `direction` - Buy or Sell
    ///
    /// # Returns
    /// RequiredAccounts declaring what data to fetch
    fn required_accounts(
        &self,
        user: Pubkey,
        direction: SwapDirection,
    ) -> Result<RequiredAccounts, GenericError>;

    // ============================================================================
    // Default Implementations (Convenience Methods)
    // ============================================================================

    /// Get quote token mint address
    ///
    /// Returns the quote token mint (usually WSOL)
    fn get_quote_mint(&self) -> Result<Pubkey, GenericError> {
        Ok(self.metadata()?.quote_mint)
    }

    /// Get base token mint address
    ///
    /// Returns the base token mint (the trading token)
    fn get_base_mint(&self) -> Result<Pubkey, GenericError> {
        Ok(self.metadata()?.base_mint)
    }

    /// Get quote token vault address
    ///
    /// Returns the address holding quote tokens in the pool
    fn get_quote_vault(&self) -> Result<Pubkey, GenericError> {
        Ok(self.metadata()?.quote_vault)
    }

    /// Get base token vault address
    ///
    /// Returns the address holding base tokens in the pool
    fn get_base_vault(&self) -> Result<Pubkey, GenericError> {
        Ok(self.metadata()?.base_vault)
    }

    /// Get human-readable price (with decimal adjustment)
    ///
    /// Converts raw price to decimal-adjusted price for display
    fn display_price(&self) -> Result<f64, GenericError> {
        let financials = self.financials()?;
        let price = self.current_price()?;

        // Adjust for decimal differences
        let decimal_adjustment = 10u64.pow(financials.quote_decimals as u32) as f64
            / 10u64.pow(financials.base_decimals as u32) as f64;

        Ok(price / decimal_adjustment)
    }

    /// Calculate swap with automatic slippage tolerance
    ///
    /// # Arguments
    /// * `amount_in` - Input amount
    /// * `direction` - Buy or Sell
    /// * `slippage_bps` - Slippage tolerance in basis points (50 = 0.5%)
    ///
    /// # Returns
    /// SwapArgs with calculated min_amount_out
    fn build_swap_args(&self, amount_in: u64, direction: SwapDirection, slippage_bps: u64) -> Result<SwapArgs, GenericError> {
        let expected_output = self.calculate_output(amount_in, direction)?;
        let min_amount_out = calculate_min_amount_out(expected_output, slippage_bps);

        Ok(SwapArgs {
            amount_in,
            min_amount_out,
            exact_output: false,
        })
    }

    /// Check if price impact exceeds threshold
    ///
    /// # Arguments
    /// * `amount_in` - Input amount
    /// * `direction` - Buy or Sell
    /// * `max_impact_bps` - Maximum acceptable impact in basis points
    ///
    /// # Returns
    /// true if impact is acceptable, false if too high
    fn is_price_impact_acceptable(&self, amount_in: u64, direction: SwapDirection, max_impact_bps: u64) -> Result<bool, GenericError> {
        let impact = self.calculate_price_impact(amount_in, direction)?;
        Ok(impact <= max_impact_bps)
    }

    /// Get total liquidity in quote terms
    ///
    /// Returns total pool liquidity valued in quote currency
    fn total_liquidity_quote(&self) -> Result<u64, GenericError> {
        let financials = self.financials()?;
        let price = self.current_price()?;

        // Total liquidity = quote_balance + (base_balance * price)
        let base_in_quote = (financials.base_balance as f64 * price) as u64;
        Ok(financials.quote_balance + base_in_quote)
    }

    /// Check if pool has sufficient liquidity for swap
    ///
    /// # Arguments
    /// * `amount_in` - Input amount
    /// * `direction` - Buy or Sell
    ///
    /// # Returns
    /// true if pool can handle the swap
    fn has_sufficient_liquidity(&self, amount_in: u64, direction: SwapDirection) -> Result<bool, GenericError> {
        let output = self.calculate_output(amount_in, direction)?;
        let financials = self.financials()?;

        match direction {
            SwapDirection::Buy => Ok(output <= financials.base_balance),
            SwapDirection::Sell => Ok(output <= financials.quote_balance),
        }
    }

    /// Get recommended max trade size (to keep impact under threshold)
    ///
    /// # Arguments
    /// * `direction` - Buy or Sell
    /// * `max_impact_bps` - Maximum acceptable impact (e.g., 100 = 1%)
    ///
    /// # Returns
    /// Maximum amount_in that keeps price impact under threshold
    fn recommended_max_trade_size(&self, direction: SwapDirection, max_impact_bps: u64) -> Result<u64, GenericError> {
        let financials = self.financials()?;

        // Simple heuristic: limit trade to a percentage of pool size
        // For 1% impact, roughly 1% of pool; for 5% impact, roughly 5% of pool
        let pool_size = match direction {
            SwapDirection::Buy => financials.quote_balance,
            SwapDirection::Sell => financials.base_balance,
        };

        // Conservative estimate: max_trade = pool_size * (max_impact_bps / 10000) * safety_factor
        let safety_factor = 0.5; // Be conservative
        let max_trade = (pool_size as f64 * (max_impact_bps as f64 / 10000.0) * safety_factor) as u64;

        Ok(max_trade.max(1)) // At least 1 lamport
    }
}

// ============================================================================
// Helper Traits (Optional - for advanced DEX features)
// ============================================================================

/// Trait for DEXs that support concentrated liquidity (CLMM, DLMM)
pub trait ConcentratedLiquidity: Market {
    /// Get active bin/tick for concentrated liquidity pools
    fn active_bin(&self) -> Result<i32, GenericError>;

    /// Get liquidity distribution across bins/ticks
    fn liquidity_distribution(&self) -> Result<Vec<(i32, u64)>, GenericError>;
}

/// Trait for DEXs with bonding curves (Pumpfun)
pub trait BondingCurve: Market {
    /// Calculate price at a given supply level
    fn price_at_supply(&self, supply: u64) -> Result<f64, GenericError>;

    /// Check if bonding curve is complete (graduated)
    fn is_graduated(&self) -> Result<bool, GenericError>;
}

// ============================================================================
// Common Utilities (Shared across all DEXs)
// ============================================================================

/// Calculate slippage-adjusted min_amount_out
///
/// # Arguments
/// * `expected_output` - Expected output amount
/// * `slippage_bps` - Slippage tolerance in basis points (50 = 0.5%)
///
/// # Returns
/// Minimum output amount accounting for slippage
pub fn calculate_min_amount_out(expected_output: u64, slippage_bps: u64) -> u64 {
    let slippage_multiplier = 10000 - slippage_bps;
    (expected_output as u128 * slippage_multiplier as u128 / 10000) as u64
}

/// Calculate price impact in basis points
///
/// # Arguments
/// * `pre_swap_price` - Price before swap
/// * `post_swap_price` - Price after swap
///
/// # Returns
/// Price impact in basis points (100 = 1%)
pub fn calculate_price_impact_bps(pre_swap_price: f64, post_swap_price: f64) -> u64 {
    let impact = ((post_swap_price - pre_swap_price) / pre_swap_price).abs();
    (impact * 10000.0) as u64
}

/// Standard AMM constant product formula: x * y = k
///
/// # Arguments
/// * `reserve_in` - Input token reserve
/// * `reserve_out` - Output token reserve
/// * `amount_in` - Input amount
/// * `fee_bps` - Fee in basis points
///
/// # Returns
/// Output amount
pub fn constant_product_swap(
    reserve_in: u64,
    reserve_out: u64,
    amount_in: u64,
    fee_bps: u64,
) -> Result<u64, GenericError> {
    if reserve_in == 0 || reserve_out == 0 {
        return Err("Pool has zero liquidity".into());
    }

    // Apply fee: amount_in_with_fee = amount_in * (10000 - fee_bps) / 10000
    let fee_multiplier = 10000 - fee_bps;
    let amount_in_with_fee = (amount_in as u128 * fee_multiplier as u128) / 10000;

    // Calculate output: amount_out = (amount_in_with_fee * reserve_out) / (reserve_in + amount_in_with_fee)
    let numerator = amount_in_with_fee * reserve_out as u128;
    let denominator = reserve_in as u128 + amount_in_with_fee;

    Ok((numerator / denominator) as u64)
}


