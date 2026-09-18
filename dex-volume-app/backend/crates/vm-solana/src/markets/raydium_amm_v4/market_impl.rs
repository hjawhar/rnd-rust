//! Phase 3: Market trait implementation for Raydium AMM V4
//!
//! This module implements the generic Market trait for Raydium AMM V4,
//! demonstrating how to use the trait-based abstraction.

use std::error::Error;
use solana_pubkey::Pubkey;
use solana_sdk::instruction::Instruction;

use crate::markets::traits::{
    Market, SwapArgs, SwapDirection, PoolMetadata, PoolFinancials, PoolFees,
    SwapContext, RequiredAccounts,
    constant_product_swap, calculate_price_impact_bps,
};
use crate::markets::raydium_amm_v4::models::raydium_amm_v4::RaydiumAMMV4;
use crate::markets::generic::market_pair::MarketPair;

type GenericError = Box<dyn Error + Send + Sync>;

#[derive(Clone, Debug, borsh::BorshSerialize)]
struct SwapInstructionArgs {
    pub amount_in: u64,
    pub min_amount_out: u64,
}

/// Wrapper for RaydiumAMMV4 that implements the Market trait
pub struct RaydiumAMMV4Market {
    pool: RaydiumAMMV4,
    market_pair: MarketPair,
    /// Current pool balances (cached from last fetch)
    quote_balance: u64,
    base_balance: u64,
}

impl RaydiumAMMV4Market {
    pub fn new(pool: RaydiumAMMV4, market_pair: MarketPair) -> Self {
        Self {
            pool,
            market_pair,
            quote_balance: 0,
            base_balance: 0,
        }
    }

}

impl Market for RaydiumAMMV4Market {
    fn metadata(&self) -> Result<PoolMetadata, GenericError> {
        let fee_bps = (self.pool.trade_fee_numerator as f64
            / self.pool.trade_fee_denominator as f64 * 10000.0) as u64;

        Ok(PoolMetadata {
            address: self.market_pair.pair.clone(),
            dex_name: "Raydium AMM V4".to_string(),
            quote_mint: self.pool.quote_mint,
            base_mint: self.pool.base_mint,
            quote_vault: self.pool.quote_vault,
            base_vault: self.pool.base_vault,
            fees: PoolFees {
                trade_fee_bps: fee_bps,
                protocol_fee_bps: None,
            },
        })
    }

    fn financials(&self) -> Result<PoolFinancials, GenericError> {
        Ok(PoolFinancials {
            quote_balance: self.quote_balance,
            base_balance: self.base_balance,
            quote_decimals: 9, // SOL decimals
            base_decimals: self.pool.base_decimal as u8,
        })
    }

    fn calculate_output(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
        let fee_bps = (self.pool.trade_fee_numerator as f64
            / self.pool.trade_fee_denominator as f64 * 10000.0) as u64;

        match direction {
            SwapDirection::Buy => {
                // Quote → Base (SOL → Token)
                constant_product_swap(
                    self.quote_balance,
                    self.base_balance,
                    amount_in,
                    fee_bps,
                )
            }
            SwapDirection::Sell => {
                // Base → Quote (Token → SOL)
                constant_product_swap(
                    self.base_balance,
                    self.quote_balance,
                    amount_in,
                    fee_bps,
                )
            }
        }
    }

    fn calculate_price_impact(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
        let pre_swap_price = self.current_price()?;

        // Calculate post-swap price
        let output = self.calculate_output(amount_in, direction)?;

        let post_swap_price = match direction {
            SwapDirection::Buy => {
                let new_quote = self.quote_balance + amount_in;
                let new_base = self.base_balance - output;
                new_quote as f64 / new_base as f64
            }
            SwapDirection::Sell => {
                let new_base = self.base_balance + amount_in;
                let new_quote = self.quote_balance - output;
                new_quote as f64 / new_base as f64
            }
        };

        Ok(calculate_price_impact_bps(pre_swap_price, post_swap_price))
    }

    fn current_price(&self) -> Result<f64, GenericError> {
        if self.base_balance == 0 {
            return Err("Pool has zero base balance".into());
        }

        // Price = quote_balance / base_balance
        Ok(self.quote_balance as f64 / self.base_balance as f64)
    }

    // ========================================================================
    // Phase 2: Pure Instruction Building (NEW, RECOMMENDED)
    // ========================================================================

    fn build_swap_instruction_pure(
        &self,
        context: SwapContext,
        args: SwapArgs,
        direction: SwapDirection,
    ) -> Result<Vec<Instruction>, GenericError> {
        use spl_associated_token_account::instruction::create_associated_token_account_idempotent;
        use spl_token::instruction::initialize_account;
        use solana_system_interface::instruction as system_instruction;
        use solana_sdk::instruction::AccountMeta;
        use solana_sdk::program_pack::Pack;
        use spl_token::state::Account as TokenAccount;
        use borsh::to_vec;
        use crate::utils::constants::{
            RAYDIUM_AUTHORITY_V4, RAYDIUM_LIQUIDITY_POOL_V4, TOKEN_PROGRAM, WSOL,
        };

        let mut instructions = Vec::new();

        // Standard rent for token account (165 bytes)
        // This is deterministic and doesn't require RPC
        const TOKEN_ACCOUNT_RENT: u64 = 2_039_280; // Standard rent-exempt amount

        let native_mint = spl_token::native_mint::ID;
        let token_program = Pubkey::from_str_const(TOKEN_PROGRAM);
        let amm_authority = Pubkey::from_str_const(RAYDIUM_AUTHORITY_V4);
        let amm_id = Pubkey::from_str_const(&self.market_pair.pair);

        match direction {
            SwapDirection::Buy => {
                // Buy: Quote (SOL) → Base (Token)
                let base_mint = if Pubkey::from_str_const(WSOL) == self.pool.base_mint {
                    self.pool.quote_mint
                } else {
                    self.pool.base_mint
                };

                // Create temporary WSOL account for input
                let seed = &format!("{}", context.user)[..32];
                let wsol_pubkey = Pubkey::create_with_seed(
                    &context.user,
                    seed,
                    &spl_token::id(),
                )?;

                let total_amount = TOKEN_ACCOUNT_RENT + args.amount_in;

                // 1. Create temporary WSOL account
                instructions.push(system_instruction::create_account_with_seed(
                    &context.user,
                    &wsol_pubkey,
                    &context.user,
                    seed,
                    total_amount,
                    TokenAccount::LEN as u64,
                    &spl_token::id(),
                ));

                // 2. Initialize WSOL account
                instructions.push(initialize_account(
                    &spl_token::id(),
                    &wsol_pubkey,
                    &native_mint,
                    &context.user,
                )?);

                // 3. Create destination ATA if needed (from context!)
                if !context.destination_ata_exists {
                    instructions.push(create_associated_token_account_idempotent(
                        &context.user,
                        &context.user,
                        &base_mint,
                        &context.token_program_id,
                    ));
                }

                // 4. Build swap instruction
                let keys = vec![
                    AccountMeta::new_readonly(token_program, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_authority, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(self.pool.base_vault, false),
                    AccountMeta::new(self.pool.quote_vault, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(wsol_pubkey, false),
                    AccountMeta::new(context.destination_ata, false),
                    AccountMeta::new(context.user, true),
                ];

                // Discriminator 9 = swap_base_in (exact input), 11 = swap_base_out (exact output)
                // swap_base_out: {max_amount_in, amount_out} — same struct layout, different semantics
                let disc = if args.exact_output { 11u8 } else { 9u8 };
                let swap_args = SwapInstructionArgs {
                    amount_in: args.amount_in,
                    min_amount_out: args.min_amount_out,
                };

                let mut data = vec![disc];
                let mut args_bytes = to_vec(&swap_args)?;
                data.append(&mut args_bytes);

                instructions.push(Instruction {
                    program_id: Pubkey::from_str_const(RAYDIUM_LIQUIDITY_POOL_V4),
                    accounts: keys,
                    data,
                });

                // 5. Close temporary WSOL account
                instructions.push(spl_token::instruction::close_account(
                    &spl_token::id(),
                    &wsol_pubkey,
                    &context.user,
                    &context.user,
                    &[],
                )?);
            }

            SwapDirection::Sell => {
                // Sell: Base (Token) → Quote (SOL)
                let quote_mint = self.pool.quote_mint;

                // Create temporary WSOL account for output
                let seed = &format!("{}", context.user)[..32];
                let wsol_pubkey = Pubkey::create_with_seed(
                    &context.user,
                    seed,
                    &spl_token::id(),
                )?;

                // 1. Create temporary WSOL account
                instructions.push(system_instruction::create_account_with_seed(
                    &context.user,
                    &wsol_pubkey,
                    &context.user,
                    seed,
                    TOKEN_ACCOUNT_RENT,
                    TokenAccount::LEN as u64,
                    &spl_token::id(),
                ));

                // 2. Initialize WSOL account
                instructions.push(initialize_account(
                    &spl_token::id(),
                    &wsol_pubkey,
                    &native_mint,
                    &context.user,
                )?);

                // 3. Create destination ATA if needed (WSOL always uses standard Token program)
                if !context.destination_ata_exists {
                    instructions.push(create_associated_token_account_idempotent(
                        &context.user,
                        &context.user,
                        &quote_mint,
                        &spl_token::ID,
                    ));
                }

                // 4. Build swap instruction
                let keys = vec![
                    AccountMeta::new_readonly(token_program, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_authority, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(self.pool.base_vault, false),
                    AccountMeta::new(self.pool.quote_vault, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(amm_id, false),
                    AccountMeta::new(context.source_ata, false),
                    AccountMeta::new(wsol_pubkey, false),
                    AccountMeta::new(context.user, true),
                ];

                let swap_args = SwapInstructionArgs {
                    amount_in: args.amount_in,
                    min_amount_out: args.min_amount_out,
                };

                let mut data = vec![9u8];
                let mut args_bytes = to_vec(&swap_args)?;
                data.append(&mut args_bytes);

                instructions.push(Instruction {
                    program_id: Pubkey::from_str_const(RAYDIUM_LIQUIDITY_POOL_V4),
                    accounts: keys,
                    data,
                });

                // 5. Close temporary WSOL account
                instructions.push(spl_token::instruction::close_account(
                    &spl_token::id(),
                    &wsol_pubkey,
                    &context.user,
                    &context.user,
                    &[],
                )?);
            }
        }

        Ok(instructions)
    }

    fn required_accounts(
        &self,
        _user: Pubkey,
        direction: SwapDirection,
    ) -> Result<RequiredAccounts, GenericError> {
        use crate::utils::constants::WSOL;

        // Handle flipped pools where base_mint = WSOL instead of the standard convention
        let wsol = Pubkey::from_str_const(WSOL);
        let (sol_mint, token_mint) = if self.pool.base_mint == wsol {
            (self.pool.base_mint, self.pool.quote_mint)   // flipped: base=SOL, quote=Token
        } else {
            (self.pool.quote_mint, self.pool.base_mint)   // standard: quote=SOL, base=Token
        };

        let (source_mint, destination_mint) = match direction {
            SwapDirection::Buy => (sol_mint, token_mint),   // SOL → Token
            SwapDirection::Sell => (token_mint, sol_mint),   // Token → SOL
        };

        Ok(RequiredAccounts {
            source_mint,
            destination_mint,
            ata_checks: vec![],      // ATAs checked by orchestrator automatically
            account_data: vec![],     // No tick arrays or extra accounts needed
            needs_pool_data: false,   // Pool data already in struct
        })
    }
}

