//! Phase 3: Market trait implementation for Pumpfun AMM
//!
//! This module implements the generic Market trait for Pumpfun's bonding curve AMM.
//! Pumpfun uses a bonding curve mechanism with virtual and real reserves.

use std::error::Error;
use solana_pubkey::Pubkey;
use solana_sdk::instruction::Instruction;
use solana_system_interface::instruction as system_instruction;

use crate::markets::traits::{
    Market, SwapArgs, SwapDirection, PoolMetadata, PoolFinancials, PoolFees,
    SwapContext, RequiredAccounts,
    calculate_price_impact_bps,
};
use crate::markets::pumpfun_amm::models::pumpfun_amm_pool::PumpfunAmmPool;
use crate::markets::generic::market_pair::MarketPair;

type GenericError = Box<dyn Error + Send + Sync>;

/// Args for buy instruction (Pumpfun AMM)
/// Specify exact tokens out, max SOL in
#[derive(Clone, Debug, borsh::BorshSerialize)]
struct BuyArgs {
    pub base_amount_out: u64,
    pub max_quote_amount_in: u64,
    pub track_volume: bool, // OptionBool wraps a single bool
}

#[derive(Clone, Debug, borsh::BorshSerialize)]
struct SellSwapInstructionArgs {
    pub base_amount_in: u64,
    pub min_quote_amount_out: u64,
}

/// Wrapper for PumpfunAmmPool that implements the Market trait
pub struct PumpfunAMMMarket {
    pool: PumpfunAmmPool,
    market_pair: MarketPair,
}

impl PumpfunAMMMarket {
    pub fn new(pool: PumpfunAmmPool, market_pair: MarketPair) -> Self {
        Self {
            pool,
            market_pair,
        }
    }

    /// Calculate bonding curve output
    ///
    /// Pumpfun uses virtual reserves for pricing: price = virtual_sol / virtual_token
    /// For constant product: virtual_sol * virtual_token = k
    fn calculate_bonding_curve_output(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
        let bonding_curve = match &self.pool.bonding_curve {
            Some(bc) => bc,
            None => return Err("Bonding curve data not available".into()),
        };

        // Pumpfun typically uses 1% fee (100 bps)
        let fee_bps = 100u64;
        let fee_multiplier = 10000 - fee_bps;
        let amount_in_with_fee = (amount_in as u128 * fee_multiplier as u128) / 10000;

        // Use virtual reserves for pricing
        let virtual_sol = bonding_curve.virtual_sol_reserves as u128;
        let virtual_token = bonding_curve.virtual_token_reserves as u128;

        if virtual_sol == 0 || virtual_token == 0 {
            return Err("Bonding curve has zero virtual reserves".into());
        }

        let output = match direction {
            SwapDirection::Buy => {
                // SOL → Token (base_amount_out)
                // k = virtual_sol * virtual_token
                // After swap: (virtual_sol + amount_in) * (virtual_token - amount_out) = k
                // amount_out = virtual_token - k / (virtual_sol + amount_in)
                let k = virtual_sol * virtual_token;
                let new_sol = virtual_sol + amount_in_with_fee;
                let new_token = k / new_sol;
                let amount_out = virtual_token.saturating_sub(new_token);
                amount_out as u64
            }
            SwapDirection::Sell => {
                // Token → SOL (quote_amount_out)
                // k = virtual_sol * virtual_token
                // After swap: (virtual_sol - amount_out) * (virtual_token + amount_in) = k
                // amount_out = virtual_sol - k / (virtual_token + amount_in)
                let k = virtual_sol * virtual_token;
                let new_token = virtual_token + amount_in_with_fee;
                let new_sol = k / new_token;
                let amount_out = virtual_sol.saturating_sub(new_sol);
                amount_out as u64
            }
        };

        Ok(output)
    }
}

impl Market for PumpfunAMMMarket {
    fn metadata(&self) -> Result<PoolMetadata, GenericError> {
        Ok(PoolMetadata {
            address: self.market_pair.pair.clone(),
            dex_name: "Pumpfun AMM".to_string(),
            quote_mint: self.pool.quote_mint, // Token (being launched)
            base_mint: self.pool.base_mint,   // SOL (base currency)
            quote_vault: self.pool.pool_quote_token_account,
            base_vault: self.pool.pool_base_token_account,
            fees: PoolFees {
                trade_fee_bps: 100, // 1% typical Pumpfun fee
                protocol_fee_bps: None,
            },
        })
    }

    fn financials(&self) -> Result<PoolFinancials, GenericError> {
        let bonding_curve = match &self.pool.bonding_curve {
            Some(bc) => bc,
            None => return Err("Bonding curve data not available".into()),
        };

        Ok(PoolFinancials {
            quote_balance: bonding_curve.real_token_reserves,
            base_balance: bonding_curve.real_sol_reserves,
            quote_decimals: 6, // Typical token decimals
            base_decimals: 9,  // SOL decimals
        })
    }

    fn calculate_output(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
        self.calculate_bonding_curve_output(amount_in, direction)
    }

    fn calculate_price_impact(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
        let pre_swap_price = self.current_price()?;

        // Calculate post-swap price
        let output = self.calculate_output(amount_in, direction)?;

        let bonding_curve = match &self.pool.bonding_curve {
            Some(bc) => bc,
            None => return Err("Bonding curve data not available".into()),
        };

        let post_swap_price = match direction {
            SwapDirection::Buy => {
                let new_sol = bonding_curve.virtual_sol_reserves + amount_in;
                let new_token = bonding_curve.virtual_token_reserves.saturating_sub(output);
                if new_token == 0 {
                    return Err("Insufficient liquidity in bonding curve".into());
                }
                new_sol as f64 / new_token as f64
            }
            SwapDirection::Sell => {
                let new_token = bonding_curve.virtual_token_reserves + amount_in;
                let new_sol = bonding_curve.virtual_sol_reserves.saturating_sub(output);
                if new_token == 0 {
                    return Err("Insufficient liquidity in bonding curve".into());
                }
                new_sol as f64 / new_token as f64
            }
        };

        Ok(calculate_price_impact_bps(pre_swap_price, post_swap_price))
    }

    fn current_price(&self) -> Result<f64, GenericError> {
        let bonding_curve = match &self.pool.bonding_curve {
            Some(bc) => bc,
            None => return Err("Bonding curve data not available".into()),
        };

        if bonding_curve.virtual_token_reserves == 0 {
            return Err("Bonding curve has zero virtual token reserves".into());
        }

        // Price = virtual_sol / virtual_token
        let price = bonding_curve.virtual_sol_reserves as f64 / bonding_curve.virtual_token_reserves as f64;
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
        use crate::utils::constants::{PUMPFUN_AMM_PROGRAM, PUMPFUN_FEE_PROGRAM, TOKEN_PROGRAM};
        use crate::markets::pumpfun_amm::utils::pumpfun::{
            get_global_volume_accumulator_pda, get_pool_v2_pda, get_pumpfun_config_pda,
            get_pumpfun_creator_vault_ata, get_pumpfun_creator_vault_authority_pda,
            get_user_volume_accumulator_pda,
        };
        use solana_sdk::pubkey;

        let mut instructions = Vec::new();

        // Create ATAs if needed (from context!)
        match direction {
            SwapDirection::Buy => {
                // Buy: SOL → Token
                // Pumpfun requires wrapping SOL into WSOL ATA before swap
                // Source: WSOL ATA, Destination: Token ATA

                // Create source WSOL ATA if needed (always TOKEN_PROGRAM)
                if !context.source_ata_exists {
                    instructions.push(create_associated_token_account_idempotent(
                        &context.user,
                        &context.user,
                        &self.pool.quote_mint, // WSOL (quote_mint for Pumpfun)
                        &spl_token::ID,
                    ));
                }

                // Create destination token ATA if needed (this is the token being bought)
                if !context.destination_ata_exists {
                    instructions.push(create_associated_token_account_idempotent(
                        &context.user,
                        &context.user,
                        &self.pool.base_mint, // Pumpfun: base_mint is the token being launched
                        &context.token_program_id, // Use detected token program (TOKEN_PROGRAM or TOKEN_2022_PROGRAM)
                    ));
                }

                // Wrap SOL: Transfer native SOL to WSOL ATA
                instructions.push(system_instruction::transfer(
                    &context.user,
                    &context.source_ata, // WSOL ATA
                    args.amount_in,       // SOL lamports to wrap
                ));

                // Sync the WSOL account to update its balance
                instructions.push(
                    spl_token::instruction::sync_native(
                        &spl_token::ID,
                        &context.source_ata,
                    )?
                );

                // Pumpfun buy: specify exact tokens out, cap max SOL in
                let swap_args = BuyArgs {
                    base_amount_out: args.min_amount_out,      // Exact tokens we want to receive
                    max_quote_amount_in: args.amount_in,       // Max SOL lamports we're willing to spend
                    track_volume: false,
                };

                let protocol_fee_recipient = pubkey!("62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV");
                let protocol_fee_recipient_token_account =
                    pubkey!("94qWNrtmfn42h3ZjUZwWvK1MEo9uVmmrBPd2hpNjYDjb");

                let coin_creator_vault_authority =
                    get_pumpfun_creator_vault_authority_pda(self.pool.coin_creator);
                let coin_creator_vault_ata =
                    get_pumpfun_creator_vault_ata(coin_creator_vault_authority, self.pool.quote_mint);

                let global_volume_acc_pda = get_global_volume_accumulator_pda();
                let user_volume_acc_pda = get_user_volume_accumulator_pda(context.user);
                let pool_v2_pda = get_pool_v2_pda(self.pool.base_mint);

                let keys: Vec<AccountMeta> = vec![
                    AccountMeta::new(Pubkey::from_str_const(&self.market_pair.pair), false),           // 1. pool
                    AccountMeta::new(context.user, true),                                              // 2. user (signer)
                    AccountMeta::new_readonly(                                                         // 3. global_config
                        Pubkey::from_str_const("ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw"), false,
                    ),
                    AccountMeta::new_readonly(self.pool.base_mint, false),                             // 4. base_mint
                    AccountMeta::new_readonly(self.pool.quote_mint, false),                            // 5. quote_mint
                    AccountMeta::new(context.destination_ata, false),                                  // 6. user_base_token_account
                    AccountMeta::new(context.source_ata, false),                                      // 7. user_quote_token_account
                    AccountMeta::new(self.pool.pool_base_token_account, false),                        // 8. pool_base_token_account
                    AccountMeta::new(self.pool.pool_quote_token_account, false),                       // 9. pool_quote_token_account
                    AccountMeta::new_readonly(protocol_fee_recipient, false),                          // 10. protocol_fee_recipient
                    AccountMeta::new(protocol_fee_recipient_token_account, false),                     // 11. protocol_fee_recipient_token_account
                    AccountMeta::new_readonly(Pubkey::from_str_const(TOKEN_PROGRAM), false),           // 12. base_token_program
                    AccountMeta::new_readonly(Pubkey::from_str_const(TOKEN_PROGRAM), false),           // 13. quote_token_program
                    AccountMeta::new_readonly(                                                         // 14. system_program
                        Pubkey::from_str_const("11111111111111111111111111111111"), false,
                    ),
                    AccountMeta::new_readonly(                                                         // 15. associated_token_program
                        Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"), false,
                    ),
                    AccountMeta::new_readonly(                                                         // 16. event_authority
                        Pubkey::from_str_const("GS4CU59F31iL7aR2Q8zVS8DRrcRnXX1yjQ66TqNVQnaR"), false,
                    ),
                    AccountMeta::new_readonly(Pubkey::from_str_const(PUMPFUN_AMM_PROGRAM), false),    // 17. program (self)
                    AccountMeta::new(coin_creator_vault_ata, false),                                  // 18. coin_creator_vault_ata
                    AccountMeta::new_readonly(coin_creator_vault_authority, false),                    // 19. coin_creator_vault_authority
                    AccountMeta::new_readonly(global_volume_acc_pda, false),                           // 20. global_volume_accumulator
                    AccountMeta::new(user_volume_acc_pda, false),                                     // 21. user_volume_accumulator
                    AccountMeta::new_readonly(get_pumpfun_config_pda(), false),                        // 22. fee_config
                    AccountMeta::new_readonly(Pubkey::from_str_const(PUMPFUN_FEE_PROGRAM), false),    // 23. fee_program
                    AccountMeta::new_readonly(pool_v2_pda, false),                                    // 24. pool_v2 (required for program upgrade)
                ];

                // Discriminator for buy: [102, 6, 61, 18, 1, 218, 235, 234]
                let mut data = vec![102, 6, 61, 18, 1, 218, 235, 234];
                let mut args_bytes = to_vec(&swap_args)?;
                data.append(&mut args_bytes);

                instructions.push(Instruction {
                    program_id: Pubkey::from_str_const(PUMPFUN_AMM_PROGRAM),
                    accounts: keys,
                    data,
                });

                // Close the WSOL account to get remaining SOL back
                instructions.push(
                    spl_token::instruction::close_account(
                        &spl_token::ID,
                        &context.source_ata,  // WSOL ATA to close
                        &context.user,         // Send remaining SOL back to user
                        &context.user,         // User is the authority
                        &[],
                    )?
                );
            }

            SwapDirection::Sell => {
                // Sell: Token → SOL
                // Source: Token ATA, Destination: WSOL ATA (then unwrapped to native SOL)

                // Create source Token ATA if needed (this is the token being sold)
                if !context.source_ata_exists {
                    instructions.push(create_associated_token_account_idempotent(
                        &context.user,
                        &context.user,
                        &self.pool.base_mint, // Token (base_mint for Pumpfun)
                        &context.token_program_id, // Use detected token program
                    ));
                }

                // Create destination WSOL ATA if needed (Pumpfun requires this even for sells)
                if !context.destination_ata_exists {
                    instructions.push(create_associated_token_account_idempotent(
                        &context.user,
                        &context.user,
                        &self.pool.quote_mint, // WSOL (quote_mint for Pumpfun)
                        &spl_token::ID, // WSOL always uses TOKEN_PROGRAM
                    ));
                }

                // Pumpfun Sell: selling tokens for SOL
                let swap_args = SellSwapInstructionArgs {
                    base_amount_in: args.amount_in,           // Tokens we're selling
                    min_quote_amount_out: args.min_amount_out, // Min SOL we want
                };

                let protocol_fee_recipient = pubkey!("62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV");
                let protocol_fee_recipient_token_account =
                    pubkey!("94qWNrtmfn42h3ZjUZwWvK1MEo9uVmmrBPd2hpNjYDjb");

                let coin_creator_vault_authority =
                    get_pumpfun_creator_vault_authority_pda(self.pool.coin_creator);
                let coin_creator_vault_ata =
                    get_pumpfun_creator_vault_ata(coin_creator_vault_authority, self.pool.quote_mint);
                let pool_v2_pda = get_pool_v2_pda(self.pool.base_mint);

                let keys: Vec<AccountMeta> = vec![
                    AccountMeta::new(Pubkey::from_str_const(&self.market_pair.pair), false),           // 1. pool
                    AccountMeta::new(context.user, true),                                              // 2. user (signer)
                    AccountMeta::new_readonly(                                                         // 3. global_config
                        Pubkey::from_str_const("ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw"), false,
                    ),
                    AccountMeta::new_readonly(self.pool.base_mint, false),                             // 4. base_mint
                    AccountMeta::new_readonly(self.pool.quote_mint, false),                            // 5. quote_mint
                    AccountMeta::new(context.source_ata, false),                                      // 6. user_base_token_account
                    AccountMeta::new(context.destination_ata, false),                                  // 7. user_quote_token_account
                    AccountMeta::new(self.pool.pool_base_token_account, false),                        // 8. pool_base_token_account
                    AccountMeta::new(self.pool.pool_quote_token_account, false),                       // 9. pool_quote_token_account
                    AccountMeta::new_readonly(protocol_fee_recipient, false),                          // 10. protocol_fee_recipient
                    AccountMeta::new(protocol_fee_recipient_token_account, false),                     // 11. protocol_fee_recipient_token_account
                    AccountMeta::new_readonly(Pubkey::from_str_const(TOKEN_PROGRAM), false),           // 12. base_token_program
                    AccountMeta::new_readonly(Pubkey::from_str_const(TOKEN_PROGRAM), false),           // 13. quote_token_program
                    AccountMeta::new_readonly(                                                         // 14. system_program
                        Pubkey::from_str_const("11111111111111111111111111111111"), false,
                    ),
                    AccountMeta::new_readonly(                                                         // 15. associated_token_program
                        Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"), false,
                    ),
                    AccountMeta::new_readonly(                                                         // 16. event_authority
                        Pubkey::from_str_const("GS4CU59F31iL7aR2Q8zVS8DRrcRnXX1yjQ66TqNVQnaR"), false,
                    ),
                    AccountMeta::new_readonly(Pubkey::from_str_const(PUMPFUN_AMM_PROGRAM), false),    // 17. program (self)
                    AccountMeta::new(coin_creator_vault_ata, false),                                  // 18. coin_creator_vault_ata
                    AccountMeta::new_readonly(coin_creator_vault_authority, false),                    // 19. coin_creator_vault_authority
                    AccountMeta::new_readonly(get_pumpfun_config_pda(), false),                        // 20. fee_config
                    AccountMeta::new_readonly(Pubkey::from_str_const(PUMPFUN_FEE_PROGRAM), false),    // 21. fee_program
                    AccountMeta::new_readonly(pool_v2_pda, false),                                    // 22. pool_v2 (required for program upgrade)
                ];

                // Discriminator for sell: [51, 230, 133, 164, 1, 127, 131, 173]
                let mut data = vec![51, 230, 133, 164, 1, 127, 131, 173];
                let mut args_bytes = to_vec(&swap_args)?;
                data.append(&mut args_bytes);

                instructions.push(Instruction {
                    program_id: Pubkey::from_str_const(PUMPFUN_AMM_PROGRAM),
                    accounts: keys,
                    data,
                });

                // Close the WSOL account to unwrap SOL back to native and get it back
                instructions.push(
                    spl_token::instruction::close_account(
                        &spl_token::ID,
                        &context.destination_ata,  // WSOL ATA to close
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
        // Pumpfun: base_mint = Token, quote_mint = WSOL
        let (source_mint, destination_mint) = match direction {
            SwapDirection::Buy => {
                // Buy: SOL → Token
                // Spend quote (WSOL), receive base (Token)
                (self.pool.quote_mint, self.pool.base_mint)
            }
            SwapDirection::Sell => {
                // Sell: Token → SOL
                // Spend base (Token), receive quote (WSOL)
                (self.pool.base_mint, self.pool.quote_mint)
            }
        };

        Ok(RequiredAccounts {
            source_mint,
            destination_mint,
            ata_checks: vec![],      // ATAs checked by orchestrator automatically
            account_data: vec![],     // No extra accounts needed
            needs_pool_data: false,   // Pool data already in struct
        })
    }
}

// Implement BondingCurve trait for Pumpfun
impl crate::markets::traits::BondingCurve for PumpfunAMMMarket {
    fn price_at_supply(&self, supply: u64) -> Result<f64, GenericError> {
        let bonding_curve = match &self.pool.bonding_curve {
            Some(bc) => bc,
            None => return Err("Bonding curve data not available".into()),
        };

        // Calculate price at a given supply level
        // For constant product bonding curve: price = virtual_sol / (total_supply - supply)
        let remaining_tokens = bonding_curve.token_total_supply.saturating_sub(supply);
        if remaining_tokens == 0 {
            return Err("Supply exceeds total token supply".into());
        }

        let price = bonding_curve.virtual_sol_reserves as f64 / remaining_tokens as f64;
        Ok(price)
    }

    fn is_graduated(&self) -> Result<bool, GenericError> {
        let bonding_curve = match &self.pool.bonding_curve {
            Some(bc) => bc,
            None => return Err("Bonding curve data not available".into()),
        };

        Ok(bonding_curve.complete)
    }
}

