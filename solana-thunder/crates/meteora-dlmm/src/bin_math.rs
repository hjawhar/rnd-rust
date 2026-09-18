//! Real Meteora DLMM swap math: bin walk with base + variable fees.
//!
//! Mirrors the official off-chain quoter (`MeteoraAg/dlmm-sdk`
//! `commons/src/quote.rs` + `extensions/lb_pair.rs` + `extensions/bin.rs`):
//! exact-in, per-bin fills at the bin's stored Q64.64 price, volatility
//! accumulator updated per crossed bin, fee = base + variable capped at 10%.
//!
//! Account layouts (verified against mainnet dumps + dlmm IDL, July 2026):
//! - LbPair (904 bytes): StaticParameters @8..40 (base_factor u16 @8,
//!   filter_period @10, decay_period @12, reduction_factor @14,
//!   variable_fee_control u32 @16, max_volatility_accumulator u32 @20,
//!   min_bin_id i32 @24, max_bin_id i32 @28, protocol_share u16 @32,
//!   base_fee_power_factor u8 @34, function_type u8 @35, collect_fee_mode
//!   u8 @36), VariableParameters @40..72 (volatility_accumulator u32 @40,
//!   volatility_reference u32 @44, index_reference i32 @48,
//!   last_update_timestamp i64 @56), active_id i32 @76, bin_step u16 @80,
//!   status u8 @82, bin_array_bitmap [u64;16] @584..712.
//! - BinArray (10136 bytes): index i64 @8, lb_pair @24..56, bins @56
//!   (70 x 144 bytes: amount_x u64 @+0, amount_y u64 @+8, price u128 Q64.64
//!   @+16, open_order_amount u64 @+112, processed_order_remaining_amount
//!   u64 @+128, limit_order_ask_side u8 @+140).

use solana_pubkey::Pubkey;
use thunder_core::math::{mul_div_ceil, mul_div_floor, Q64};
use thunder_core::{AccountDataProvider, GenericError};

use crate::METEORA_DYNAMIC_LMM;

pub const FEE_PRECISION: u128 = 1_000_000_000;
pub const MAX_FEE_RATE: u128 = 100_000_000; // 10%
pub const BASIS_POINT_MAX: u128 = 10_000;
pub const MAX_BIN_PER_ARRAY: i32 = 70;

const BIN_ARRAY_HEADER: usize = 56;
const BIN_SIZE: usize = 144;
const BIN_ARRAY_DATA_LEN: usize = BIN_ARRAY_HEADER + 70 * BIN_SIZE;

/// Max bin arrays touched per swap before giving up.
const MAX_ARRAYS_VISITED: usize = 8;

// ============================================================================
// Live LbPair parsing
// ============================================================================

/// Swap-relevant fields of a live DLMM LbPair account.
#[derive(Clone)]
pub struct DlmmPairState {
    // Static parameters
    pub base_factor: u16,
    pub filter_period: u16,
    pub decay_period: u16,
    pub reduction_factor: u16,
    pub variable_fee_control: u32,
    pub max_volatility_accumulator: u32,
    pub min_bin_id: i32,
    pub max_bin_id: i32,
    pub base_fee_power_factor: u8,
    pub collect_fee_mode: u8,
    // Variable parameters
    pub volatility_accumulator: u32,
    pub volatility_reference: u32,
    pub index_reference: i32,
    pub last_update_timestamp: i64,
    // Pair
    pub active_id: i32,
    pub bin_step: u16,
    pub bin_array_bitmap: [u64; 16],
}

impl DlmmPairState {
    pub fn parse(data: &[u8]) -> Result<Self, GenericError> {
        if data.len() < 712 {
            return Err("DLMM pool account too short".into());
        }
        let u16_at = |o: usize| u16::from_le_bytes(data[o..o + 2].try_into().unwrap());
        let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
        let i32_at = |o: usize| i32::from_le_bytes(data[o..o + 4].try_into().unwrap());
        let mut bitmap = [0u64; 16];
        for (i, w) in bitmap.iter_mut().enumerate() {
            let off = 584 + i * 8;
            *w = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        }
        Ok(Self {
            base_factor: u16_at(8),
            filter_period: u16_at(10),
            decay_period: u16_at(12),
            reduction_factor: u16_at(14),
            variable_fee_control: u32_at(16),
            max_volatility_accumulator: u32_at(20),
            min_bin_id: i32_at(24),
            max_bin_id: i32_at(28),
            base_fee_power_factor: data[34],
            collect_fee_mode: data[36],
            volatility_accumulator: u32_at(40),
            volatility_reference: u32_at(44),
            index_reference: i32_at(48),
            last_update_timestamp: i64::from_le_bytes(data[56..64].try_into().unwrap()),
            active_id: i32_at(76),
            bin_step: u16_at(80),
            bin_array_bitmap: bitmap,
        })
    }

    /// base fee rate (1e-9 units) = base_factor * bin_step * 10 * 10^power.
    pub fn base_fee_rate(&self) -> u128 {
        (self.base_factor as u128)
            .saturating_mul(self.bin_step as u128)
            .saturating_mul(10)
            .saturating_mul(10u128.pow(self.base_fee_power_factor as u32))
    }

    /// variable fee rate (1e-9 units) for a given volatility accumulator:
    /// ceil(vfc * (va * bin_step)^2 / 1e11).
    pub fn variable_fee_rate(&self, volatility_accumulator: u32) -> u128 {
        if self.variable_fee_control == 0 {
            return 0;
        }
        let vfa_bin = (volatility_accumulator as u128).saturating_mul(self.bin_step as u128);
        let square = vfa_bin.saturating_mul(vfa_bin);
        let v_fee = (self.variable_fee_control as u128).saturating_mul(square);
        (v_fee + 99_999_999_999) / 100_000_000_000
    }

    pub fn total_fee_rate(&self, volatility_accumulator: u32) -> u128 {
        (self.base_fee_rate() + self.variable_fee_rate(volatility_accumulator)).min(MAX_FEE_RATE)
    }

    /// Whether the fee is charged on the input token for this direction.
    /// collect_fee_mode 0 = InputOnly, 1 = OnlyY (fee on Y side).
    pub fn fee_on_input(&self, swap_for_y: bool) -> bool {
        match self.collect_fee_mode {
            1 => !swap_for_y,
            _ => true,
        }
    }

    /// Decay the volatility reference based on elapsed time since the last
    /// on-chain update (dlmm `update_references`).
    pub fn update_references(&mut self, current_timestamp: i64) {
        let elapsed = current_timestamp.saturating_sub(self.last_update_timestamp);
        if elapsed >= self.filter_period as i64 {
            self.index_reference = self.active_id;
            if elapsed < self.decay_period as i64 {
                self.volatility_reference = ((self.volatility_accumulator as u64
                    * self.reduction_factor as u64)
                    / BASIS_POINT_MAX as u64) as u32;
            } else {
                self.volatility_reference = 0;
            }
        }
    }

    /// va = min(vref + |index_ref - bin_id| * 10000, max_va).
    pub fn volatility_accumulator_for(&self, bin_id: i32) -> u32 {
        let delta = (self.index_reference as i64 - bin_id as i64).unsigned_abs();
        let va = self.volatility_reference as u64 + delta * BASIS_POINT_MAX as u64;
        va.min(self.max_volatility_accumulator as u64) as u32
    }
}

// ============================================================================
// PDA derivation (self-contained: mirrors aggregator/cache.rs)
// ============================================================================

/// Bin array PDA: seeds ["bin_array", lb_pair, i64_le(index)].
pub fn bin_array_pda(lb_pair: &Pubkey, index: i64) -> Pubkey {
    let program = Pubkey::from_str_const(METEORA_DYNAMIC_LMM);
    Pubkey::find_program_address(
        &[b"bin_array", lb_pair.as_ref(), &index.to_le_bytes()],
        &program,
    )
    .0
}

/// Bitmap extension PDA: seeds ["bitmap", lb_pair].
pub fn bitmap_extension_pda(lb_pair: &Pubkey) -> Pubkey {
    let program = Pubkey::from_str_const(METEORA_DYNAMIC_LMM);
    Pubkey::find_program_address(&[b"bitmap", lb_pair.as_ref()], &program).0
}

/// The bin array index containing `bin_id` (floor division by 70).
pub fn bin_array_index(bin_id: i32) -> i64 {
    (bin_id as i64).div_euclid(MAX_BIN_PER_ARRAY as i64)
}

// ============================================================================
// Bitmap checks
// ============================================================================

/// In-pool bitmap covers array indices -512..=511 (bit = idx + 512).
fn in_pool_bitmap_bit(bitmap: &[u64; 16], idx: i64) -> Option<bool> {
    if !(-512..=511).contains(&idx) {
        return None;
    }
    let pos = (idx + 512) as usize;
    Some(bitmap[pos / 64] & (1u64 << (pos % 64)) != 0)
}

/// Extension bitmap: positive side first (idx 512..), then negative
/// (idx -513..). Layout mirrors the on-chain BinArrayBitmapExtension
/// (lb_pair @8..40, then two equal-size bitmap halves).
fn extension_bitmap_bit(ext: &[u8], idx: i64) -> Option<bool> {
    if ext.len() <= 40 || (-512..=511).contains(&idx) {
        return None;
    }
    let half = (ext.len() - 40) / 2;
    let bits_per_side = (half * 8) as i64;
    let (byte_start, offset) = if idx > 511 {
        (40usize, idx - 512)
    } else {
        (40 + half, -idx - 513)
    };
    if offset < 0 || offset >= bits_per_side {
        return None;
    }
    let word = offset as usize / 64;
    let bit = offset as usize % 64;
    let off = byte_start + word * 8;
    if off + 8 > ext.len() {
        return None;
    }
    let w = u64::from_le_bytes(ext[off..off + 8].try_into().unwrap());
    Some(w & (1u64 << bit) != 0)
}

// ============================================================================
// Swap computation
// ============================================================================

struct BinView<'a> {
    data: &'a [u8],
    array_index: i64,
}

impl<'a> BinView<'a> {
    /// (available_out, price) for the bin, in the swap direction.
    /// available_out includes market-making liquidity plus open/processing
    /// limit orders on the fillable side (all fill at the same bin price).
    fn read(&self, bin_id: i32, swap_for_y: bool) -> (u64, u128) {
        let slot = (bin_id as i64 - self.array_index * MAX_BIN_PER_ARRAY as i64) as usize;
        let off = BIN_ARRAY_HEADER + slot * BIN_SIZE;
        let d = self.data;
        let amount_x = u64::from_le_bytes(d[off..off + 8].try_into().unwrap());
        let amount_y = u64::from_le_bytes(d[off + 8..off + 16].try_into().unwrap());
        let price = u128::from_le_bytes(d[off + 16..off + 32].try_into().unwrap());
        let open_order = u64::from_le_bytes(d[off + 112..off + 120].try_into().unwrap());
        let processed_remaining =
            u64::from_le_bytes(d[off + 128..off + 136].try_into().unwrap());
        let is_ask_side = d[off + 140] != 0;

        let mm = if swap_for_y { amount_y } else { amount_x };
        // swap_for_y fills bid-side orders; swap_for_x fills ask-side orders.
        let orders = if (swap_for_y && !is_ask_side) || (!swap_for_y && is_ask_side) {
            open_order.saturating_add(processed_remaining)
        } else {
            0
        };
        (mm.saturating_add(orders), price)
    }
}

/// out for `amount_in` at a bin `price` (Q64.64 y-per-x):
/// swap_for_y: floor(price * in / 2^64); else floor(in * 2^64 / price).
fn amount_out_at_price(amount_in: u128, price: u128, swap_for_y: bool) -> Option<u128> {
    if swap_for_y {
        mul_div_floor(price, amount_in, Q64)
    } else {
        mul_div_floor(amount_in, Q64, price)
    }
}

/// in needed for `amount_out` at a bin price (rounded up).
fn amount_in_for_out(amount_out: u128, price: u128, swap_for_y: bool) -> Option<u128> {
    if swap_for_y {
        mul_div_ceil(amount_out, Q64, price)
    } else {
        mul_div_ceil(price, amount_out, Q64)
    }
}

/// Compute the exact-in output for a DLMM swap by walking bins.
///
/// `swap_for_y`: input is token X (active_id walks DOWN); the output is Y.
/// This matches the crate's physical "Sell" (X -> Y) convention.
///
/// Missing-data policy: if a bin array that the bitmap marks as holding
/// liquidity is not available from the provider before the input is fully
/// consumed, returns `Err` (router skips the pool for that size).
pub fn compute_swap_output(
    pair: &DlmmPairState,
    lb_pair: &Pubkey,
    amount_in: u64,
    swap_for_y: bool,
    provider: &dyn AccountDataProvider,
    current_timestamp: i64,
) -> Result<u64, GenericError> {
    if amount_in == 0 {
        return Ok(0);
    }
    let mut pair_state = pair.clone();
    pair_state.update_references(current_timestamp);

    let fee_on_input = pair_state.fee_on_input(swap_for_y);
    let ext_data = provider.pool_account_data(&bitmap_extension_pda(lb_pair));

    let mut amount_left: u64 = amount_in;
    let mut total_out: u128 = 0;
    let mut bin_id = pair_state.active_id;
    let step: i32 = if swap_for_y { -1 } else { 1 };

    let mut current_array_index = bin_array_index(bin_id);
    let mut current_array_data: Option<Vec<u8>> = None;
    let mut arrays_visited = 0usize;

    while amount_left > 0 {
        if bin_id < pair_state.min_bin_id || bin_id > pair_state.max_bin_id {
            return Err("DLMM swap exhausts bin range — insufficient liquidity for size".into());
        }
        let idx = bin_array_index(bin_id);
        if idx != current_array_index || current_array_data.is_none() {
            current_array_index = idx;
            // Bitmap gate: skip whole arrays that hold no liquidity.
            let has_liquidity = match in_pool_bitmap_bit(&pair_state.bin_array_bitmap, idx) {
                Some(bit) => bit,
                None => match ext_data.as_deref().and_then(|e| extension_bitmap_bit(e, idx)) {
                    Some(bit) => bit,
                    // Unknown (no extension data): assume it may hold
                    // liquidity and require the account.
                    None => true,
                },
            };
            if !has_liquidity {
                // Jump past this empty array.
                current_array_data = None;
                bin_id = if swap_for_y {
                    (idx * MAX_BIN_PER_ARRAY as i64 - 1) as i32
                } else {
                    ((idx + 1) * MAX_BIN_PER_ARRAY as i64) as i32
                };
                arrays_visited += 1;
                if arrays_visited > MAX_ARRAYS_VISITED {
                    return Err("DLMM walk exceeded max bin arrays".into());
                }
                continue;
            }
            let pda = bin_array_pda(lb_pair, idx);
            let data = provider.pool_account_data(&pda).ok_or_else(|| {
                format!("DLMM bin array {pda} (index {idx}) not in provider — pool skipped for this size")
            })?;
            if data.len() < BIN_ARRAY_DATA_LEN {
                return Err("DLMM bin array account too short".into());
            }
            current_array_data = Some(data);
            arrays_visited += 1;
            if arrays_visited > MAX_ARRAYS_VISITED {
                return Err("DLMM walk exceeded max bin arrays".into());
            }
        }

        let view = BinView {
            data: current_array_data.as_deref().unwrap(),
            array_index: current_array_index,
        };
        let (avail_out, stored_price) = view.read(bin_id, swap_for_y);

        if avail_out > 0 {
            // Volatility accumulator updates per bin actually traded.
            let va = pair_state.volatility_accumulator_for(bin_id);
            let fee_rate = pair_state.total_fee_rate(va);

            let price = if stored_price > 0 {
                stored_price
            } else {
                // Fallback: (1 + bin_step/10000)^bin_id in Q64.64.
                let p = (1.0 + pair_state.bin_step as f64 / 10_000.0).powi(bin_id)
                    * (1u128 << 64) as f64;
                if !(1.0..=3.4e38).contains(&p) {
                    return Err("DLMM bin price out of range".into());
                }
                p as u128
            };

            let max_in =
                amount_in_for_out(avail_out as u128, price, swap_for_y).ok_or("fee math overflow")?;

            if fee_on_input {
                // fee charged on gross input: net = gross - ceil(gross*rate/1e9)
                let fee_whole = mul_div_ceil(amount_left as u128, fee_rate, FEE_PRECISION)
                    .ok_or("fee math overflow")?;
                let net_avail = (amount_left as u128).saturating_sub(fee_whole);
                if net_avail >= max_in {
                    // Bin fully consumed.
                    total_out += avail_out as u128;
                    let fee_amt = mul_div_ceil(max_in, fee_rate, FEE_PRECISION - fee_rate)
                        .ok_or("fee math overflow")?;
                    let gross = max_in.saturating_add(fee_amt);
                    amount_left =
                        amount_left.saturating_sub(gross.min(amount_left as u128) as u64);
                } else {
                    let out_bin = amount_out_at_price(net_avail, price, swap_for_y)
                        .ok_or("bin math overflow")?
                        .min(avail_out as u128);
                    total_out += out_bin;
                    amount_left = 0;
                }
            } else {
                // fee charged on output.
                if (amount_left as u128) >= max_in {
                    let fee_amt = mul_div_ceil(avail_out as u128, fee_rate, FEE_PRECISION)
                        .ok_or("fee math overflow")?;
                    total_out += (avail_out as u128).saturating_sub(fee_amt);
                    amount_left =
                        amount_left.saturating_sub(max_in.min(amount_left as u128) as u64);
                } else {
                    let gross_out = amount_out_at_price(amount_left as u128, price, swap_for_y)
                        .ok_or("bin math overflow")?
                        .min(avail_out as u128);
                    let fee_amt = mul_div_ceil(gross_out, fee_rate, FEE_PRECISION)
                        .ok_or("fee math overflow")?;
                    total_out += gross_out.saturating_sub(fee_amt);
                    amount_left = 0;
                }
            }
        }

        if amount_left > 0 {
            bin_id += step;
        }
    }

    u64::try_from(total_out).map_err(|_| "output exceeds u64".into())
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

    fn make_pair(active_id: i32, bin_step: u16, base_factor: u16) -> DlmmPairState {
        let mut bitmap = [0u64; 16];
        // Mark the arrays we create in tests as holding liquidity.
        for idx in -2..=2i64 {
            let pos = (idx + 512) as usize;
            bitmap[pos / 64] |= 1u64 << (pos % 64);
        }
        DlmmPairState {
            base_factor,
            filter_period: 10,
            decay_period: 120,
            reduction_factor: 5000,
            variable_fee_control: 0, // no variable fee in synthetic tests
            max_volatility_accumulator: 100_000,
            min_bin_id: -436704,
            max_bin_id: 436704,
            base_fee_power_factor: 0,
            collect_fee_mode: 0,
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: active_id,
            last_update_timestamp: 0,
            active_id,
            bin_step,
            bin_array_bitmap: bitmap,
        }
    }

    /// Build a bin array where each listed bin holds the given (x, y).
    fn make_bin_array(
        lb_pair: &Pubkey,
        index: i64,
        bins: &[(i32, u64, u64)],
        bin_step: u16,
    ) -> Vec<u8> {
        let mut data = vec![0u8; BIN_ARRAY_DATA_LEN];
        data[8..16].copy_from_slice(&index.to_le_bytes());
        data[24..56].copy_from_slice(lb_pair.as_ref());
        for (bin_id, x, y) in bins {
            let slot = (*bin_id as i64 - index * 70) as usize;
            let off = BIN_ARRAY_HEADER + slot * BIN_SIZE;
            data[off..off + 8].copy_from_slice(&x.to_le_bytes());
            data[off + 8..off + 16].copy_from_slice(&y.to_le_bytes());
            let price = ((1.0 + bin_step as f64 / 10_000.0).powi(*bin_id)
                * (1u128 << 64) as f64) as u128;
            data[off + 16..off + 32].copy_from_slice(&price.to_le_bytes());
        }
        data
    }

    /// Within one bin the price is fixed: out = in_net * price (swap_for_y).
    #[test]
    fn test_single_bin_fixed_price() {
        let lb_pair = Pubkey::new_unique();
        // active bin 0, price exactly 1.0, plenty of y.
        let pair = make_pair(0, 10, 10_000); // base fee = 10000*10*10 = 1e6 (0.1%)
        let provider = MapProvider::new();
        provider.insert(
            bin_array_pda(&lb_pair, 0),
            make_bin_array(&lb_pair, 0, &[(0, 0, 1_000_000_000)], 10),
        );

        let amount_in: u64 = 100_000_000;
        let out = compute_swap_output(&pair, &lb_pair, amount_in, true, &provider, 1000).unwrap();
        // fee rate = 1e6/1e9 = 0.1% on input; price = 1.0.
        let expected = (amount_in as f64 * (1.0 - 0.001)) as u64;
        assert!(
            (out as i64 - expected as i64).abs() <= 2,
            "out {out} expected ~{expected}"
        );
    }

    /// Walking down across bins: each bin fills at its own (worse) price.
    #[test]
    fn test_multi_bin_walk_concave() {
        let lb_pair = Pubkey::new_unique();
        let pair = make_pair(0, 100, 1_000); // 1% bin step, base fee 0.1%
        let provider = MapProvider::new();
        // Bins 0, -1, -2 each hold y = 1000e6 (price 1.0, ~0.99, ~0.9801...).
        provider.insert(
            bin_array_pda(&lb_pair, -1),
            make_bin_array(
                &lb_pair,
                -1,
                &[(-1, 0, 1_000_000_000), (-2, 0, 1_000_000_000)],
                100,
            ),
        );
        provider.insert(
            bin_array_pda(&lb_pair, 0),
            make_bin_array(&lb_pair, 0, &[(0, 0, 1_000_000_000)], 100),
        );

        // Small swap: stays in bin 0 -> effective price ~1.0.
        let small: u64 = 100_000_000;
        let out_small =
            compute_swap_output(&pair, &lb_pair, small, true, &provider, 1000).unwrap();
        // Large swap: needs bins 0 and -1 -> blended price worse than 1.0.
        let large = small * 15; // 1.5e9 net > 1e9 in bin 0
        let out_large =
            compute_swap_output(&pair, &lb_pair, large, true, &provider, 1000).unwrap();

        let px_small = out_small as f64 / small as f64;
        let px_large = out_large as f64 / large as f64;
        assert!(
            px_large < px_small,
            "large trade must get worse price: {px_large} vs {px_small}"
        );
        // Exact: first 1e9 of net input yields 1e9 out (price 1.0);
        // remainder fills at price 1/1.01.
        let net = large as f64 * (1.0 - 0.001);
        let expected = 1_000_000_000.0 + (net - 1_000_000_000.0 / 1.0) * (1.0 / 1.01f64.powi(1));
        // Actually amount_in to drain bin 0 is 1e9 (price 1.0), remainder at bin -1.
        let rel = (out_large as f64 - expected).abs() / expected;
        assert!(rel < 1e-3, "out {out_large} vs expected {expected}");
    }

    /// Missing bin array -> Err (router skips pool for that size).
    #[test]
    fn test_missing_bin_array_errors() {
        let lb_pair = Pubkey::new_unique();
        let pair = make_pair(0, 100, 1_000);
        let provider = MapProvider::new();
        provider.insert(
            bin_array_pda(&lb_pair, 0),
            make_bin_array(&lb_pair, 0, &[(0, 0, 1_000_000_000)], 100),
        );
        // Swap larger than bin 0 holds; array -1 marked as having liquidity
        // in the bitmap but not present in the provider.
        let res = compute_swap_output(&pair, &lb_pair, 5_000_000_000, true, &provider, 1000);
        assert!(res.is_err());
    }

    /// Upward walk (swap_for_x): consumes x from bins above.
    #[test]
    fn test_walk_up_for_x() {
        let lb_pair = Pubkey::new_unique();
        let pair = make_pair(0, 100, 1_000);
        let provider = MapProvider::new();
        provider.insert(
            bin_array_pda(&lb_pair, 0),
            make_bin_array(
                &lb_pair,
                0,
                &[(0, 500_000_000, 0), (1, 500_000_000, 0)],
                100,
            ),
        );
        let amount_in: u64 = 600_000_000; // drains bin 0's x, dips into bin 1
        let out = compute_swap_output(&pair, &lb_pair, amount_in, false, &provider, 1000).unwrap();
        assert!(out > 0 && out < amount_in, "y in {amount_in} -> x out {out}");
    }
}
