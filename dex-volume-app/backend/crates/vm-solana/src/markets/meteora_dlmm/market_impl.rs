//! Phase 3: Market trait implementation for Meteora DLMM
//!
//! This module implements the generic Market trait for Meteora Dynamic Liquidity Market Maker (DLMM).
//! DLMM uses bin-based pricing similar to CLMM but with discrete price bins.

use std::error::Error;
use solana_pubkey::Pubkey;
use solana_sdk::instruction::Instruction;

use crate::markets::traits::{
    Market, SwapArgs, SwapDirection, PoolMetadata, PoolFinancials, PoolFees,
    SwapContext, RequiredAccounts,
    calculate_price_impact_bps,
};
use crate::markets::meteora_dlmm::models::meteora_dynamic_lmm::MeteoraDLMMPool;
use crate::markets::generic::market_pair::MarketPair;

type GenericError = Box<dyn Error + Send + Sync>;

#[derive(Clone, Debug, borsh::BorshSerialize)]
struct SwapInstructionArgs {
    pub amount_in: u64,
    pub min_amount_out: u64,
    pub remaining_accounts_info: Vec<u8>,
}

/// Wrapper for MeteoraDLMMPool that implements the Market trait
pub struct MeteoraDLMMMarket {
    pool: MeteoraDLMMPool,
    market_pair: MarketPair,
    /// Current reserve balances (cached from last fetch)
    reserve_x_balance: u64,
    reserve_y_balance: u64,
}

impl MeteoraDLMMMarket {
    pub fn new(pool: MeteoraDLMMPool, market_pair: MarketPair) -> Self {
        Self {
            pool,
            market_pair,
            reserve_x_balance: 0,
            reserve_y_balance: 0,
        }
    }


    /// Calculate output for DLMM swap
    ///
    /// Note: This is a simplified calculation. Real DLMM swaps traverse multiple bins.
    /// For production, use the actual DLMM math library or fetch from RPC simulation.
    fn calculate_dlmm_output(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
        // Get current bin price
        let price = self.current_price()?;

        // Calculate base fee from parameters
        // DLMM uses dynamic fees based on volatility
        let base_fee = self.pool.parameters.base_factor as u64; // in basis points
        let fee_bps = base_fee.max(10).min(200); // Clamp between 0.1% and 2%

        let fee_multiplier = 10000 - fee_bps;
        let amount_in_with_fee = (amount_in as u128 * fee_multiplier as u128) / 10000;

        // Simplified output calculation using current bin price
        let output = match direction {
            SwapDirection::Buy => {
                // Y → X (SOL → Token)
                // output ≈ amount_in / price
                (amount_in_with_fee as f64 / price) as u64
            }
            SwapDirection::Sell => {
                // X → Y (Token → SOL)
                // output ≈ amount_in * price
                (amount_in_with_fee as f64 * price) as u64
            }
        };

        Ok(output)
    }

    /// Get bin price from active_id
    ///
    /// DLMM uses: price = (1 + bin_step / 10000)^active_id
    fn bin_id_to_price(&self) -> f64 {
        let bin_step = self.pool.bin_step as f64;
        let active_id = self.pool.active_id;

        // price = (1 + bin_step / 10000)^active_id
        let base = 1.0 + (bin_step / 10000.0);
        base.powi(active_id)
    }
}

impl Market for MeteoraDLMMMarket {
    fn metadata(&self) -> Result<PoolMetadata, GenericError> {
        // DLMM has dynamic fees, use base fee
        let trade_fee_bps = self.pool.parameters.base_factor as u64;

        Ok(PoolMetadata {
            address: self.market_pair.pair.clone(),
            dex_name: "Meteora DLMM".to_string(),
            quote_mint: self.pool.token_y_mint, // Y is usually quote (SOL)
            base_mint: self.pool.token_x_mint,  // X is usually base (token)
            quote_vault: self.pool.reserve_y,
            base_vault: self.pool.reserve_x,
            fees: PoolFees {
                trade_fee_bps,
                protocol_fee_bps: Some(self.pool.parameters.protocol_share as u64),
            },
        })
    }

    fn financials(&self) -> Result<PoolFinancials, GenericError> {
        Ok(PoolFinancials {
            quote_balance: self.reserve_y_balance,
            base_balance: self.reserve_x_balance,
            quote_decimals: 9, // Assuming SOL (Y)
            base_decimals: 6,  // Common token decimals (X)
        })
    }

    fn calculate_output(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
        self.calculate_dlmm_output(amount_in, direction)
    }

    fn calculate_price_impact(&self, amount_in: u64, direction: SwapDirection) -> Result<u64, GenericError> {
        let pre_swap_price = self.current_price()?;

        // Calculate post-swap price (simplified - real DLMM would traverse bins)
        let output = self.calculate_output(amount_in, direction)?;

        let post_swap_price = match direction {
            SwapDirection::Buy => {
                // After buying X with Y, price increases slightly
                let new_reserve_y = self.reserve_y_balance + amount_in;
                let new_reserve_x = self.reserve_x_balance.saturating_sub(output);
                if new_reserve_x == 0 {
                    return Err("Insufficient liquidity in pool".into());
                }
                new_reserve_y as f64 / new_reserve_x as f64
            }
            SwapDirection::Sell => {
                // After selling X for Y, price decreases slightly
                let new_reserve_x = self.reserve_x_balance + amount_in;
                let new_reserve_y = self.reserve_y_balance.saturating_sub(output);
                if new_reserve_x == 0 {
                    return Err("Insufficient liquidity in pool".into());
                }
                new_reserve_y as f64 / new_reserve_x as f64
            }
        };

        Ok(calculate_price_impact_bps(pre_swap_price, post_swap_price))
    }

    fn current_price(&self) -> Result<f64, GenericError> {
        // Use bin-based pricing
        let price = self.bin_id_to_price();
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
        use spl_token::instruction::initialize_account;
        use solana_sdk::instruction::AccountMeta;
        use solana_sdk::program_pack::Pack;
        use solana_system_interface::instruction as system_instruction;
        use spl_token::state::Account as TokenAccount;
        use borsh::to_vec;
        use crate::utils::constants::{METEORA_DYNAMIC_LMM, METEORA_EVENTS_AUTHORITY, TOKEN_PROGRAM, MEMO_PROGRAM_V2, WSOL};
        use crate::markets::meteora_dlmm::utils::meteora_dynamic_lmm::derive_bin_array_pda;

        let mut instructions = Vec::new();

        const TOKEN_ACCOUNT_RENT: u64 = 2_039_280;

        let native_mint = spl_token::native_mint::ID;
        let wsol = Pubkey::from_str_const(WSOL);
        let meteora_dlmm_program = Pubkey::from_str_const(METEORA_DYNAMIC_LMM);
        // Derive bin array PDA from active bin (pure, no RPC!)
        const MAX_BIN_PER_ARRAY: i64 = 70;
        let bin_array_index = (self.pool.active_id as i64).div_euclid(MAX_BIN_PER_ARRAY);
        let (bin_array_pda, _) = derive_bin_array_pda(Pubkey::from_str_const(&self.market_pair.pair), bin_array_index);

        // Determine correct token programs for Token X and Token Y
        // Token X = non-SOL token (may be Token-2022), Token Y = SOL (always standard Token)
        let token_x_program = if self.pool.token_x_mint == wsol {
            Pubkey::from_str_const(TOKEN_PROGRAM)
        } else {
            context.token_program_id
        };
        let token_y_program = if self.pool.token_y_mint == wsol {
            Pubkey::from_str_const(TOKEN_PROGRAM)
        } else {
            context.token_program_id
        };

        match direction {
            SwapDirection::Buy => {
                // Buy: Y (SOL) → X (Token)

                // 1. Create temporary WSOL account for input
                let seed = &format!("{}", context.user)[..32];
                let wsol_pubkey = Pubkey::create_with_seed(
                    &context.user,
                    seed,
                    &spl_token::id(),
                )?;

                let total_amount = TOKEN_ACCOUNT_RENT + args.amount_in;

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

                // 3. Create destination ATA if needed (token uses detected program)
                if !context.destination_ata_exists {
                    instructions.push(create_associated_token_account_idempotent(
                        &context.user,
                        &context.user,
                        &self.pool.token_x_mint,
                        &context.token_program_id,
                    ));
                }

                // 4. Build swap instruction
                let swap_args = SwapInstructionArgs {
                    amount_in: args.amount_in,
                    min_amount_out: args.min_amount_out,
                    remaining_accounts_info: vec![],
                };

                let keys: Vec<AccountMeta> = vec![
                    AccountMeta::new(Pubkey::from_str_const(&self.market_pair.pair), false),
                    AccountMeta::new_readonly(meteora_dlmm_program, false),
                    AccountMeta::new(self.pool.reserve_x, false),
                    AccountMeta::new(self.pool.reserve_y, false),
                    AccountMeta::new(wsol_pubkey, false),  // user_token_in = temp WSOL
                    AccountMeta::new(context.destination_ata, false),
                    AccountMeta::new_readonly(self.pool.token_x_mint, false),
                    AccountMeta::new_readonly(self.pool.token_y_mint, false),
                    AccountMeta::new(self.pool.oracle, false),
                    AccountMeta::new_readonly(meteora_dlmm_program, false), // Host Fee In
                    AccountMeta::new(context.user, true),
                    AccountMeta::new_readonly(token_x_program, false), // Token X Program
                    AccountMeta::new_readonly(token_y_program, false), // Token Y Program
                    AccountMeta::new_readonly(Pubkey::from_str_const(MEMO_PROGRAM_V2), false),
                    AccountMeta::new_readonly(Pubkey::from_str_const(METEORA_EVENTS_AUTHORITY), false),
                    AccountMeta::new_readonly(meteora_dlmm_program, false), // Program
                    AccountMeta::new(bin_array_pda, false),
                ];

                // Discriminator for DLMM swap
                let mut data = vec![65, 75, 63, 76, 235, 91, 91, 136];
                let mut args_bytes = to_vec(&swap_args)?;
                data.append(&mut args_bytes);

                instructions.push(Instruction {
                    program_id: meteora_dlmm_program,
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
                // Sell: X (Token) → Y (SOL)

                // 1. Create temporary WSOL account for output
                let seed = &format!("{}", context.user)[..32];
                let wsol_pubkey = Pubkey::create_with_seed(
                    &context.user,
                    seed,
                    &spl_token::id(),
                )?;

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

                // 3. Build swap instruction
                let swap_args = SwapInstructionArgs {
                    amount_in: args.amount_in,
                    min_amount_out: args.min_amount_out,
                    remaining_accounts_info: vec![],
                };

                let keys: Vec<AccountMeta> = vec![
                    AccountMeta::new(Pubkey::from_str_const(&self.market_pair.pair), false),
                    AccountMeta::new_readonly(meteora_dlmm_program, false),
                    AccountMeta::new(self.pool.reserve_x, false),
                    AccountMeta::new(self.pool.reserve_y, false),
                    AccountMeta::new(context.source_ata, false),
                    AccountMeta::new(wsol_pubkey, false),  // user_token_out = temp WSOL
                    AccountMeta::new_readonly(self.pool.token_x_mint, false),
                    AccountMeta::new_readonly(self.pool.token_y_mint, false),
                    AccountMeta::new(self.pool.oracle, false),
                    AccountMeta::new_readonly(meteora_dlmm_program, false), // Host Fee In
                    AccountMeta::new(context.user, true),
                    AccountMeta::new_readonly(token_x_program, false), // Token X Program
                    AccountMeta::new_readonly(token_y_program, false), // Token Y Program
                    AccountMeta::new_readonly(Pubkey::from_str_const(MEMO_PROGRAM_V2), false),
                    AccountMeta::new_readonly(Pubkey::from_str_const(METEORA_EVENTS_AUTHORITY), false),
                    AccountMeta::new_readonly(meteora_dlmm_program, false), // Program
                    AccountMeta::new(bin_array_pda, false),
                ];

                // Discriminator for DLMM swap
                let mut data = vec![65, 75, 63, 76, 235, 91, 91, 136];
                let mut args_bytes = to_vec(&swap_args)?;
                data.append(&mut args_bytes);

                instructions.push(Instruction {
                    program_id: meteora_dlmm_program,
                    accounts: keys,
                    data,
                });

                // 4. Close temporary WSOL account
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
        // Meteora DLMM: base_mint = Token, quote_mint = SOL (standard convention)
        let (source_mint, destination_mint) = match direction {
            SwapDirection::Buy => {
                // Buy: SOL → Token
                // Spend quote (SOL), receive base (Token)
                (self.pool.token_y_mint, self.pool.token_x_mint)
            }
            SwapDirection::Sell => {
                // Sell: Token → SOL
                // Spend base (Token), receive quote (SOL)
                (self.pool.token_x_mint, self.pool.token_y_mint)
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

// Implement ConcentratedLiquidity trait for DLMM (it's bin-based concentrated liquidity)
impl crate::markets::traits::ConcentratedLiquidity for MeteoraDLMMMarket {
    fn active_bin(&self) -> Result<i32, GenericError> {
        Ok(self.pool.active_id)
    }

    fn liquidity_distribution(&self) -> Result<Vec<(i32, u64)>, GenericError> {
        // For now, return current bin with simplified liquidity
        // In production, this would traverse bin arrays
        let liquidity = (self.reserve_x_balance + self.reserve_y_balance) / 2;
        Ok(vec![(self.pool.active_id, liquidity)])
    }
}

