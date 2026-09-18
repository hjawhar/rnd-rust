pub mod models;
pub mod utils;

pub use models::{
    MeteoraDAMMPool, MeteoraDAMMV2Pool, VaultAuthority, CurveType, Config,
    V2PoolFees, BaseFee, PoolMetrics, DynamicFee, RewardInfo,
};
pub use utils::derive_token_vault_address;

use thunder_core::math::{mul_div_ceil, U512};
use thunder_core::{
    GenericError, Market, PoolFinancials, PoolFees, PoolMetadata,
    SwapDirection, calculate_price_impact_bps, infer_mint_decimals, quote_priority,
};

// =============================================================================
// DAMM v2 (cp-amm) fixed-point primitives
// =============================================================================
//
// IMPORTANT scaling note (verified against MeteoraAg/damm-v2
// `liquidity_handler/concentrated_liquidity.rs`): cp-amm stores `liquidity`
// with 64 MORE fractional bits than Raydium CLMM. sqrt_price is Q64.64, but:
//   delta_a = L * (√Pu - √Pl) / (√Pu * √Pl)        (no << 64 on numerator)
//   delta_b = L * (√Pu - √Pl) >> 128               (not >> 64)
//   √P'(a in) = ceil(L * √P / (L + Δa * √P))
//   √P'(b in) = √P + floor((Δb << 128) / L)

/// √P' after adding `amount` of token_a (price moves DOWN), rounding up.
fn v2_next_sqrt_price_from_a_in(sqrt_price: u128, liquidity: u128, amount: u128) -> Option<u128> {
    if amount == 0 {
        return Some(sqrt_price);
    }
    let l = U512::from_u128(liquidity);
    let product = U512::from_u128(amount).checked_mul_u128(sqrt_price)?;
    let denominator = l.checked_add(&product)?;
    let n = l.checked_mul_u128(sqrt_price)?;
    let (q, r) = n.div_rem(&denominator)?;
    let q = q.to_u128()?;
    if r.is_zero() { Some(q) } else { q.checked_add(1) }
}

/// √P' after adding `amount` of token_b (price moves UP), rounding down.
fn v2_next_sqrt_price_from_b_in(sqrt_price: u128, liquidity: u128, amount: u128) -> Option<u128> {
    if amount == 0 {
        return Some(sqrt_price);
    }
    if liquidity == 0 {
        return None;
    }
    let quotient = U512::from_u128(amount)
        .checked_shl64()?
        .checked_shl64()?
        .div_rem(&U512::from_u128(liquidity))?
        .0
        .to_u128()?;
    sqrt_price.checked_add(quotient)
}

/// Δa between two sqrt prices (floor): L * (√Pu - √Pl) / (√Pu * √Pl).
fn v2_delta_a(lower: u128, upper: u128, liquidity: u128) -> Option<u128> {
    if lower == 0 || upper < lower {
        return None;
    }
    let num = U512::from_u128(liquidity).checked_mul_u128(upper - lower)?;
    let denom = U512::from_u128(lower).checked_mul_u128(upper)?;
    num.div_rem(&denom)?.0.to_u128()
}

/// Δb between two sqrt prices (floor): L * (√Pu - √Pl) >> 128.
fn v2_delta_b(lower: u128, upper: u128, liquidity: u128) -> Option<u128> {
    if upper < lower {
        return None;
    }
    let prod = U512::from_u128(liquidity).checked_mul_u128(upper - lower)?;
    let denom = U512::from_u128(1).checked_shl64()?.checked_shl64()?;
    prod.div_rem(&denom)?.0.to_u128()
}

// =============================================================================
// Constants
// =============================================================================

pub const METEORA_DYNAMIC_AMM: &str = "Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB";
pub const METEORA_DYNAMIC_AMM_V2: &str = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG";
pub const METEORA_VAULT_PROGRAM: &str = "24Uqj9JCLxUeoC3hGfh5W3s9FM9uCHDS2SG3LYwBpyTi";


// =============================================================================
// DAMM V1 (dynamic-amm) vault-share math
// =============================================================================
//
// V1 pools do NOT own their reserves: funds live in SHARED Mercurial-style
// dynamic vaults (vault program 24Uqj9...), one vault per TOKEN, shared by
// every pool holding that token. The vault's token account balance is the
// total across all pools plus the lending buffer — treating it as this
// pool's reserve inflated quotes by 100-400x on live SOL/USDC pools
// (verified: pool 5yuefgbJJpmFNK2iiYbLSpv1aZXq7F9AUKkZKErTYCvs actually
// holds 39,902 USDC / 526 SOL of the vault totals of 8.1M USDC / 218k SOL).
//
// Real reserve = pool's vault-LP share of the vault's UNLOCKED amount:
//   unlocked(t)      = total_amount - locked_profit(t)
//   locked_profit(t) = last_updated_locked_profit
//                      * (1e12 - (t - last_report) * degradation) / 1e12
//                      (0 once the ratio reaches 1e12)
//   reserve          = pool_vault_lp.amount * unlocked / vault_lp_mint.supply
//
// Vault account offsets (verified against mainnet dumps, July 2026; the
// account is 10240 bytes): disc 8, enabled u8 @8, bumps @9..11,
// total_amount u64 @11, token_vault @19, fee_vault @51, token_mint @83,
// lp_mint @115, strategies [Pubkey;30] @147, base/admin/operator @1107,
// locked_profit_tracker @1203 (last_updated_locked_profit u64, last_report
// u64, locked_profit_degradation u64).
//
// A constant-product quote over share-math reserves with the pool's trade
// fee matched Jupiter's live V1 quote to 0.0001 bps.

const V1_VAULT_MIN_LEN: usize = 1227;
const V1_LOCKED_PROFIT_DEGRADATION_DENOMINATOR: u128 = 1_000_000_000_000;

/// Unlocked vault amount at `now` (unix secs) from raw vault account bytes.
fn v1_vault_unlocked_amount(vault_data: &[u8], now: u64) -> Option<u64> {
    if vault_data.len() < V1_VAULT_MIN_LEN {
        return None;
    }
    let total_amount = u64::from_le_bytes(vault_data[11..19].try_into().unwrap());
    let last_updated_locked_profit =
        u64::from_le_bytes(vault_data[1203..1211].try_into().unwrap());
    let last_report = u64::from_le_bytes(vault_data[1211..1219].try_into().unwrap());
    let degradation = u64::from_le_bytes(vault_data[1219..1227].try_into().unwrap());

    let duration = now.saturating_sub(last_report) as u128;
    let ratio = duration.saturating_mul(degradation as u128);
    let locked = if ratio >= V1_LOCKED_PROFIT_DEGRADATION_DENOMINATOR {
        0u64
    } else {
        (last_updated_locked_profit as u128
            * (V1_LOCKED_PROFIT_DEGRADATION_DENOMINATOR - ratio)
            / V1_LOCKED_PROFIT_DEGRADATION_DENOMINATOR) as u64
    };
    total_amount.checked_sub(locked)
}

/// The vault's lp_mint pubkey from raw vault account bytes (offset 115).
pub fn v1_vault_lp_mint(vault_data: &[u8]) -> Option<solana_pubkey::Pubkey> {
    if vault_data.len() < 147 {
        return None;
    }
    solana_pubkey::Pubkey::try_from(&vault_data[115..147]).ok()
}

// =============================================================================
// V1: MeteoraDAMMMarket
// =============================================================================

/// Market wrapper for Meteora Dynamic AMM V1 pools.
pub struct MeteoraDAMMMarket {
    pub pool: MeteoraDAMMPool,
    pub pool_address: String,
    /// Cached vault balances (updated externally after fetching on-chain data).
    pub a_vault_balance: u64,
    pub b_vault_balance: u64,
    pub token_a_decimals: u8,
    pub token_b_decimals: u8,
    /// True when the pool's token_a is actually the quote currency (WSOL/USDC),
    /// meaning the default quote=token_b / base=token_a mapping is inverted.
    pub flipped: bool,
}

impl MeteoraDAMMMarket {
    pub fn new(pool: MeteoraDAMMPool, pool_address: String) -> Self {
        let flipped = quote_priority(&pool.token_a_mint).unwrap_or(usize::MAX) < quote_priority(&pool.token_b_mint).unwrap_or(usize::MAX);
        let token_a_decimals = infer_mint_decimals(&pool.token_a_mint);
        let token_b_decimals = infer_mint_decimals(&pool.token_b_mint);
        Self {
            pool,
            pool_address,
            a_vault_balance: 0,
            b_vault_balance: 0,
            token_a_decimals,
            token_b_decimals,
            flipped,
        }
    }

    /// This pool's real reserve for one side: its vault-LP share of the
    /// vault's unlocked amount. All three accounts (vault state, vault LP
    /// mint, pool's vault-LP token account) must be in the provider.
    fn v1_reserve_from_provider(
        &self,
        vault: &solana_pubkey::Pubkey,
        pool_vault_lp: &solana_pubkey::Pubkey,
        provider: &dyn thunder_core::AccountDataProvider,
        now: u64,
    ) -> Result<u64, GenericError> {
        let vault_data = provider
            .pool_account_data(vault)
            .ok_or("V1 vault state account not in provider — pool unquotable")?;
        let unlocked = v1_vault_unlocked_amount(&vault_data, now)
            .ok_or("V1 vault account malformed")?;
        let lp_mint = v1_vault_lp_mint(&vault_data).ok_or("V1 vault lp_mint unreadable")?;
        let mint_data = provider
            .pool_account_data(&lp_mint)
            .ok_or("V1 vault lp mint not in provider — pool unquotable")?;
        if mint_data.len() < 44 {
            return Err("V1 vault lp mint account too short".into());
        }
        // SPL Mint: supply u64 at offset 36.
        let supply = u64::from_le_bytes(mint_data[36..44].try_into().unwrap());
        if supply == 0 {
            return Err("V1 vault lp mint has zero supply".into());
        }
        let lp_data = provider
            .pool_account_data(pool_vault_lp)
            .ok_or("V1 pool vault-LP token account not in provider — pool unquotable")?;
        if lp_data.len() < 72 {
            return Err("V1 pool vault-LP token account too short".into());
        }
        // SPL TokenAccount: amount u64 at offset 64.
        let share = u64::from_le_bytes(lp_data[64..72].try_into().unwrap());
        Ok((share as u128 * unlocked as u128 / supply as u128) as u64)
    }

    /// Constant-product output over the pool's REAL (share-math) reserves,
    /// with the trade fee applied on input at full numerator precision.
    /// Stable-curve pools are not modeled and return Err (router skips
    /// them) — documented behavior.
    fn v1_swap_with_reserves(
        &self,
        amount_in: u64,
        direction: SwapDirection,
        reserve_a: u64,
        reserve_b: u64,
    ) -> Result<u64, GenericError> {
        if matches!(self.pool.curve_type, CurveType::Stable { .. }) {
            return Err(
                "Meteora DAMM V1 stable curve not modeled — pool skipped for quoting".into(),
            );
        }
        if reserve_a == 0 || reserve_b == 0 {
            return Err("V1 pool has zero share-math reserves".into());
        }
        let physical_direction = if self.flipped {
            match direction {
                SwapDirection::Buy => SwapDirection::Sell,
                SwapDirection::Sell => SwapDirection::Buy,
            }
        } else {
            direction
        };
        let fees = &self.pool.fees;
        if fees.trade_fee_denominator == 0 {
            return Err("V1 pool has zero fee denominator".into());
        }
        let fee = (amount_in as u128 * fees.trade_fee_numerator as u128)
            / fees.trade_fee_denominator as u128;
        let net = (amount_in as u128).saturating_sub(fee);
        // Physical Buy = token_b in -> token_a out; Sell = a in -> b out.
        let (reserve_in, reserve_out) = match physical_direction {
            SwapDirection::Buy => (reserve_b as u128, reserve_a as u128),
            SwapDirection::Sell => (reserve_a as u128, reserve_b as u128),
        };
        let out = reserve_out * net / (reserve_in + net);
        Ok(out as u64)
    }
}

impl Market for MeteoraDAMMMarket {
    fn is_active(&self) -> bool {
        self.pool.enabled
    }

    fn metadata(&self) -> Result<PoolMetadata, GenericError> {
        let fee_bps = (self.pool.fees.trade_fee_numerator as f64
            / self.pool.fees.trade_fee_denominator as f64
            * 10000.0) as u64;

        let protocol_fee_bps = (self.pool.fees.protocol_trade_fee_numerator as f64
            / self.pool.fees.protocol_trade_fee_denominator as f64
            * 10000.0) as u64;

        Ok(PoolMetadata {
            address: self.pool_address.clone(),
            dex_name: match self.pool.curve_type {
                CurveType::ConstantProduct => "Meteora DAMM (Constant Product)".to_string(),
                CurveType::Stable { .. } => "Meteora DAMM (Stable)".to_string(),
            },
            quote_mint: if self.flipped { self.pool.token_a_mint } else { self.pool.token_b_mint },
            base_mint: if self.flipped { self.pool.token_b_mint } else { self.pool.token_a_mint },
            quote_vault: if self.flipped { derive_token_vault_address(self.pool.a_vault).0 } else { derive_token_vault_address(self.pool.b_vault).0 },
            base_vault: if self.flipped { derive_token_vault_address(self.pool.b_vault).0 } else { derive_token_vault_address(self.pool.a_vault).0 },
            fees: PoolFees {
                trade_fee_bps: fee_bps,
                protocol_fee_bps: Some(protocol_fee_bps),
            },
        })
    }

    fn financials(&self) -> Result<PoolFinancials, GenericError> {
        Ok(PoolFinancials {
            quote_balance: if self.flipped { self.a_vault_balance } else { self.b_vault_balance },
            base_balance: if self.flipped { self.b_vault_balance } else { self.a_vault_balance },
            quote_decimals: if self.flipped { self.token_a_decimals } else { self.token_b_decimals },
            base_decimals: if self.flipped { self.token_b_decimals } else { self.token_a_decimals },
        })
    }

    /// See `calculate_output_live` — cached "vault balances" are shared
    /// across pools and are not this pool's reserves. Err instead of a
    /// silently inflated quote.
    fn calculate_output(
        &self,
        _amount_in: u64,
        _direction: SwapDirection,
    ) -> Result<u64, GenericError> {
        Err(
            "Meteora DAMM V1 reserves live in shared vaults — quoting requires \
             vault accounts via calculate_output_live_ex"
                .into(),
        )
    }

    /// V1 pools cannot be quoted from vault token-account balances: those
    /// vaults are SHARED across all pools of the same token (see the
    /// vault-share note above). Without the vault state + LP accounts this
    /// pool's reserves are unknown, so this errs instead of returning a
    /// silently wrong constant-product quote (the old behavior inflated a
    /// 1 SOL -> USDC quote to ~692k USDC through a thin V1 pool). Use
    /// `calculate_output_live_ex` with an account provider.
    fn calculate_output_live(
        &self,
        _amount_in: u64,
        _direction: SwapDirection,
        _pool_data: Option<&[u8]>,
        _quote_vault_balance: u64,
        _base_vault_balance: u64,
    ) -> Result<u64, GenericError> {
        Err(
            "Meteora DAMM V1 reserves live in shared vaults — quoting requires \
             vault accounts via calculate_output_live_ex"
                .into(),
        )
    }

    /// Real V1 quote: share-math reserves from the vault state, vault LP
    /// mint and the pool's vault-LP token accounts (all via the provider),
    /// then constant product with the pool's trade fee. Missing accounts or
    /// stable-curve pools -> Err (router skips the pool).
    fn calculate_output_live_ex(
        &self,
        amount_in: u64,
        direction: SwapDirection,
        _pool_data: Option<&[u8]>,
        _quote_vault_balance: u64,
        _base_vault_balance: u64,
        accounts: Option<&dyn thunder_core::AccountDataProvider>,
    ) -> Result<u64, GenericError> {
        let provider = accounts.ok_or(
            "Meteora DAMM V1 requires an account provider for vault-share reserves",
        )?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let reserve_a = self.v1_reserve_from_provider(
            &self.pool.a_vault,
            &self.pool.a_vault_lp,
            provider,
            now,
        )?;
        let reserve_b = self.v1_reserve_from_provider(
            &self.pool.b_vault,
            &self.pool.b_vault_lp,
            provider,
            now,
        )?;
        self.v1_swap_with_reserves(amount_in, direction, reserve_a, reserve_b)
    }

    fn calculate_price_impact(
        &self,
        amount_in: u64,
        direction: SwapDirection,
    ) -> Result<u64, GenericError> {
        let pre_swap_price = self.current_price()?;
        let output = self.calculate_output(amount_in, direction)?;

        // Price impact uses physical direction (already normalized in calculate_output).
        let physical_direction = if self.flipped {
            match direction {
                SwapDirection::Buy => SwapDirection::Sell,
                SwapDirection::Sell => SwapDirection::Buy,
            }
        } else {
            direction
        };

        let post_swap_price = match physical_direction {
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
        let (quote_bal, base_bal, quote_dec, base_dec) = if self.flipped {
            (self.a_vault_balance, self.b_vault_balance, self.token_a_decimals, self.token_b_decimals)
        } else {
            (self.b_vault_balance, self.a_vault_balance, self.token_b_decimals, self.token_a_decimals)
        };
        if base_bal == 0 {
            return Err("Pool has zero base balance".into());
        }
        let quote_human = quote_bal as f64 / 10f64.powi(quote_dec as i32);
        let base_human = base_bal as f64 / 10f64.powi(base_dec as i32);
        Ok(quote_human / base_human)
    }
}


// =============================================================================
// V2: MeteoraDAMMV2Market
// =============================================================================

/// Market wrapper for Meteora Dynamic AMM V2 pools.
pub struct MeteoraDAMMV2Market {
    pub pool: MeteoraDAMMV2Pool,
    pub pool_address: String,
    pub a_vault_balance: u64,
    pub b_vault_balance: u64,
    /// Decimals for token_a and token_b. Needed for sqrt_price conversion.
    /// Defaults to (6, 9) if not provided (assumes token_a=token, token_b=SOL).
    pub token_a_decimals: u8,
    pub token_b_decimals: u8,
    /// True when the pool's token_a is actually the quote currency (WSOL/USDC).
    pub flipped: bool,
}

impl MeteoraDAMMV2Market {
    pub fn new(pool: MeteoraDAMMV2Pool, pool_address: String) -> Self {
        let flipped = quote_priority(&pool.token_a_mint).unwrap_or(usize::MAX) < quote_priority(&pool.token_b_mint).unwrap_or(usize::MAX);
        let da = infer_mint_decimals(&pool.token_a_mint);
        let db = infer_mint_decimals(&pool.token_b_mint);
        Self {
            pool,
            pool_address,
            a_vault_balance: 0,
            b_vault_balance: 0,
            token_a_decimals: da,
            token_b_decimals: db,
            flipped,
        }
    }


    /// Human-readable price (token_b per token_a) from a Q64.64 sqrt price.
    ///
    /// `(sqrt_price / 2^64)^2` IS the raw token_b-per-token_a amount ratio
    /// (raw units). Decimal adjustment happens exactly ONCE:
    /// human_b_per_a = raw * 10^(dec_a - dec_b).
    fn sqrt_price_to_human_b_per_a(&self, sqrt_price: u128) -> f64 {
        let sqrt_price_f64 = sqrt_price as f64 / (1u128 << 64) as f64;
        let raw = sqrt_price_f64 * sqrt_price_f64;
        raw * 10f64.powi(self.token_a_decimals as i32 - self.token_b_decimals as i32)
    }

    /// Base fee in basis points (display only — the swap math uses the full
    /// 1e-9 numerator). cliff_fee_numerator is in parts-per-billion.
    fn calculate_base_fee_bps(&self) -> u64 {
        self.pool.pool_fees.base_fee.cliff_fee_numerator / 100_000
    }

    /// Current total trading fee numerator (1e-9 units): scheduled base fee
    /// plus dynamic fee when initialized.
    ///
    /// `volatility_accumulator`: live value if parsed from fresh pool bytes,
    /// else the cached one.
    fn v2_total_fee_numerator(&self, volatility_accumulator: u128) -> u64 {
        let bf = &self.pool.pool_fees.base_fee;
        let base = if bf.number_of_period == 0 || bf.period_frequency == 0 {
            bf.cliff_fee_numerator
        } else {
            // Fee scheduler. Elapsed periods since activation:
            // activation_type 1 = timestamp (computable), 0 = slot (current
            // slot unknown off-chain -> assume the schedule has completed;
            // only short-lived launch pools are mid-schedule).
            let periods = if self.pool.activation_type == 1 {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(u64::MAX);
                now.saturating_sub(self.pool.activation_point)
                    .checked_div(bf.period_frequency)
                    .unwrap_or(u64::MAX)
                    .min(bf.number_of_period as u64)
            } else {
                bf.number_of_period as u64
            };
            match bf.fee_scheduler_mode {
                // Linear: cliff - periods * reduction_factor
                0 => bf
                    .cliff_fee_numerator
                    .saturating_sub(periods.saturating_mul(bf.reduction_factor)),
                // Exponential: cliff * (1 - reduction/10000)^periods
                1 => {
                    let r = 1.0 - bf.reduction_factor as f64 / 10_000.0;
                    (bf.cliff_fee_numerator as f64 * r.powi(periods.min(10_000) as i32)) as u64
                }
                // Rate limiter / market-cap scheduler: not modeled, use cliff.
                _ => bf.cliff_fee_numerator,
            }
        };

        let dynamic = {
            let df = &self.pool.pool_fees.dynamic_fee;
            if df.initialized != 0 && df.variable_fee_control > 0 {
                let vfa_bin = volatility_accumulator.saturating_mul(df.bin_step as u128);
                let square = vfa_bin.saturating_mul(vfa_bin);
                let v_fee = (df.variable_fee_control as u128).saturating_mul(square);
                ((v_fee + 99_999_999_999) / 100_000_000_000) as u64
            } else {
                0
            }
        };

        // MAX_FEE_NUMERATOR (50%) cap, mirroring cp-amm.
        (base.saturating_add(dynamic)).min(500_000_000)
    }

    /// Exact V2 output from (liquidity, sqrt_price) using the cp-amm
    /// in-range formulas (see the fixed-point scaling note at the top of
    /// this file — cp-amm liquidity carries 64 extra fractional bits).
    ///
    /// Fee placement mirrors cp-amm FeeMode: fee on input only for
    /// b_to_a with collect_fee_mode OnlyB(1)/Compounding(2); otherwise on
    /// output. Price-range bounds enforced except for Compounding pools
    /// (pure constant product, no range).
    #[allow(clippy::too_many_arguments)]
    fn v2_swap_exact_in(
        &self,
        amount_in: u64,
        a_to_b: bool,
        liquidity: u128,
        sqrt_price: u128,
        volatility_accumulator: u128,
    ) -> Result<u64, GenericError> {
        if liquidity == 0 {
            return Err("Pool has zero liquidity".into());
        }
        if sqrt_price == 0 {
            return Err("Pool has zero price".into());
        }
        let fee_num = self.v2_total_fee_numerator(volatility_accumulator) as u128;
        const FEE_DENOM: u128 = 1_000_000_000;

        let is_compounding = self.pool.collect_fee_mode == 2;
        let fee_on_input = !a_to_b && matches!(self.pool.collect_fee_mode, 1 | 2);

        let net_in: u128 = if fee_on_input {
            let fee = mul_div_ceil(amount_in as u128, fee_num, FEE_DENOM).ok_or("fee overflow")?;
            (amount_in as u128).saturating_sub(fee)
        } else {
            amount_in as u128
        };

        let gross_out: u128 = if a_to_b {
            let sqrt_new = v2_next_sqrt_price_from_a_in(sqrt_price, liquidity, net_in)
                .ok_or("sqrt price math overflow")?;
            if !is_compounding && self.pool.sqrt_min_price > 0 && sqrt_new < self.pool.sqrt_min_price
            {
                return Err("swap exceeds pool min price — insufficient range liquidity".into());
            }
            v2_delta_b(sqrt_new, sqrt_price, liquidity).ok_or("delta overflow")?
        } else {
            let sqrt_new = v2_next_sqrt_price_from_b_in(sqrt_price, liquidity, net_in)
                .ok_or("sqrt price math overflow")?;
            if !is_compounding
                && self.pool.sqrt_max_price > 0
                && sqrt_new > self.pool.sqrt_max_price
            {
                return Err("swap exceeds pool max price — insufficient range liquidity".into());
            }
            v2_delta_a(sqrt_price, sqrt_new, liquidity).ok_or("delta overflow")?
        };

        let out = if fee_on_input {
            gross_out
        } else {
            let fee = mul_div_ceil(gross_out, fee_num, FEE_DENOM).ok_or("fee overflow")?;
            gross_out.saturating_sub(fee)
        };
        u64::try_from(out).map_err(|_| "output exceeds u64".into())
    }

    /// V2 output using current (cached or live) pool state.
    fn calculate_v2_output(
        &self,
        amount_in: u64,
        direction: SwapDirection,
    ) -> Result<u64, GenericError> {
        // Normalize direction when flipped.
        let physical_direction = if self.flipped {
            match direction {
                SwapDirection::Buy => SwapDirection::Sell,
                SwapDirection::Sell => SwapDirection::Buy,
            }
        } else {
            direction
        };
        // Physical Buy = quote(b) -> base(a) = b_to_a; Sell = a_to_b.
        let a_to_b = physical_direction == SwapDirection::Sell;
        self.v2_swap_exact_in(
            amount_in,
            a_to_b,
            self.pool.liquidity,
            self.pool.sqrt_price,
            self.pool.pool_fees.dynamic_fee.volatility_accumulator,
        )
    }
}

impl Market for MeteoraDAMMV2Market {
    fn is_active(&self) -> bool {
        self.pool.pool_status == 0
    }

    fn metadata(&self) -> Result<PoolMetadata, GenericError> {
        let trade_fee_bps = self.calculate_base_fee_bps();
        let protocol_fee_bps =
            (self.pool.pool_fees.protocol_fee_percent as u64 * trade_fee_bps) / 100;

        Ok(PoolMetadata {
            address: self.pool_address.clone(),
            dex_name: "Meteora DAMM V2".to_string(),
            quote_mint: if self.flipped { self.pool.token_a_mint } else { self.pool.token_b_mint },
            base_mint: if self.flipped { self.pool.token_b_mint } else { self.pool.token_a_mint },
            quote_vault: if self.flipped { self.pool.token_a_vault } else { self.pool.token_b_vault },
            base_vault: if self.flipped { self.pool.token_b_vault } else { self.pool.token_a_vault },
            fees: PoolFees {
                trade_fee_bps,
                protocol_fee_bps: Some(protocol_fee_bps),
            },
        })
    }

    fn financials(&self) -> Result<PoolFinancials, GenericError> {
        Ok(PoolFinancials {
            quote_balance: if self.flipped { self.a_vault_balance } else { self.b_vault_balance },
            base_balance: if self.flipped { self.b_vault_balance } else { self.a_vault_balance },
            quote_decimals: if self.flipped { self.token_a_decimals } else { self.token_b_decimals },
            base_decimals: if self.flipped { self.token_b_decimals } else { self.token_a_decimals },
        })
    }

    fn calculate_output(
        &self,
        amount_in: u64,
        direction: SwapDirection,
    ) -> Result<u64, GenericError> {
        self.calculate_v2_output(amount_in, direction)
    }

    /// With a provider present but no live pool bytes, the pool's current
    /// sqrt_price/liquidity are unverifiable (cached snapshots can be
    /// months stale) -> Err so the router skips the pool until its account
    /// streams in. Without a provider (CLI path) this delegates to the
    /// cached-state math.
    fn calculate_output_live_ex(
        &self,
        amount_in: u64,
        direction: SwapDirection,
        pool_data: Option<&[u8]>,
        quote_vault_balance: u64,
        base_vault_balance: u64,
        accounts: Option<&dyn thunder_core::AccountDataProvider>,
    ) -> Result<u64, GenericError> {
        if accounts.is_some() && pool_data.map_or(true, |d| d.len() < 472) {
            return Err(
                "DAMM V2 pool account bytes not in provider — pool unquotable until streamed"
                    .into(),
            );
        }
        self.calculate_output_live(
            amount_in,
            direction,
            pool_data,
            quote_vault_balance,
            base_vault_balance,
        )
    }

    fn calculate_output_live(
        &self,
        amount_in: u64,
        direction: SwapDirection,
        pool_data: Option<&[u8]>,
        quote_vault_balance: u64,
        base_vault_balance: u64,
    ) -> Result<u64, GenericError> {
        // Reconstruct physical vault balances from normalized inputs
        // (used only as a safety cap on the computed output).
        let (a_vault_balance, b_vault_balance) = if self.flipped {
            (quote_vault_balance, base_vault_balance)
        } else {
            (base_vault_balance, quote_vault_balance)
        };

        // Parse live swap-volatile fields, falling back to cached values.
        // Verified offsets: liquidity @360..376, sqrt_price @456..472,
        // dynamic_fee.volatility_accumulator @120..136.
        let (sqrt_price, liquidity, volatility_accumulator) = match pool_data {
            Some(data) if data.len() >= 472 => (
                u128::from_le_bytes(data[456..472].try_into().unwrap()),
                u128::from_le_bytes(data[360..376].try_into().unwrap()),
                u128::from_le_bytes(data[120..136].try_into().unwrap()),
            ),
            _ => (
                self.pool.sqrt_price,
                self.pool.liquidity,
                self.pool.pool_fees.dynamic_fee.volatility_accumulator,
            ),
        };

        let physical_direction = if self.flipped {
            match direction {
                SwapDirection::Buy => SwapDirection::Sell,
                SwapDirection::Sell => SwapDirection::Buy,
            }
        } else {
            direction
        };
        // Physical Buy = b_to_a; Sell = a_to_b.
        let a_to_b = physical_direction == SwapDirection::Sell;

        let out =
            self.v2_swap_exact_in(amount_in, a_to_b, liquidity, sqrt_price, volatility_accumulator)?;

        // Safety cap at the destination vault balance when known.
        let cap = if a_to_b { b_vault_balance } else { a_vault_balance };
        Ok(if cap > 0 { out.min(cap) } else { out })
    }

    fn calculate_price_impact(
        &self,
        amount_in: u64,
        direction: SwapDirection,
    ) -> Result<u64, GenericError> {
        let pre_swap_price = self.current_price()?;
        let output = self.calculate_output(amount_in, direction)?;

        let physical_direction = if self.flipped {
            match direction {
                SwapDirection::Buy => SwapDirection::Sell,
                SwapDirection::Sell => SwapDirection::Buy,
            }
        } else {
            direction
        };

        let post_swap_price = match physical_direction {
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
        // Always derive the price from sqrt_price. Reserve ratios are NOT a
        // price for concentrated positions (the old reserve-based path gave
        // ~5.2 USDC/SOL when ground truth was ~76).
        //
        // human_b_per_a = (sqrt_price/2^64)^2 * 10^(dec_a - dec_b): the
        // decimal adjustment is applied exactly once on the raw ratio.
        let human_b_per_a = self.sqrt_price_to_human_b_per_a(self.pool.sqrt_price);
        if human_b_per_a == 0.0 || !human_b_per_a.is_finite() {
            return Err("Pool has zero price".into());
        }
        // NOT flipped (quote=b, base=a): quote-per-base = b per a as-is.
        // Flipped (quote=a, base=b): quote-per-base = a per b = 1/(b per a).
        if self.flipped {
            Ok(1.0 / human_b_per_a)
        } else {
            Ok(human_b_per_a)
        }
    }
}
// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use solana_pubkey::Pubkey;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use thunder_core::{AccountDataProvider, Market, SwapDirection, WSOL, USDC};

    #[derive(Default)]
    struct MapProvider(Mutex<HashMap<Pubkey, Vec<u8>>>);

    impl MapProvider {
        fn insert(&self, k: Pubkey, v: Vec<u8>) {
            self.0.lock().unwrap().insert(k, v);
        }
    }

    impl AccountDataProvider for MapProvider {
        fn pool_account_data(&self, pubkey: &Pubkey) -> Option<Vec<u8>> {
            self.0.lock().unwrap().get(pubkey).cloned()
        }
        fn token_balance(&self, _vault: &Pubkey) -> u64 {
            0
        }
    }

    // ---- V1 helpers ----

    fn make_v1_pool(
        token_a_mint: Pubkey,
        token_b_mint: Pubkey,
        curve_type: CurveType,
    ) -> MeteoraDAMMPool {
        MeteoraDAMMPool {
            lp_mint: Pubkey::new_unique(),
            token_a_mint,
            token_b_mint,
            a_vault: Pubkey::new_unique(),
            b_vault: Pubkey::new_unique(),
            a_vault_lp: Pubkey::new_unique(),
            b_vault_lp: Pubkey::new_unique(),
            a_vault_lp_bump: 0,
            enabled: true,
            protocol_token_a_fee: Pubkey::default(),
            protocol_token_b_fee: Pubkey::default(),
            fee_last_updated_at: 0,
            padding0: [0; 24],
            fees: models::PoolFees {
                trade_fee_numerator: 25,
                trade_fee_denominator: 10_000,
                protocol_trade_fee_numerator: 20_000,
                protocol_trade_fee_denominator: 100_000,
            },
            pool_type: models::PoolType::Permissionless,
            stake: Pubkey::default(),
            total_locked_lp: 0,
            bootstrapping: models::Bootstrapping {
                activation_point: 0,
                whitelisted_vault: Pubkey::default(),
                pool_creator: Pubkey::default(),
                activation_type: 0,
            },
            partner_info: models::PartnerInfo {
                fee_numerator: 0,
                partner_authority: Pubkey::default(),
                pending_fee_a: 0,
                pending_fee_b: 0,
            },
            padding: models::Padding {
                padding0: [0; 6],
                padding1: [0; 21],
                padding2: [0; 21],
            },
            curve_type,
        }
    }

    /// Synthetic vault state account: total_amount @11, lp_mint @115,
    /// zeroed locked-profit tracker @1203 (fully unlocked).
    fn make_vault_account(total_amount: u64, lp_mint: Pubkey) -> Vec<u8> {
        let mut d = vec![0u8; 1240];
        d[11..19].copy_from_slice(&total_amount.to_le_bytes());
        d[115..147].copy_from_slice(lp_mint.as_ref());
        d
    }

    fn make_mint_account(supply: u64) -> Vec<u8> {
        let mut d = vec![0u8; 82];
        d[36..44].copy_from_slice(&supply.to_le_bytes());
        d
    }

    fn make_token_account(amount: u64) -> Vec<u8> {
        let mut d = vec![0u8; 165];
        d[64..72].copy_from_slice(&amount.to_le_bytes());
        d
    }

    /// Pinned against the verified mainnet state of V1 pool
    /// 5yuefgbJJpmFNK2iiYbLSpv1aZXq7F9AUKkZKErTYCvs (USDC/SOL, 2026-07-13):
    /// USDC vault total 9517645965628, lp supply 8184101686896, pool share
    /// 34311489576; SOL vault total 258082376415167, lp supply
    /// 229323810346216, pool share 467270279986. Share-math reserves are
    /// ~39,902 USDC / ~526 SOL — NOT the shared vault token balances of
    /// 8.1M USDC / 218k SOL (a 203x/416x inflation, which produced the
    /// 1 SOL -> 692,677 USDC repro). CP over these reserves matched
    /// Jupiter to 0.0001 bps.
    #[test]
    fn test_v1_share_math_reserves_pinned() {
        let usdc = Pubkey::from_str_const(USDC);
        let wsol = Pubkey::from_str_const(WSOL);
        let pool = make_v1_pool(usdc, wsol, CurveType::ConstantProduct);
        let a_vault = pool.a_vault;
        let b_vault = pool.b_vault;
        let a_vault_lp = pool.a_vault_lp;
        let b_vault_lp = pool.b_vault_lp;
        let market = MeteoraDAMMMarket::new(pool, Pubkey::new_unique().to_string());
        assert!(!market.flipped, "quote should be WSOL (token_b)");

        let a_lp_mint = Pubkey::new_unique();
        let b_lp_mint = Pubkey::new_unique();
        let provider = MapProvider::default();
        provider.insert(a_vault, make_vault_account(9_517_645_965_628, a_lp_mint));
        provider.insert(a_lp_mint, make_mint_account(8_184_101_686_896));
        provider.insert(a_vault_lp, make_token_account(34_311_489_576));
        provider.insert(b_vault, make_vault_account(258_082_376_415_167, b_lp_mint));
        provider.insert(b_lp_mint, make_mint_account(229_323_810_346_216));
        provider.insert(b_vault_lp, make_token_account(467_270_279_986));

        // 1 SOL -> USDC (market Buy: quote SOL -> base USDC).
        let out = market
            .calculate_output_live_ex(
                1_000_000_000,
                SwapDirection::Buy,
                None,
                0,
                0,
                Some(&provider),
            )
            .unwrap();
        assert_eq!(out, 75_545_853); // ~75.5 USDC, not ~692k

        // 1000 USDC -> SOL (market Sell).
        let out2 = market
            .calculate_output_live_ex(
                1_000_000_000,
                SwapDirection::Sell,
                None,
                0,
                0,
                Some(&provider),
            )
            .unwrap();
        assert_eq!(out2, 12_825_340_705); // ~12.83 SOL

        // Missing vault account -> Err (router skips pool).
        let empty = MapProvider::default();
        assert!(market
            .calculate_output_live_ex(1_000_000_000, SwapDirection::Buy, None, 0, 0, Some(&empty))
            .is_err());

        // No provider / legacy paths -> Err (no silent shared-vault quotes).
        assert!(market
            .calculate_output_live_ex(1_000_000_000, SwapDirection::Buy, None, 0, 0, None)
            .is_err());
        assert!(market.calculate_output(1_000_000_000, SwapDirection::Buy).is_err());
        assert!(market
            .calculate_output_live(1_000_000_000, SwapDirection::Buy, None, 1, 1)
            .is_err());
    }

    /// Stable-curve V1 pools are not modeled -> Err, documented.
    #[test]
    fn test_v1_stable_curve_errs() {
        let usdc = Pubkey::from_str_const(USDC);
        let usdt = Pubkey::from_str_const(thunder_core::USDT);
        let pool = make_v1_pool(
            usdc,
            usdt,
            CurveType::Stable {
                amp: 100,
                token_multiplier: models::TokenMultiplier {
                    token_a_multiplier: 1,
                    token_b_multiplier: 1,
                    precision_factor: 1,
                },
                depeg: models::Depeg {
                    base_virtual_price: 0,
                    base_cache_updated: 0,
                    depeg_type: models::DepegType::None,
                },
                last_amp_updated_timestamp: 0,
            },
        );
        let a_vault = pool.a_vault;
        let b_vault = pool.b_vault;
        let a_vault_lp = pool.a_vault_lp;
        let b_vault_lp = pool.b_vault_lp;
        let market = MeteoraDAMMMarket::new(pool, Pubkey::new_unique().to_string());

        let a_lp_mint = Pubkey::new_unique();
        let b_lp_mint = Pubkey::new_unique();
        let provider = MapProvider::default();
        provider.insert(a_vault, make_vault_account(1_000_000, a_lp_mint));
        provider.insert(a_lp_mint, make_mint_account(1_000_000));
        provider.insert(a_vault_lp, make_token_account(1_000_000));
        provider.insert(b_vault, make_vault_account(1_000_000, b_lp_mint));
        provider.insert(b_lp_mint, make_mint_account(1_000_000));
        provider.insert(b_vault_lp, make_token_account(1_000_000));

        let res = market.calculate_output_live_ex(
            1_000,
            SwapDirection::Buy,
            None,
            0,
            0,
            Some(&provider),
        );
        assert!(res.is_err());
    }

    /// Locked profit decays linearly with degradation until fully unlocked.
    #[test]
    fn test_v1_locked_profit_unlock() {
        let lp_mint = Pubkey::new_unique();
        let mut vault = make_vault_account(1_000_000, lp_mint);
        // locked tracker: last_updated_locked_profit=100_000, last_report=1000,
        // degradation such that full unlock takes 100s (1e12/100 per sec).
        vault[1203..1211].copy_from_slice(&100_000u64.to_le_bytes());
        vault[1211..1219].copy_from_slice(&1000u64.to_le_bytes());
        vault[1219..1227].copy_from_slice(&10_000_000_000u64.to_le_bytes());

        // t = last_report: fully locked.
        assert_eq!(v1_vault_unlocked_amount(&vault, 1000), Some(900_000));
        // t = +50s: half unlocked.
        assert_eq!(v1_vault_unlocked_amount(&vault, 1050), Some(950_000));
        // t = +100s and beyond: fully unlocked.
        assert_eq!(v1_vault_unlocked_amount(&vault, 1100), Some(1_000_000));
        assert_eq!(v1_vault_unlocked_amount(&vault, 99_999), Some(1_000_000));
    }

    fn make_v2_pool(
        liquidity: u128,
        sqrt_price: u128,
        cliff_fee_numerator: u64,
        collect_fee_mode: u8,
    ) -> MeteoraDAMMV2Pool {
        MeteoraDAMMV2Pool {
            pool_fees: V2PoolFees {
                base_fee: BaseFee {
                    cliff_fee_numerator,
                    fee_scheduler_mode: 0,
                    padding_0: [0; 5],
                    number_of_period: 0,
                    period_frequency: 0,
                    reduction_factor: 0,
                    padding_1: 0,
                },
                protocol_fee_percent: 20,
                partner_fee_percent: 0,
                referral_fee_percent: 20,
                padding_0: [0; 5],
                dynamic_fee: DynamicFee {
                    initialized: 0,
                    padding: [0; 7],
                    max_volatility_accumulator: 0,
                    variable_fee_control: 0,
                    bin_step: 0,
                    filter_period: 0,
                    decay_period: 0,
                    reduction_factor: 0,
                    last_update_timestamp: 0,
                    bin_step_u128: 0,
                    sqrt_price_reference: 0,
                    volatility_accumulator: 0,
                    volatility_reference: 0,
                },
                padding_1: [0; 2],
            },
            token_a_mint: Pubkey::from_str_const(WSOL),
            token_b_mint: Pubkey::from_str_const(USDC),
            token_a_vault: Pubkey::new_unique(),
            token_b_vault: Pubkey::new_unique(),
            whitelisted_vault: Pubkey::default(),
            partner: Pubkey::default(),
            liquidity,
            _padding: 0,
            protocol_a_fee: 0,
            protocol_b_fee: 0,
            partner_a_fee: 0,
            partner_b_fee: 0,
            sqrt_min_price: 4295048016,
            sqrt_max_price: 79226673521066979257578248091,
            sqrt_price,
            activation_point: 0,
            activation_type: 1,
            pool_status: 0,
            token_a_flag: 0,
            token_b_flag: 0,
            collect_fee_mode,
            pool_type: 0,
            _padding_0: [0; 2],
            fee_a_per_liquidity: [0; 32],
            fee_b_per_liquidity: [0; 32],
            permanent_lock_liquidity: 0,
            metrics: PoolMetrics {
                total_lp_a_fee: 0,
                total_lp_b_fee: 0,
                total_protocol_a_fee: 0,
                total_protocol_b_fee: 0,
                total_partner_a_fee: 0,
                total_partner_b_fee: 0,
                total_position: 0,
                padding: 0,
            },
            creator: Pubkey::default(),
            token_a_amount: 0,
            token_b_amount: 0,
            layout_version: 1,
            _padding_3: [0; 7],
            _padding_4: [0; 3],
            reward_infos: std::array::from_fn(|_| RewardInfo {
                initialized: 0,
                reward_token_flag: 0,
                _padding_0: [0; 6],
                _padding_1: [0; 8],
                mint: Pubkey::default(),
                vault: Pubkey::default(),
                funder: Pubkey::default(),
                reward_duration: 0,
                reward_duration_end: 0,
                reward_rate: 0,
                reward_per_token_stored: [0; 32],
                last_update_time: 0,
                cumulative_seconds_with_empty_liquidity_reward: 0,
            }),
        }
    }

    /// Pinned regression from mainnet ground truth (pool
    /// 8Pm2kZpnxD3hoMmt4bjStX2Pw2Z9abpbHzZxMPqxPmie, slot 432642505):
    /// (sqrt_price/2^64)^2 = 0.075981 IS the raw USDC-per-SOL amount ratio.
    /// Executed swap after the 4bps fee gave ratio 0.075947. The old code
    /// used the decimal-adjusted human price (75.981) here, overestimating
    /// output by exactly 10^3 on SOL(9)/USDC(6).
    #[test]
    fn test_v2_raw_ratio_not_human_price_1000x_regression() {
        let raw_ratio: f64 = 0.075981;
        let sqrt_price = (raw_ratio.sqrt() * (1u128 << 64) as f64) as u128;
        // Live liquidity magnitude from the ground-truth pool.
        let liquidity: u128 = 127_882_556_264_287_465_142_493_523_274_359;
        let pool = make_v2_pool(liquidity, sqrt_price, 400_000, 0);
        let market = MeteoraDAMMV2Market::new(pool, Pubkey::new_unique().to_string());
        assert!(market.flipped, "WSOL is token_a -> flipped");

        // 1 SOL in (market Buy: quote SOL -> base USDC).
        let out = market.calculate_output(1_000_000_000, SwapDirection::Buy).unwrap();
        // Expected: 1e9 * 0.075981 * (1 - 0.0004) ~= 75_950_608 raw USDC.
        let expected = (1e9 * raw_ratio * (1.0 - 0.0004)) as i64;
        assert!(
            (out as i64 - expected).abs() < 5_000,
            "out {out} vs expected {expected} (the 1000x bug would give ~7.6e10)"
        );
        // Explicit 1000x guard:
        assert!(out < 100_000_000, "output must be ~7.6e7 raw units, not ~7.6e10");

        // current_price: quote(SOL) per base(USDC) = 1/75.981.
        let price = market.current_price().unwrap();
        let human = 1.0 / price; // USDC per SOL
        assert!(
            (human - 75.981).abs() < 0.01,
            "human price ~75.98 USDC/SOL, got {human}"
        );
    }

    /// Exact-math concavity: double input -> strictly less than double output.
    #[test]
    fn test_v2_concave() {
        let sqrt_price = (0.075981f64.sqrt() * (1u128 << 64) as f64) as u128;
        // Small liquidity so impact is visible.
        let liquidity: u128 = 1u128 << 100;
        let pool = make_v2_pool(liquidity, sqrt_price, 400_000, 0);
        let market = MeteoraDAMMV2Market::new(pool, Pubkey::new_unique().to_string());

        let a: u64 = 10_000_000_000;
        let out1 = market.calculate_output(a, SwapDirection::Buy).unwrap();
        let out2 = market.calculate_output(a * 2, SwapDirection::Buy).unwrap();
        assert!(out2 > out1);
        assert!(out2 < out1 * 2, "concave: {out2} < 2*{out1}");
    }

    /// Exceeding the pool's price range must Err, not clamp silently.
    #[test]
    fn test_v2_range_violation_errors() {
        let sqrt_price = (0.075981f64.sqrt() * (1u128 << 64) as f64) as u128;
        let liquidity: u128 = 1u128 << 80;
        let mut pool = make_v2_pool(liquidity, sqrt_price, 400_000, 0);
        // Max price barely above current: a b_to_a (Sell: USDC -> SOL) swap
        // pushing the price up must violate the range.
        pool.sqrt_max_price = sqrt_price + (sqrt_price >> 10);
        let market = MeteoraDAMMV2Market::new(pool, Pubkey::new_unique().to_string());
        let res = market.calculate_output(u64::MAX / 4, SwapDirection::Sell);
        assert!(res.is_err(), "expected range violation, got {res:?}");
    }

    /// Live path parses sqrt_price/liquidity from raw bytes at the verified
    /// offsets (456..472, 360..376) and must agree with the cached path.
    #[test]
    fn test_v2_live_bytes_offsets() {
        let sqrt_price = (0.075981f64.sqrt() * (1u128 << 64) as f64) as u128;
        let liquidity: u128 = 127_882_556_264_287_465_142_493_523_274_359;
        let pool = make_v2_pool(liquidity, sqrt_price, 400_000, 0);
        let market = MeteoraDAMMV2Market::new(pool, Pubkey::new_unique().to_string());

        // Synthesize live account bytes with DIFFERENT sqrt_price at the
        // verified offsets; the live path must pick them up.
        let live_sqrt = (0.080f64.sqrt() * (1u128 << 64) as f64) as u128;
        let mut data = vec![0u8; 1112];
        data[456..472].copy_from_slice(&live_sqrt.to_le_bytes());
        data[360..376].copy_from_slice(&liquidity.to_le_bytes());

        let out_live = market
            .calculate_output_live(1_000_000_000, SwapDirection::Buy, Some(&data), 0, 0)
            .unwrap();
        let expected = (1e9 * 0.080 * (1.0 - 0.0004)) as i64;
        assert!(
            (out_live as i64 - expected).abs() < 5_000,
            "live out {out_live} vs {expected}"
        );
    }
}
