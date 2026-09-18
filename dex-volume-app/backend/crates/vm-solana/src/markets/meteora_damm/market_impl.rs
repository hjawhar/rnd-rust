//! Phase 3: Market trait implementation for Meteora DAMM
//!
//! This module implements the generic Market trait for Meteora Dynamic AMM (DAMM).
//! DAMM supports both constant product and stable curve types.

use std::error::Error;
use solana_pubkey::Pubkey;
use solana_sdk::instruction::Instruction;

use crate::markets::meteora_damm::utils::meteora_dynamic_amm::derive_token_vault_address;
use crate::markets::traits::{
    Market, SwapArgs, SwapDirection, PoolMetadata, PoolFinancials, PoolFees,
    SwapContext, RequiredAccounts,
    constant_product_swap, calculate_price_impact_bps,
};
use crate::markets::meteora_damm::models::meteora_dynamic_amm::{MeteoraDAMMPool, CurveType};
use crate::markets::generic::market_pair::MarketPair;

type GenericError = Box<dyn Error + Send + Sync>;

#[derive(Clone, Debug, borsh::BorshSerialize)]
struct SwapInstructionArgs {
    pub in_amount: u64,
    pub minimum_out_amount: u64,
}

/// Wrapper for MeteoraDAMMPool that implements the Market trait
pub struct MeteoraDAMMMarket {
    pool: MeteoraDAMMPool,
    market_pair: MarketPair,
    /// Current vault balances (cached from last fetch)
    a_vault_balance: u64,
    b_vault_balance: u64,
}

impl MeteoraDAMMMarket {
    pub fn new(pool: MeteoraDAMMPool, market_pair: MarketPair) -> Self {
        Self {
            pool,
            market_pair,
            a_vault_balance: 0,
            b_vault_balance: 0,
        }
    }


    /// Calculate output based on curve type
    fn calculate_damm_output(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
        let fee_bps = (self.pool.fees.trade_fee_numerator as f64
            / self.pool.fees.trade_fee_denominator as f64 * 10000.0) as u64;

        match &self.pool.curve_type {
            CurveType::ConstantProduct => {
                // Use standard constant product formula
                match direction {
                    SwapDirection::Buy => {
                        // B → A (SOL → Token)
                        constant_product_swap(
                            self.b_vault_balance,
                            self.a_vault_balance,
                            amount_in,
                            fee_bps,
                        )
                    }
                    SwapDirection::Sell => {
                        // A → B (Token → SOL)
                        constant_product_swap(
                            self.a_vault_balance,
                            self.b_vault_balance,
                            amount_in,
                            fee_bps,
                        )
                    }
                }
            }
            CurveType::Stable { amp, .. } => {
                // Stable curve calculation (simplified)
                // Real implementation would use StableSwap invariant: An^n ∑x_i = AD + D^(n+1)/(n^n ∏x_i)
                // For now, use approximate calculation
                let amp_value = *amp;

                // With high amp, price impact is reduced (more stable)
                // Simplified: output ≈ constant_product * (1 + amp_factor)
                let base_output = match direction {
                    SwapDirection::Buy => {
                        constant_product_swap(
                            self.b_vault_balance,
                            self.a_vault_balance,
                            amount_in,
                            fee_bps,
                        )?
                    }
                    SwapDirection::Sell => {
                        constant_product_swap(
                            self.a_vault_balance,
                            self.b_vault_balance,
                            amount_in,
                            fee_bps,
                        )?
                    }
                };

                // Amplification factor reduces slippage
                let amp_factor = (amp_value as f64 / 100.0).min(10.0); // Cap at 10x
                let stable_output = (base_output as f64 * (1.0 + amp_factor / 100.0)) as u64;

                Ok(stable_output.min(match direction {
                    SwapDirection::Buy => self.a_vault_balance,
                    SwapDirection::Sell => self.b_vault_balance,
                }))
            }
        }
    }
}

impl Market for MeteoraDAMMMarket {
    fn metadata(&self) -> Result<PoolMetadata, GenericError> {
        let fee_bps = (self.pool.fees.trade_fee_numerator as f64
            / self.pool.fees.trade_fee_denominator as f64 * 10000.0) as u64;

        let protocol_fee_bps = (self.pool.fees.protocol_trade_fee_numerator as f64
            / self.pool.fees.protocol_trade_fee_denominator as f64 * 10000.0) as u64;

        Ok(PoolMetadata {
            address: self.market_pair.pair.clone(),
            dex_name: match self.pool.curve_type {
                CurveType::ConstantProduct => "Meteora DAMM (Constant Product)".to_string(),
                CurveType::Stable { .. } => "Meteora DAMM (Stable)".to_string(),
            },
            quote_mint: self.pool.token_b_mint, // B is usually quote (SOL)
            base_mint: self.pool.token_a_mint,  // A is usually base (token)
            quote_vault: derive_token_vault_address(self.pool.b_vault).0,
            base_vault: derive_token_vault_address(self.pool.a_vault).0,
            fees: PoolFees {
                trade_fee_bps: fee_bps,
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
        self.calculate_damm_output(amount_in, direction)
    }

    fn calculate_price_impact(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
        let pre_swap_price = self.current_price()?;

        // Calculate post-swap price
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
        if self.a_vault_balance == 0 {
            return Err("Pool has zero base balance".into());
        }

        // Price = B / A (quote / base)
        Ok(self.b_vault_balance as f64 / self.a_vault_balance as f64)
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
        use crate::utils::constants::{METEORA_DYNAMIC_AMM, METEORA_VAULT_PROGRAM, TOKEN_PROGRAM};
        use crate::markets::meteora_damm::utils::meteora_dynamic_amm::{derive_token_vault_address, derive_token_lp_mint};

        let mut instructions = Vec::new();

        // Check if pool is enabled
        if !self.pool.enabled {
            return Err("Pool is not enabled".into());
        }

        // Derive vault addresses (pure, no RPC needed!)
        let (a_token_vault, _) = derive_token_vault_address(self.pool.a_vault);
        let (b_token_vault, _) = derive_token_vault_address(self.pool.b_vault);

        // Extract LP mints from vault authority accounts (preferred) or derive (fallback)
        use crate::markets::meteora_damm::models::meteora_dynamic_amm_vault_authority::VaultAuthority;
        use borsh::BorshDeserialize;

        let a_vault_lp_mint = if let Some(account_data) = context.extra_accounts.get(&self.pool.a_vault.to_string()) {
            // Parse vault authority account (skip 8-byte discriminator)
            if account_data.len() > 8 {
                if let Ok(vault_auth) = VaultAuthority::deserialize(&mut &account_data[8..]) {
                    vault_auth.lp_mint
                } else {
                    // Fallback to derivation if deserialization fails
                    derive_token_lp_mint(self.pool.a_vault).0
                }
            } else {
                // Fallback to derivation if account data too short
                derive_token_lp_mint(self.pool.a_vault).0
            }
        } else {
            // Fallback to derivation if vault authority not fetched
            derive_token_lp_mint(self.pool.a_vault).0
        };

        let b_vault_lp_mint = if let Some(account_data) = context.extra_accounts.get(&self.pool.b_vault.to_string()) {
            // Parse vault authority account (skip 8-byte discriminator)
            if account_data.len() > 8 {
                if let Ok(vault_auth) = VaultAuthority::deserialize(&mut &account_data[8..]) {
                    vault_auth.lp_mint
                } else {
                    // Fallback to derivation if deserialization fails
                    derive_token_lp_mint(self.pool.b_vault).0
                }
            } else {
                // Fallback to derivation if account data too short
                derive_token_lp_mint(self.pool.b_vault).0
            }
        } else {
            // Fallback to derivation if vault authority not fetched
            derive_token_lp_mint(self.pool.b_vault).0
        };

        match direction {
            SwapDirection::Buy => {
                // Buy: B (SOL) → A (Token)
                // Meteora DAMM requires wrapping SOL into WSOL ATA before swap

                // Create source WSOL ATA if needed
                if !context.source_ata_exists {
                    instructions.push(create_associated_token_account_idempotent(
                        &context.user,
                        &context.user,
                        &self.pool.token_b_mint, // WSOL or TOKEN_2022 quote
                        &context.token_program_id, // Use detected token program
                    ));
                }

                // Create destination Token ATA if needed
                if !context.destination_ata_exists {
                    instructions.push(create_associated_token_account_idempotent(
                        &context.user,
                        &context.user,
                        &self.pool.token_a_mint, // Token
                        &context.token_program_id,
                    ));
                }

                // Wrap SOL: Transfer native SOL to WSOL ATA
                instructions.push(solana_system_interface::instruction::transfer(
                    &context.user,
                    &context.source_ata, // WSOL ATA
                    args.amount_in,       // SOL lamports to wrap
                ));

                // Sync the WSOL account to update its balance
                instructions.push(
                    spl_token::instruction::sync_native(
                        &context.token_program_id, // Use detected token program
                        &context.source_ata,
                    )?
                );

                // Build swap instruction
                let swap_args = SwapInstructionArgs {
                    in_amount: args.amount_in,
                    minimum_out_amount: args.min_amount_out,
                };

                let keys: Vec<AccountMeta> = vec![
                    AccountMeta::new(Pubkey::from_str_const(&self.market_pair.pair), false),
                    AccountMeta::new(context.source_ata, false),
                    AccountMeta::new(context.destination_ata, false),
                    AccountMeta::new(self.pool.a_vault, false),
                    AccountMeta::new(self.pool.b_vault, false),
                    AccountMeta::new(a_token_vault, false),
                    AccountMeta::new(b_token_vault, false),
                    AccountMeta::new(a_vault_lp_mint, false),
                    AccountMeta::new(b_vault_lp_mint, false),
                    AccountMeta::new(self.pool.a_vault_lp, false),
                    AccountMeta::new(self.pool.b_vault_lp, false),
                    AccountMeta::new(self.pool.protocol_token_b_fee, false), // Protocol fee for token B
                    AccountMeta::new(context.user, true),
                    AccountMeta::new(Pubkey::from_str_const(METEORA_VAULT_PROGRAM), false),
                    AccountMeta::new_readonly(Pubkey::from_str_const(TOKEN_PROGRAM), false),
                ];

                // Discriminator for swap: [248, 198, 158, 145, 225, 117, 135, 200]
                let mut data = vec![248, 198, 158, 145, 225, 117, 135, 200];
                let mut args_bytes = to_vec(&swap_args)?;
                data.append(&mut args_bytes);

                instructions.push(Instruction {
                    program_id: Pubkey::from_str_const(METEORA_DYNAMIC_AMM),
                    accounts: keys,
                    data,
                });

                // Close the WSOL account to unwrap remaining SOL back to native
                // This returns any leftover SOL from slippage/dust to the user
                instructions.push(
                    spl_token::instruction::close_account(
                        &context.token_program_id, // Use detected token program
                        &context.source_ata,  // WSOL ATA to close
                        &context.user,         // Send remaining SOL back to user
                        &context.user,         // User is the authority
                        &[],
                    )?
                );
            }

            SwapDirection::Sell => {
                // Sell: A (Token) → B (SOL)

                // Create source Token ATA if needed (must exist to sell from)
                if !context.source_ata_exists {
                    instructions.push(create_associated_token_account_idempotent(
                        &context.user,
                        &context.user,
                        &self.pool.token_a_mint, // Token being sold
                        &context.token_program_id,
                    ));
                }

                // Create destination WSOL ATA if needed
                if !context.destination_ata_exists {
                    instructions.push(create_associated_token_account_idempotent(
                        &context.user,
                        &context.user,
                        &self.pool.token_b_mint, // WSOL or TOKEN_2022 quote
                        &context.token_program_id, // Use detected token program
                    ));
                }

                // Build swap instruction
                let swap_args = SwapInstructionArgs {
                    in_amount: args.amount_in,
                    minimum_out_amount: args.min_amount_out,
                };

                let keys: Vec<AccountMeta> = vec![
                    AccountMeta::new(Pubkey::from_str_const(&self.market_pair.pair), false),
                    AccountMeta::new(context.source_ata, false),
                    AccountMeta::new(context.destination_ata, false),
                    AccountMeta::new(self.pool.a_vault, false),
                    AccountMeta::new(self.pool.b_vault, false),
                    AccountMeta::new(a_token_vault, false),
                    AccountMeta::new(b_token_vault, false),
                    AccountMeta::new(a_vault_lp_mint, false),
                    AccountMeta::new(b_vault_lp_mint, false),
                    AccountMeta::new(self.pool.a_vault_lp, false),
                    AccountMeta::new(self.pool.b_vault_lp, false),
                    AccountMeta::new(self.pool.protocol_token_a_fee, false), // Protocol fee for token A
                    AccountMeta::new(context.user, true),
                    AccountMeta::new(Pubkey::from_str_const(METEORA_VAULT_PROGRAM), false),
                    AccountMeta::new_readonly(Pubkey::from_str_const(TOKEN_PROGRAM), false),
                ];

                // Discriminator for swap: [248, 198, 158, 145, 225, 117, 135, 200]
                let mut data = vec![248, 198, 158, 145, 225, 117, 135, 200];
                let mut args_bytes = to_vec(&swap_args)?;
                data.append(&mut args_bytes);

                instructions.push(Instruction {
                    program_id: Pubkey::from_str_const(METEORA_DYNAMIC_AMM),
                    accounts: keys,
                    data,
                });

                // Close the WSOL account to unwrap received SOL back to native
                // Meteora returns WSOL, we need to unwrap it to get native SOL
                instructions.push(
                    spl_token::instruction::close_account(
                        &context.token_program_id, // Use detected token program
                        &context.destination_ata,  // WSOL ATA to close (received from swap)
                        &context.user,              // Send unwrapped SOL back to user
                        &context.user,              // User is the authority
                        &[],
                    )?
                );
            }
        }

        Ok(instructions)
    }

    fn required_accounts(
        &self,
        _user: Pubkey,
        direction: SwapDirection,
    ) -> Result<RequiredAccounts, GenericError> {
        // Meteora DAMM: base_mint = Token, quote_mint = SOL (standard convention)
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
            // Fetch vault authority accounts to get LP mints (fallback to derivation if needed)
            account_data: vec![self.pool.a_vault, self.pool.b_vault],
            needs_pool_data: false,
        })
    }
}

