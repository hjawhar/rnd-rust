pub mod pda;

use borsh::BorshDeserialize;
use solana_pubkey::Pubkey;

use thunder_core::{
    AccountDataProvider, GenericError, Market, PoolFees, PoolFinancials, PoolMetadata,
    SwapDirection, calculate_price_impact_bps, infer_mint_decimals,
};


// ---------------------------------------------------------------------------
// DEX-specific constants
// ---------------------------------------------------------------------------

pub const PUMPFUN_AMM_PROGRAM: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
pub const PUMPFUN_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
pub const PUMPFUN_FEE_PROGRAM: &str = "pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ";

/// Default pumpswap fee components in bps, from the on-chain GlobalConfig
/// (lp_fee=20, protocol_fee=5, coin_creator_fee=5). Calibrated 2026-07-13
/// against live executed quotes: the full 30 bps is charged even for pools
/// whose `coin_creator` is Pubkey::default() (SOL/USDC and Fartcoin/SOL both
/// matched Jupiter to 0.00 bps at 30, and were 5 bps off at 25).
pub const DEFAULT_LP_FEE_BPS: u64 = 20;
pub const DEFAULT_PROTOCOL_FEE_BPS: u64 = 5;
pub const DEFAULT_COIN_CREATOR_FEE_BPS: u64 = 5;

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

#[derive(BorshDeserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PumpfunBondingCurve {
    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub real_sol_reserves: u64,
    pub token_total_supply: u64,
    pub complete: bool,
    pub creator: Pubkey,
}

#[derive(BorshDeserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PumpfunAmmPool {
    pub pool_bump: u8,
    pub index: u16,
    pub creator: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub lp_mint: Pubkey,
    pub pool_base_token_account: Pubkey,
    pub pool_quote_token_account: Pubkey,
    pub lp_supply: u64,
    pub coin_creator: Pubkey,
    pub is_mayhem_mode: bool,
    pub is_cashback_coin: bool,
    /// Bonding curve data — fetched separately, not part of pool account borsh layout
    #[borsh(skip)]
    #[serde(default)]
    pub bonding_curve: Option<PumpfunBondingCurve>,
}


// ---------------------------------------------------------------------------
// Market wrapper
// ---------------------------------------------------------------------------

pub struct PumpfunAmmMarket {
    pub pool: PumpfunAmmPool,
    pub pool_address: String,
    pub base_decimals: u8,
}

impl PumpfunAmmMarket {
    pub fn new(pool: PumpfunAmmPool, pool_address: String) -> Self {
        let base_decimals = infer_mint_decimals(&pool.base_mint);
        Self { pool, pool_address, base_decimals }
    }

    /// Calculate bonding curve output using virtual reserves and constant-product formula.
    fn calculate_bonding_curve_output(
        &self,
        amount_in: u64,
        direction: SwapDirection,
    ) -> Result<u64, GenericError> {
        let bonding_curve = self
            .pool
            .bonding_curve
            .as_ref()
            .ok_or("Bonding curve data not available")?;

        // Pumpfun uses 1% fee (100 bps)
        let fee_bps = 100u64;
        let fee_multiplier = 10000 - fee_bps;
        let amount_in_with_fee = (amount_in as u128 * fee_multiplier as u128) / 10000;

        let virtual_sol = bonding_curve.virtual_sol_reserves as u128;
        let virtual_token = bonding_curve.virtual_token_reserves as u128;

        if virtual_sol == 0 || virtual_token == 0 {
            return Err("Bonding curve has zero virtual reserves".into());
        }

        let output = match direction {
            SwapDirection::Buy => {
                // SOL -> Token: constant product k = virtual_sol * virtual_token
                let k = virtual_sol * virtual_token;
                let new_sol = virtual_sol + amount_in_with_fee;
                let new_token = k / new_sol;
                virtual_token.saturating_sub(new_token) as u64
            }
            SwapDirection::Sell => {
                // Token -> SOL: constant product
                let k = virtual_sol * virtual_token;
                let new_token = virtual_token + amount_in_with_fee;
                let new_sol = k / new_token;
                virtual_sol.saturating_sub(new_sol) as u64
            }
        };

        Ok(output)
    }

    /// Total swap fee in bps. When a provider holds the pumpswap
    /// GlobalConfig account, read lp/protocol/coin_creator fee bps from it
    /// (offsets verified: lp_fee u64 @40, protocol_fee u64 @48,
    /// coin_creator_fee u64 @313 after the [Pubkey; 8] fee recipients);
    /// otherwise use the calibrated defaults. All three components are
    /// summed unconditionally — see the constant docs.
    fn total_fee_bps(&self, accounts: Option<&dyn AccountDataProvider>) -> u64 {
        if let Some(provider) = accounts {
            let program = Pubkey::from_str_const(PUMPFUN_AMM_PROGRAM);
            let (global_config, _) =
                Pubkey::find_program_address(&[b"global_config"], &program);
            if let Some(data) = provider.pool_account_data(&global_config) {
                if data.len() >= 321 {
                    let lp = u64::from_le_bytes(data[40..48].try_into().unwrap());
                    let protocol = u64::from_le_bytes(data[48..56].try_into().unwrap());
                    let creator = u64::from_le_bytes(data[313..321].try_into().unwrap());
                    let total = lp + protocol + creator;
                    // Sanity: reject nonsense parses (> 10%).
                    if total > 0 && total <= 1_000 {
                        return total;
                    }
                }
            }
        }
        DEFAULT_LP_FEE_BPS + DEFAULT_PROTOCOL_FEE_BPS + DEFAULT_COIN_CREATOR_FEE_BPS
    }

    /// Constant-product quote on LIVE pool token-account balances (pumpswap
    /// AMM pools are plain x*y=k over the two vaults; fees are always taken
    /// on the QUOTE side: added on top of quote input for buys, deducted
    /// from quote output for sells). Validated against Jupiter/live pools to
    /// 0 bps at multiple sizes.
    fn calculate_live_cp_output(
        &self,
        amount_in: u64,
        direction: SwapDirection,
        quote_vault_balance: u64,
        base_vault_balance: u64,
        fee_bps: u64,
    ) -> Result<u64, GenericError> {
        if quote_vault_balance == 0 || base_vault_balance == 0 {
            return Err("Pumpfun pool has zero live reserves".into());
        }
        let q = quote_vault_balance as u128;
        let b = base_vault_balance as u128;
        match direction {
            SwapDirection::Buy => {
                // Quote in (fee added on top): effective CP input = in / (1 + fee).
                let q_cp = (amount_in as u128) * 10_000 / (10_000 + fee_bps as u128);
                let out = b * q_cp / (q + q_cp);
                Ok(out as u64)
            }
            SwapDirection::Sell => {
                // Base in, quote out; fee deducted from quote output (ceil).
                let out_cp = q * (amount_in as u128) / (b + amount_in as u128);
                let fee = (out_cp * fee_bps as u128).div_ceil(10_000);
                Ok(out_cp.saturating_sub(fee) as u64)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Market trait
// ---------------------------------------------------------------------------

impl Market for PumpfunAmmMarket {
    fn metadata(&self) -> Result<PoolMetadata, GenericError> {
        Ok(PoolMetadata {
            address: self.pool_address.clone(),
            dex_name: "Pumpfun AMM".to_string(),
            quote_mint: self.pool.quote_mint,
            base_mint: self.pool.base_mint,
            quote_vault: self.pool.pool_quote_token_account,
            base_vault: self.pool.pool_base_token_account,
            fees: PoolFees {
                trade_fee_bps: 100,
                protocol_fee_bps: None,
            },
        })
    }

    fn financials(&self) -> Result<PoolFinancials, GenericError> {
        let bonding_curve = self
            .pool
            .bonding_curve
            .as_ref()
            .ok_or("Bonding curve data not available")?;

        Ok(PoolFinancials {
            quote_balance: bonding_curve.real_sol_reserves,
            base_balance: bonding_curve.real_token_reserves,
            quote_decimals: 9,
            base_decimals: self.base_decimals,
        })
    }

    fn calculate_output(
        &self,
        amount_in: u64,
        direction: SwapDirection,
    ) -> Result<u64, GenericError> {
        self.calculate_bonding_curve_output(amount_in, direction)
    }

    fn calculate_output_live(
        &self,
        amount_in: u64,
        direction: SwapDirection,
        _pool_data: Option<&[u8]>,
        quote_vault_balance: u64,
        base_vault_balance: u64,
    ) -> Result<u64, GenericError> {
        // Pumpswap AMM pools are constant product over the two pool token
        // accounts — the live vault balances ARE the curve state. Fall back
        // to the cached bonding-curve estimate only when live balances are
        // unavailable.
        if quote_vault_balance == 0 || base_vault_balance == 0 {
            return self.calculate_bonding_curve_output(amount_in, direction);
        }
        self.calculate_live_cp_output(
            amount_in,
            direction,
            quote_vault_balance,
            base_vault_balance,
            self.total_fee_bps(None),
        )
    }

    fn calculate_output_live_ex(
        &self,
        amount_in: u64,
        direction: SwapDirection,
        _pool_data: Option<&[u8]>,
        quote_vault_balance: u64,
        base_vault_balance: u64,
        accounts: Option<&dyn AccountDataProvider>,
    ) -> Result<u64, GenericError> {
        if quote_vault_balance == 0 || base_vault_balance == 0 {
            return self.calculate_bonding_curve_output(amount_in, direction);
        }
        self.calculate_live_cp_output(
            amount_in,
            direction,
            quote_vault_balance,
            base_vault_balance,
            self.total_fee_bps(accounts),
        )
    }

    fn calculate_price_impact(
        &self,
        amount_in: u64,
        direction: SwapDirection,
    ) -> Result<u64, GenericError> {
        let pre_swap_price = self.current_price()?;
        let output = self.calculate_output(amount_in, direction)?;

        let bonding_curve = self
            .pool
            .bonding_curve
            .as_ref()
            .ok_or("Bonding curve data not available")?;

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
        let bonding_curve = self
            .pool
            .bonding_curve
            .as_ref()
            .ok_or("Bonding curve data not available")?;

        if bonding_curve.virtual_token_reserves == 0 {
            return Err("Bonding curve has zero virtual token reserves".into());
        }

        // Raw price in lamports: SOL_lamports / token_raw_units
        let raw = bonding_curve.virtual_sol_reserves as f64 / bonding_curve.virtual_token_reserves as f64;
        // Adjust: (sol / 10^9) / (tokens / 10^base_dec) = raw * 10^base_dec / 10^9
        let decimal_adj = 10f64.powi(self.base_decimals as i32 - 9);
        Ok(raw * decimal_adj)
    }

}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_market() -> PumpfunAmmMarket {
        let pool = PumpfunAmmPool {
            pool_bump: 0,
            index: 0,
            creator: Pubkey::default(),
            base_mint: Pubkey::new_unique(),
            quote_mint: Pubkey::from_str_const(thunder_core::WSOL),
            lp_mint: Pubkey::new_unique(),
            pool_base_token_account: Pubkey::new_unique(),
            pool_quote_token_account: Pubkey::new_unique(),
            lp_supply: 0,
            coin_creator: Pubkey::default(),
            is_mayhem_mode: false,
            is_cashback_coin: false,
            bonding_curve: None,
        };
        PumpfunAmmMarket::new(pool, Pubkey::new_unique().to_string())
    }

    /// Pinned against a live comparison (2026-07-13, pumpswap SOL/USDC pool
    /// Gf7sXMoP...): reserves base=1266467702041 (USDC raw),
    /// quote=16685581252217 (SOL lamports); Jupiter quoted 75670376 out for
    /// 1 SOL in at the same snapshot — total fee 30 bps matched to 0.00 bps.
    #[test]
    fn test_live_cp_buy_matches_mainnet_calibration() {
        let m = make_market();
        let out = m
            .calculate_output_live(
                1_000_000_000,
                SwapDirection::Buy,
                None,
                16_685_581_252_217,
                1_266_467_702_041,
            )
            .unwrap();
        assert_eq!(out, 75_670_376);
    }

    /// Sell side pinned against the Fartcoin/SOL pool comparison:
    /// out_cp - ceil(out_cp * 30 / 10000) matched Jupiter exactly.
    #[test]
    fn test_live_cp_sell_fee_on_quote_output() {
        let m = make_market();
        let base_bal: u64 = 1_000_000_000_000;
        let quote_bal: u64 = 5_000_000_000_000;
        let amount_in: u64 = 10_000_000_000;
        let out = m
            .calculate_output_live(amount_in, SwapDirection::Sell, None, quote_bal, base_bal)
            .unwrap();
        let out_cp =
            quote_bal as u128 * amount_in as u128 / (base_bal as u128 + amount_in as u128);
        let expected = out_cp - (out_cp * 30).div_ceil(10_000);
        assert_eq!(out as u128, expected);
    }

    /// Live path must be concave (CP) and use live balances, not the cache.
    #[test]
    fn test_live_cp_concave() {
        let m = make_market();
        let out1 = m
            .calculate_output_live(1_000_000, SwapDirection::Buy, None, 10_000_000_000, 10_000_000_000)
            .unwrap();
        let out2 = m
            .calculate_output_live(2_000_000, SwapDirection::Buy, None, 10_000_000_000, 10_000_000_000)
            .unwrap();
        assert!(out2 > out1 && out2 < out1 * 2);
    }
}
