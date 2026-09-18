//! Phase 3: Market trait implementation for Meteora DAMM V2
//!
//! This module implements the generic Market trait for Meteora Dynamic AMM V2.
//! V2 uses sqrt_price directly and has improved fee structures.

use std::error::Error;
use solana_pubkey::Pubkey;
use solana_sdk::instruction::Instruction;

use crate::markets::traits::{
    Market, SwapArgs, SwapDirection, PoolMetadata, PoolFinancials, PoolFees,
    SwapContext, RequiredAccounts,
    calculate_price_impact_bps,
};
use crate::markets::meteora_damm::models::meteora_dynamic_amm_v2::MeteoraDAMMV2Pool;
use crate::markets::generic::market_pair::MarketPair;

type GenericError = Box<dyn Error + Send + Sync>;

#[derive(Clone, Debug, borsh::BorshSerialize)]
struct SwapInstructionArgs {
    pub in_amount: u64,
    pub minimum_out_amount: u64,
}

/// Wrapper for MeteoraDAMMV2Pool that implements the Market trait
pub struct MeteoraDAMMV2Market {
    pool: MeteoraDAMMV2Pool,
    market_pair: MarketPair,
    /// Current vault balances (cached from last fetch)
    a_vault_balance: u64,
    b_vault_balance: u64,
}

impl MeteoraDAMMV2Market {
    pub fn new(pool: MeteoraDAMMV2Pool, market_pair: MarketPair) -> Self {
        Self {
            pool,
            market_pair,
            a_vault_balance: 0,
            b_vault_balance: 0,
        }
    }


    /// Convert sqrt_price to regular price
    ///
    /// V2 uses Q128 fixed-point format for sqrt(price)
    /// price = (sqrt_price / 2^64)^2
    fn sqrt_price_to_price(&self) -> f64 {
        let sqrt_price_f64 = self.pool.sqrt_price as f64 / (1u128 << 64) as f64;
        sqrt_price_f64 * sqrt_price_f64
    }

    /// Calculate base fee in basis points
    fn calculate_base_fee_bps(&self) -> u64 {
        // V2 uses cliff_fee_numerator with 10000 denominator
        let numerator = self.pool.pool_fees.base_fee.cliff_fee_numerator;
        numerator / 100 // Convert to basis points (assuming numerator is in parts per million)
    }

    /// Calculate output for V2 swap (simplified)
    fn calculate_v2_output(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
        let price = self.sqrt_price_to_price();
        let fee_bps = self.calculate_base_fee_bps();

        let fee_multiplier = 10000 - fee_bps;
        let amount_in_with_fee = (amount_in as u128 * fee_multiplier as u128) / 10000;

        // Simplified output calculation using current price
        let output = match direction {
            SwapDirection::Buy => {
                // B → A (SOL → Token)
                // output ≈ amount_in / price
                (amount_in_with_fee as f64 / price) as u64
            }
            SwapDirection::Sell => {
                // A → B (Token → SOL)
                // output ≈ amount_in * price
                (amount_in_with_fee as f64 * price) as u64
            }
        };

        Ok(output)
    }
}

impl Market for MeteoraDAMMV2Market {
    fn metadata(&self) -> Result<PoolMetadata, GenericError> {
        let trade_fee_bps = self.calculate_base_fee_bps();
        let protocol_fee_bps = (self.pool.pool_fees.protocol_fee_percent as u64 * trade_fee_bps) / 100;

        Ok(PoolMetadata {
            address: self.market_pair.pair.clone(),
            dex_name: "Meteora DAMM V2".to_string(),
            quote_mint: self.pool.token_b_mint, // B is usually quote (SOL)
            base_mint: self.pool.token_a_mint,  // A is usually base (token)
            quote_vault: self.pool.token_b_vault,
            base_vault: self.pool.token_a_vault,
            fees: PoolFees {
                trade_fee_bps,
                protocol_fee_bps: Some(protocol_fee_bps),
            },
        })
    }

    fn financials(&self) -> Result<PoolFinancials, GenericError> {
        Ok(PoolFinancials {
            quote_balance: self.b_vault_balance,
            base_balance: self.a_vault_balance,
            quote_decimals: 9, // Assuming SOL (B)
            base_decimals: 6,  // Common token decimals (A)
        })
    }

    fn calculate_output(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
        self.calculate_v2_output(amount_in, direction)
    }

    fn calculate_price_impact(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
        let pre_swap_price = self.current_price()?;

        // Calculate post-swap price (simplified)
        let output = self.calculate_output(amount_in, direction)?;

        let post_swap_price = match direction {
            SwapDirection::Buy => {
                let new_b = self.b_vault_balance + amount_in;
                let new_a = self.a_vault_balance.saturating_sub(output);
                if new_a == 0 {
                    return Err("Insufficient liquidity in pool".into());
                }
                new_b as f64 / new_a as f64
            }
            SwapDirection::Sell => {
                let new_a = self.a_vault_balance + amount_in;
                let new_b = self.b_vault_balance.saturating_sub(output);
                if new_a == 0 {
                    return Err("Insufficient liquidity in pool".into());
                }
                new_b as f64 / new_a as f64
            }
        };

        Ok(calculate_price_impact_bps(pre_swap_price, post_swap_price))
    }

    fn current_price(&self) -> Result<f64, GenericError> {
        // Use sqrt_price for accurate V2 pricing
        let price = self.sqrt_price_to_price();
        Ok(price)
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
        use solana_sdk::instruction::AccountMeta;
        use borsh::to_vec;
        use crate::utils::constants::{METEORA_DYNAMIC_AMM_V2, TOKEN_PROGRAM};

        let mut instructions = Vec::new();

        // Check if pool is active (status 1 = active)
        if self.pool.pool_status != 1 {
            return Err(format!("Pool is not active (status: {})", self.pool.pool_status).into());
        }

        // V2 uses simplified vault structure (no extra derivations needed)

        match direction {
            SwapDirection::Buy => {
                // Buy: B (SOL) → A (Token)

                // Create destination ATA if needed (token uses detected program)
                if !context.destination_ata_exists {
                    instructions.push(create_associated_token_account_idempotent(
                        &context.user,
                        &context.user,
                        &self.pool.token_a_mint,
                        &context.token_program_id,
                    ));
                }

                // Build swap instruction (same as V1)
                let swap_args = SwapInstructionArgs {
                    in_amount: args.amount_in,
                    minimum_out_amount: args.min_amount_out,
                };

                // V2 has simplified account structure (no vault_lp accounts)
                let keys: Vec<AccountMeta> = vec![
                    AccountMeta::new(Pubkey::from_str_const(&self.market_pair.pair), false),
                    AccountMeta::new(context.source_ata, false),
                    AccountMeta::new(context.destination_ata, false),
                    AccountMeta::new(self.pool.token_a_vault, false),
                    AccountMeta::new(self.pool.token_b_vault, false),
                    AccountMeta::new_readonly(Pubkey::from_str_const(TOKEN_PROGRAM), false),
                    AccountMeta::new(context.user, true),
                ];

                // Discriminator for V2 swap
                let mut data = vec![248, 198, 158, 145, 225, 117, 135, 200];
                let mut args_bytes = to_vec(&swap_args)?;
                data.append(&mut args_bytes);

                instructions.push(Instruction {
                    program_id: Pubkey::from_str_const(METEORA_DYNAMIC_AMM_V2),
                    accounts: keys,
                    data,
                });
            }

            SwapDirection::Sell => {
                // Sell: A (Token) → B (SOL)

                // Create destination ATA if needed (WSOL always uses standard Token program)
                if !context.destination_ata_exists {
                    instructions.push(create_associated_token_account_idempotent(
                        &context.user,
                        &context.user,
                        &self.pool.token_b_mint,
                        &spl_token::ID,
                    ));
                }

                // Build swap instruction (same as V1)
                let swap_args = SwapInstructionArgs {
                    in_amount: args.amount_in,
                    minimum_out_amount: args.min_amount_out,
                };

                // V2 has simplified account structure (no vault_lp accounts)
                let keys: Vec<AccountMeta> = vec![
                    AccountMeta::new(Pubkey::from_str_const(&self.market_pair.pair), false),
                    AccountMeta::new(context.source_ata, false),
                    AccountMeta::new(context.destination_ata, false),
                    AccountMeta::new(self.pool.token_a_vault, false),
                    AccountMeta::new(self.pool.token_b_vault, false),
                    AccountMeta::new_readonly(Pubkey::from_str_const(TOKEN_PROGRAM), false),
                    AccountMeta::new(context.user, true),
                ];

                // Discriminator for V2 swap
                let mut data = vec![248, 198, 158, 145, 225, 117, 135, 200];
                let mut args_bytes = to_vec(&swap_args)?;
                data.append(&mut args_bytes);

                instructions.push(Instruction {
                    program_id: Pubkey::from_str_const(METEORA_DYNAMIC_AMM_V2),
                    accounts: keys,
                    data,
                });
            }
        }

        Ok(instructions)
    }

    fn required_accounts(
        &self,
        _user: Pubkey,
        direction: SwapDirection,
    ) -> Result<RequiredAccounts, GenericError> {
        // Meteora DAMM V2: base_mint = Token, quote_mint = SOL (standard convention)
        let (source_mint, destination_mint) = match direction {
            SwapDirection::Buy => {
                // Buy: SOL → Token
                // Spend quote (SOL), receive base (Token)
                (self.pool.token_b_mint, self.pool.token_a_mint)
            }
            SwapDirection::Sell => {
                // Sell: Token → SOL
                // Spend base (Token), receive quote (SOL)
                (self.pool.token_a_mint, self.pool.token_b_mint)
            }
        };

        Ok(RequiredAccounts {
            source_mint,
            destination_mint,
            ata_checks: vec![],
            account_data: vec![],
            needs_pool_data: false,
        })
    }
}

