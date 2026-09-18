//! Real Raydium CLMM swap math: Q64.64 tick walk with liquidity crossing.
//!
//! Mirrors the on-chain program (`raydium-io/raydium-clmm`, `tick_math.rs` +
//! `swap_math.rs` + `swap.rs`): exact-in, fee-on-input, per-tick-segment
//! stepping, `liquidity_net` applied when crossing initialized ticks.
//!
//! Account layouts (verified against mainnet dumps, July 2026):
//! - PoolState (1544 bytes): amm_config @9..41, tick_spacing @235..237,
//!   liquidity @237..253, sqrt_price_x64 @253..269, tick_current @269..273,
//!   tick_array_bitmap @904..1032.
//! - AmmConfig (117 bytes): trade_fee_rate u32 @47..51 (units of 10^-6;
//!   observed tiers 100 / 200 / 500 / 2500 / 10000).
//! - TickArrayState (10240 bytes): pool_id @8..40, start_tick_index @40..44,
//!   ticks @44 (60 x 168 bytes: tick i32 @+0, liquidity_net i128 @+4,
//!   liquidity_gross u128 @+20), initialized_tick_count @10124.

use solana_pubkey::Pubkey;
use thunder_core::math::{
    get_amount_0_delta, get_amount_1_delta, mul_div_ceil, mul_div_floor,
    next_sqrt_price_from_amount_0_in, next_sqrt_price_from_amount_1_in,
};
use thunder_core::{AccountDataProvider, GenericError};

use crate::tick_arrays::{
    is_tick_array_initialized, pda_array_bitmap_address, pda_tick_array_address,
    tick_array_start_index,
};

pub const FEE_RATE_DENOMINATOR: u64 = 1_000_000;
/// Fallback fee (25 bps in 10^-6 units) used ONLY when the amm_config
/// account is unavailable from the provider.
pub const FALLBACK_TRADE_FEE_RATE: u32 = 2_500;

pub const MIN_TICK: i32 = -443_636;
pub const MAX_TICK: i32 = 443_636;
pub const MIN_SQRT_PRICE_X64: u128 = 4_295_048_016;
pub const MAX_SQRT_PRICE_X64: u128 = 79_226_673_521_066_979_257_578_248_091;

const TICK_ARRAY_SIZE: i32 = 60;
const TICK_STATE_SIZE: usize = 168;
const TICK_ARRAY_HEADER: usize = 44; // 8 disc + 32 pool_id + 4 start_tick_index
const TICK_ARRAY_DATA_LEN: usize = TICK_ARRAY_HEADER + 60 * TICK_STATE_SIZE;

/// Max initialized tick-arrays visited before giving up (walk is bounded by
/// how many arrays the provider actually holds anyway).
const MAX_ARRAYS_VISITED: usize = 10;
/// Max uninitialized array slots skipped while searching for the next
/// initialized array in the bitmap.
const MAX_ARRAY_SCAN: usize = 1024;
/// Max tick crossings per swap (safety bound).
const MAX_TICK_CROSSINGS: usize = 400;

// ============================================================================
// Pool state field parsing (raw account bytes, offsets verified above)
// ============================================================================

/// Swap-relevant fields of a live CLMM pool account.
pub struct ClmmPoolState {
    pub amm_config: Pubkey,
    pub tick_spacing: u16,
    pub liquidity: u128,
    pub sqrt_price_x64: u128,
    pub tick_current: i32,
    pub tick_array_bitmap: [u64; 16],
}

impl ClmmPoolState {
    pub fn parse(data: &[u8]) -> Result<Self, GenericError> {
        if data.len() < 1032 {
            return Err("CLMM pool account too short".into());
        }
        let amm_config = Pubkey::try_from(&data[9..41]).map_err(|_| "bad amm_config")?;
        let tick_spacing = u16::from_le_bytes(data[235..237].try_into().unwrap());
        let liquidity = u128::from_le_bytes(data[237..253].try_into().unwrap());
        let sqrt_price_x64 = u128::from_le_bytes(data[253..269].try_into().unwrap());
        let tick_current = i32::from_le_bytes(data[269..273].try_into().unwrap());
        let mut tick_array_bitmap = [0u64; 16];
        for (i, w) in tick_array_bitmap.iter_mut().enumerate() {
            let off = 904 + i * 8;
            *w = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        }
        Ok(Self {
            amm_config,
            tick_spacing,
            liquidity,
            sqrt_price_x64,
            tick_current,
            tick_array_bitmap,
        })
    }
}

/// Read `trade_fee_rate` (10^-6 units) from a raw AmmConfig account.
/// Returns None when the data is malformed or the value is not sane.
pub fn parse_amm_config_trade_fee_rate(data: &[u8]) -> Option<u32> {
    if data.len() < 51 {
        return None;
    }
    let rate = u32::from_le_bytes(data[47..51].try_into().unwrap());
    // Sanity: on-chain tiers are well below 10% (100_000).
    if rate == 0 || rate >= 500_000 { None } else { Some(rate) }
}

// ============================================================================
// Tick math (exact Raydium constants, `tick_math.rs`)
// ============================================================================

/// sqrt(1.0001^tick) in Q64.64 — exact port of Raydium's
/// `get_sqrt_price_at_tick` (same magic constants, same rounding).
pub fn get_sqrt_price_at_tick(tick: i32) -> Result<u128, GenericError> {
    if !(MIN_TICK..=MAX_TICK).contains(&tick) {
        return Err(format!("tick {tick} out of range").into());
    }
    let abs_tick = tick.unsigned_abs();

    let mut ratio: u128 = if abs_tick & 0x1 != 0 {
        0xfffcb933bd6fb800
    } else {
        1u128 << 64
    };
    const MULTIPLIERS: [(u32, u128); 18] = [
        (0x2, 0xfff97272373d4000),
        (0x4, 0xfff2e50f5f657000),
        (0x8, 0xffe5caca7e10f000),
        (0x10, 0xffcb9843d60f7000),
        (0x20, 0xff973b41fa98e800),
        (0x40, 0xff2ea16466c9b000),
        (0x80, 0xfe5dee046a9a3800),
        (0x100, 0xfcbe86c7900bb000),
        (0x200, 0xf987a7253ac65800),
        (0x400, 0xf3392b0822bb6000),
        (0x800, 0xe7159475a2caf000),
        (0x1000, 0xd097f3bdfd2f2000),
        (0x2000, 0xa9f746462d9f8000),
        (0x4000, 0x70d869a156f31c00),
        (0x8000, 0x31be135f97ed3200),
        (0x10000, 0x9aa508b5b85a500),
        (0x20000, 0x5d6af8dedc582c),
        (0x40000, 0x2216e584f5fa),
    ];
    for (mask, mult) in MULTIPLIERS {
        if abs_tick & mask != 0 {
            // ratio <= 2^64 and mult < 2^64, so the product fits u128.
            ratio = (ratio * mult) >> 64;
        }
    }
    if tick > 0 {
        ratio = u128::MAX / ratio;
    }
    Ok(ratio)
}

// ============================================================================
// Tick array access
// ============================================================================

struct TickInfo {
    tick: i32,
    liquidity_net: i128,
}

/// Find the next initialized tick inside one tick array.
/// zero_for_one: highest initialized tick with `tick_value <= from_tick`.
/// one_for_zero: lowest initialized tick with `tick_value > from_tick`.
fn find_initialized_tick_in_array(
    data: &[u8],
    from_tick: i32,
    zero_for_one: bool,
) -> Option<TickInfo> {
    let read = |i: usize| -> (i32, i128, u128) {
        let off = TICK_ARRAY_HEADER + i * TICK_STATE_SIZE;
        let tick = i32::from_le_bytes(data[off..off + 4].try_into().unwrap());
        let net = i128::from_le_bytes(data[off + 4..off + 20].try_into().unwrap());
        let gross = u128::from_le_bytes(data[off + 20..off + 36].try_into().unwrap());
        (tick, net, gross)
    };
    if zero_for_one {
        for i in (0..60).rev() {
            let (tick, net, gross) = read(i);
            if gross > 0 && tick <= from_tick {
                return Some(TickInfo { tick, liquidity_net: net });
            }
        }
    } else {
        for i in 0..60 {
            let (tick, net, gross) = read(i);
            if gross > 0 && tick > from_tick {
                return Some(TickInfo { tick, liquidity_net: net });
            }
        }
    }
    None
}

// ============================================================================
// Swap computation
// ============================================================================

/// Compute the exact-in output for a CLMM swap by walking ticks with live
/// account data.
///
/// `zero_for_one`: input is token_0 (price moves down). This matches the
/// physical "Buy" (token_0 -> token_1... no: token_0 in means the pool's
/// token_1 flows out) and agrees with `compute_clmm_remaining_accounts`'
/// `is_buy` semantics: is_buy == a_to_b == zero_for_one == tick decreases.
///
/// Missing-data policy (documented behavior change): if the walk needs a
/// tick array that the provider does not hold (or the pool's initialized
/// arrays are exhausted) before the full input is consumed, this returns
/// `Err` — the router will skip this pool for that trade size instead of
/// returning an optimistic linear estimate.
pub fn compute_swap_output(
    pool: &ClmmPoolState,
    pool_id: &Pubkey,
    amount_in: u64,
    zero_for_one: bool,
    provider: &dyn AccountDataProvider,
) -> Result<u64, GenericError> {
    if amount_in == 0 {
        return Ok(0);
    }
    let fee_rate = provider
        .pool_account_data(&pool.amm_config)
        .as_deref()
        .and_then(parse_amm_config_trade_fee_rate)
        .unwrap_or(FALLBACK_TRADE_FEE_RATE) as u64;

    let ticks_per_array = pool.tick_spacing as i32 * TICK_ARRAY_SIZE;
    if ticks_per_array == 0 {
        return Err("invalid tick spacing".into());
    }

    // Bitmap extension (optional): only needed for arrays beyond +-512.
    let ext_data = pda_array_bitmap_address(pool_id)
        .ok()
        .and_then(|(pda, _)| provider.pool_account_data(&pda));

    let mut sqrt_p = pool.sqrt_price_x64;
    let mut tick = pool.tick_current;
    let mut liquidity = pool.liquidity;
    let mut remaining: u64 = amount_in;
    let mut total_out: u128 = 0;
    let mut crossings = 0usize;
    // Cache fetched tick arrays by start index: each tick crossing restarts
    // the search from the current array, so without a cache the same array
    // would be re-fetched (and re-counted) once per crossing.
    let mut array_cache: std::collections::HashMap<i32, Vec<u8>> =
        std::collections::HashMap::new();

    while remaining > 0 {
        crossings += 1;
        if crossings > MAX_TICK_CROSSINGS {
            return Err("CLMM walk exceeded max tick crossings".into());
        }

        // ---- locate the next initialized tick in swap direction ----
        let mut found: Option<TickInfo> = None;
        let mut array_start = tick_array_start_index(tick, pool.tick_spacing);
        for _ in 0..MAX_ARRAY_SCAN {
            let idx = array_start.div_euclid(ticks_per_array);
            if idx < -512 || idx > 511 {
                // Beyond in-pool bitmap: extension required.
                if ext_data.is_none() || idx.abs() > 7680 {
                    break;
                }
            }
            if is_tick_array_initialized(&pool.tick_array_bitmap, ext_data.as_deref(), idx) {
                if !array_cache.contains_key(&array_start) {
                    if array_cache.len() >= MAX_ARRAYS_VISITED {
                        return Err("CLMM walk exceeded max tick arrays".into());
                    }
                    let (pda, _) = pda_tick_array_address(pool_id, array_start)?;
                    let data = provider.pool_account_data(&pda).ok_or_else(|| {
                        format!(
                            "CLMM tick array {pda} (start {array_start}) not in provider — \
                             pool skipped for this size"
                        )
                    })?;
                    if data.len() < TICK_ARRAY_DATA_LEN {
                        return Err("CLMM tick array account too short".into());
                    }
                    array_cache.insert(array_start, data);
                }
                let data = array_cache.get(&array_start).unwrap();
                if let Some(t) = find_initialized_tick_in_array(data, tick, zero_for_one) {
                    found = Some(t);
                    break;
                }
            }
            array_start += if zero_for_one { -ticks_per_array } else { ticks_per_array };
        }
        let Some(next_tick) = found else {
            return Err(
                "CLMM swap exhausts initialized tick arrays — insufficient liquidity for size"
                    .into(),
            );
        };

        let target = get_sqrt_price_at_tick(next_tick.tick)?;
        let target = if zero_for_one {
            target.max(MIN_SQRT_PRICE_X64 + 1)
        } else {
            target.min(MAX_SQRT_PRICE_X64 - 1)
        };

        // ---- step within [target, sqrt_p] (fee on input) ----
        let remaining_net =
            mul_div_floor(remaining as u128, FEE_RATE_DENOMINATOR as u128 - fee_rate as u128, FEE_RATE_DENOMINATOR as u128)
                .ok_or("fee math overflow")?;

        let amount_in_to_target = if liquidity == 0 {
            0u128
        } else if zero_for_one {
            get_amount_0_delta(target, sqrt_p, liquidity, true).ok_or("delta overflow")?
        } else {
            get_amount_1_delta(sqrt_p, target, liquidity, true).ok_or("delta overflow")?
        };

        if remaining_net >= amount_in_to_target {
            // Reach the tick boundary and cross it.
            if liquidity > 0 {
                let step_out = if zero_for_one {
                    get_amount_1_delta(target, sqrt_p, liquidity, false)
                } else {
                    get_amount_0_delta(sqrt_p, target, liquidity, false)
                }
                .ok_or("delta overflow")?;
                total_out = total_out.checked_add(step_out).ok_or("output overflow")?;
            }
            // Gross input consumed = net + fee (fee = ceil(net * rate / (1e6 - rate))).
            let fee_amt = mul_div_ceil(
                amount_in_to_target,
                fee_rate as u128,
                FEE_RATE_DENOMINATOR as u128 - fee_rate as u128,
            )
            .ok_or("fee math overflow")?;
            let gross = amount_in_to_target.saturating_add(fee_amt);
            remaining = remaining.saturating_sub(gross.min(remaining as u128) as u64);

            sqrt_p = target;
            // Cross: moving down subtracts liquidity_net, moving up adds it.
            liquidity = if zero_for_one {
                apply_liquidity_net(liquidity, next_tick.liquidity_net.checked_neg().ok_or("liq net overflow")?)?
            } else {
                apply_liquidity_net(liquidity, next_tick.liquidity_net)?
            };
            tick = if zero_for_one { next_tick.tick - 1 } else { next_tick.tick };
            if !(MIN_TICK..=MAX_TICK).contains(&tick) {
                return Err("CLMM walk hit tick bounds".into());
            }
        } else {
            // Final partial step within current liquidity.
            if liquidity == 0 {
                // Cannot absorb anything here and cannot reach a boundary.
                return Err("CLMM pool has zero active liquidity for remaining input".into());
            }
            let sqrt_next = if zero_for_one {
                next_sqrt_price_from_amount_0_in(sqrt_p, liquidity, remaining_net)
            } else {
                next_sqrt_price_from_amount_1_in(sqrt_p, liquidity, remaining_net)
            }
            .ok_or("sqrt price math overflow")?;
            let step_out = if zero_for_one {
                get_amount_1_delta(sqrt_next, sqrt_p, liquidity, false)
            } else {
                get_amount_0_delta(sqrt_p, sqrt_next, liquidity, false)
            }
            .ok_or("delta overflow")?;
            total_out = total_out.checked_add(step_out).ok_or("output overflow")?;
            remaining = 0;
        }
    }

    u64::try_from(total_out).map_err(|_| "output exceeds u64".into())
}

fn apply_liquidity_net(liquidity: u128, net: i128) -> Result<u128, GenericError> {
    if net >= 0 {
        liquidity.checked_add(net as u128).ok_or_else(|| "liquidity overflow".into())
    } else {
        liquidity
            .checked_sub(net.unsigned_abs())
            .ok_or_else(|| "liquidity underflow".into())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MapProvider(Mutex<HashMap<Pubkey, Vec<u8>>>);

    impl MapProvider {
        fn new() -> Self {
            Self(Mutex::new(HashMap::new()))
        }
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

    #[test]
    fn test_sqrt_price_at_tick_bounds_and_zero() {
        assert_eq!(get_sqrt_price_at_tick(0).unwrap(), 1u128 << 64);
        assert_eq!(get_sqrt_price_at_tick(MIN_TICK).unwrap(), MIN_SQRT_PRICE_X64);
        assert_eq!(get_sqrt_price_at_tick(MAX_TICK).unwrap(), MAX_SQRT_PRICE_X64);
        assert!(get_sqrt_price_at_tick(MAX_TICK + 1).is_err());
    }

    #[test]
    fn test_sqrt_price_at_tick_matches_float() {
        // Q64.64 fixed point should match f64 within float precision.
        for tick in [-443600, -100000, -25798, -60, -1, 1, 60, 25798, 100000, 443600] {
            let fixed = get_sqrt_price_at_tick(tick).unwrap() as f64 / (1u128 << 64) as f64;
            let float = 1.0001f64.powi(tick).sqrt();
            let rel = ((fixed - float) / float).abs();
            assert!(rel < 1e-9, "tick {tick}: fixed {fixed} vs float {float}");
        }
    }

    /// Build a synthetic tick array account with the given initialized ticks.
    fn make_tick_array(pool_id: &Pubkey, start_index: i32, ticks: &[(i32, i128)]) -> Vec<u8> {
        let mut data = vec![0u8; 10240];
        data[8..40].copy_from_slice(pool_id.as_ref());
        data[40..44].copy_from_slice(&start_index.to_le_bytes());
        for (tick, net) in ticks {
            let i = (tick - start_index) as usize; // tick_spacing = 1 in tests
            let off = TICK_ARRAY_HEADER + i * TICK_STATE_SIZE;
            data[off..off + 4].copy_from_slice(&tick.to_le_bytes());
            data[off + 4..off + 20].copy_from_slice(&net.to_le_bytes());
            let gross: u128 = net.unsigned_abs().max(1);
            data[off + 20..off + 36].copy_from_slice(&gross.to_le_bytes());
        }
        data
    }

    fn make_pool(
        liquidity: u128,
        sqrt_price_x64: u128,
        tick_current: i32,
        bitmap: [u64; 16],
    ) -> ClmmPoolState {
        ClmmPoolState {
            amm_config: Pubkey::new_unique(),
            tick_spacing: 1,
            liquidity,
            sqrt_price_x64,
            tick_current,
            tick_array_bitmap: bitmap,
        }
    }

    /// Single-segment swap (no crossing): output must match the closed-form
    /// in-range formula out = L * (sqrt_p - sqrt_new) / 2^64.
    #[test]
    fn test_walk_single_segment_matches_closed_form() {
        let pool_id = Pubkey::new_unique();
        let l: u128 = 1 << 80;
        let sqrt_p = 1u128 << 64; // price 1.0, tick 0
        let mut bitmap = [0u64; 16];
        bitmap[7] |= 1u64 << 63; // array index -1 initialized (start -60)
        let pool = make_pool(l, sqrt_p, 0, bitmap);

        let provider = MapProvider::new();
        // Far-away initialized tick at -60 so the whole swap stays in range.
        let arr = make_tick_array(&pool_id, -60, &[(-60, 1000)]);
        provider.insert(pda_tick_array_address(&pool_id, -60).unwrap().0, arr);

        let amount_in: u64 = 1_000_000_000;
        let out = compute_swap_output(&pool, &pool_id, amount_in, true, &provider).unwrap();

        // Closed form with fallback fee 25 bps (no amm_config in provider).
        // out = L*(sp - sp_new) with sp_new = L*sp/(L + net*sp), i.e.
        // out = net * sp^2 * L / (L + net*sp) — computed directly to avoid
        // f64 cancellation in (sp - sp_new).
        let net = amount_in as f64 * (1.0 - 0.0025);
        let sp = sqrt_p as f64 / (1u128 << 64) as f64;
        let lf = l as f64;
        let expected = net * sp * sp * lf / (lf + net * sp);
        let rel = (out as f64 - expected).abs() / expected;
        // With L = 2^80 the Q64.64 sqrt-price step quantizes output in units
        // of L >> 64 = 65536 (the on-chain program quantizes identically),
        // so allow ~1 bps here; real pools have far smaller L/amount ratios.
        assert!(rel < 1e-4, "out {out} vs expected {expected}");
    }

    /// Crossing an initialized tick must apply liquidity_net.
    /// Setup: L=2^80 above tick -10, liquidity_net at -10 removes half.
    /// A swap large enough to cross -10 must produce less output than the
    /// same swap in a pool with constant L (concavity through the cross).
    #[test]
    fn test_walk_crosses_tick_applies_liquidity_net() {
        let pool_id = Pubkey::new_unique();
        let l: u128 = 1 << 68;
        let sqrt_p = 1u128 << 64;
        let mut bitmap = [0u64; 16];
        bitmap[7] |= 1u64 << 63; // array -1 (start -60)
        let pool = make_pool(l, sqrt_p, 0, bitmap);

        // Tick -10 has liquidity_net = +L/2 (crossing down removes L/2).
        let half = (l / 2) as i128;
        let provider = MapProvider::new();
        let arr = make_tick_array(&pool_id, -60, &[(-60, half), (-10, half)]);
        provider.insert(pda_tick_array_address(&pool_id, -60).unwrap().0, arr);

        // Amount that pushes well past tick -10:
        // amount0 to move from tick 0 to -10 at L is ~ L*(1/s(-10) - 1) ~ L*5e-4.
        let to_boundary = (l as f64 * 5.0e-4) as u64;
        let amount_in = to_boundary * 2;
        let out_crossing =
            compute_swap_output(&pool, &pool_id, amount_in, true, &provider).unwrap();

        // Constant-L reference pool (same total range, no mid tick).
        let provider2 = MapProvider::new();
        let arr2 = make_tick_array(&pool_id, -60, &[(-60, half)]);
        provider2.insert(pda_tick_array_address(&pool_id, -60).unwrap().0, arr2);
        let out_constant =
            compute_swap_output(&pool, &pool_id, amount_in, true, &provider2).unwrap();

        assert!(
            out_crossing < out_constant,
            "crossing {out_crossing} must be < constant {out_constant}"
        );
        // Sanity: both are in the right ballpark (~ amount_in at price ~1).
        assert!(out_crossing > amount_in / 2);
    }

    /// When the walk needs an array the provider doesn't hold -> Err.
    #[test]
    fn test_walk_missing_array_errors() {
        let pool_id = Pubkey::new_unique();
        let l: u128 = 1 << 40; // small liquidity: forced to cross far
        let sqrt_p = 1u128 << 64;
        let mut bitmap = [0u64; 16];
        bitmap[7] |= 1u64 << 63; // array -1 initialized but NOT in provider
        let pool = make_pool(l, sqrt_p, 0, bitmap);

        let provider = MapProvider::new();
        let res = compute_swap_output(&pool, &pool_id, u64::MAX / 2, true, &provider);
        assert!(res.is_err());
    }

    /// Concavity: doubling input must yield strictly less than double output.
    #[test]
    fn test_walk_concave() {
        let pool_id = Pubkey::new_unique();
        let l: u128 = 1 << 72;
        let sqrt_p = 1u128 << 64;
        let mut bitmap = [0u64; 16];
        bitmap[7] |= 1u64 << 63;
        let pool = make_pool(l, sqrt_p, 0, bitmap);

        let provider = MapProvider::new();
        let arr = make_tick_array(&pool_id, -60, &[(-60, (l / 2) as i128)]);
        provider.insert(pda_tick_array_address(&pool_id, -60).unwrap().0, arr);

        let a: u64 = 1 << 40;
        let out1 = compute_swap_output(&pool, &pool_id, a, true, &provider).unwrap();
        let out2 = compute_swap_output(&pool, &pool_id, a * 2, true, &provider).unwrap();
        assert!(out2 < out1 * 2, "must be concave: {out2} vs 2x{out1}");
        assert!(out2 > out1, "more in, more out");
    }
}
