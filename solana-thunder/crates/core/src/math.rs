//! Fixed-point math shared by the concentrated-liquidity DEX crates
//! (Raydium CLMM, Meteora DAMM v2, Meteora DLMM).
//!
//! All sqrt prices are Q64.64 (`sqrt_price_x64`): the raw token1-per-token0
//! amount ratio is `(sqrt_price_x64 / 2^64)^2`.
//!
//! Intermediate products can reach 2^320 (`(liquidity << 64) * sqrt_price`),
//! so a minimal 512-bit unsigned integer is vendored here. Correctness over
//! speed: division is bit-by-bit long division, mirroring the semantics of
//! the `uint`-crate U256/U512 math used by the on-chain programs.

/// Q64.64 fixed point resolution.
pub const Q64: u128 = 1u128 << 64;

// ============================================================================
// Minimal 512-bit unsigned integer (little-endian u64 limbs)
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct U512([u64; 8]);

impl U512 {
    pub const ZERO: U512 = U512([0; 8]);

    pub fn from_u128(v: u128) -> Self {
        let mut l = [0u64; 8];
        l[0] = v as u64;
        l[1] = (v >> 64) as u64;
        U512(l)
    }

    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|&l| l == 0)
    }

    /// Convert back to u128; None if the value doesn't fit.
    pub fn to_u128(&self) -> Option<u128> {
        if self.0[2..].iter().any(|&l| l != 0) {
            return None;
        }
        Some((self.0[1] as u128) << 64 | self.0[0] as u128)
    }

    fn cmp_u512(&self, other: &U512) -> core::cmp::Ordering {
        for i in (0..8).rev() {
            if self.0[i] != other.0[i] {
                return self.0[i].cmp(&other.0[i]);
            }
        }
        core::cmp::Ordering::Equal
    }

    /// self + other; None on overflow past 512 bits.
    pub fn checked_add(&self, other: &U512) -> Option<U512> {
        let mut out = [0u64; 8];
        let mut carry = 0u64;
        for i in 0..8 {
            let (s1, c1) = self.0[i].overflowing_add(other.0[i]);
            let (s2, c2) = s1.overflowing_add(carry);
            out[i] = s2;
            carry = (c1 as u64) + (c2 as u64);
        }
        if carry != 0 { None } else { Some(U512(out)) }
    }

    /// self - other; None if other > self.
    pub fn checked_sub(&self, other: &U512) -> Option<U512> {
        if self.cmp_u512(other) == core::cmp::Ordering::Less {
            return None;
        }
        let mut out = [0u64; 8];
        let mut borrow = 0u64;
        for i in 0..8 {
            let (d1, b1) = self.0[i].overflowing_sub(other.0[i]);
            let (d2, b2) = d1.overflowing_sub(borrow);
            out[i] = d2;
            borrow = (b1 as u64) + (b2 as u64);
        }
        Some(U512(out))
    }

    /// self * rhs (u128); None on overflow past 512 bits.
    pub fn checked_mul_u128(&self, rhs: u128) -> Option<U512> {
        let r = [rhs as u64, (rhs >> 64) as u64];
        let mut out = [0u64; 8];
        for (j, &rj) in r.iter().enumerate() {
            if rj == 0 {
                continue;
            }
            let mut carry: u128 = 0;
            for i in 0..8 {
                if i + j >= 8 {
                    if self.0[i] != 0 {
                        return None; // limb overflow
                    }
                    continue;
                }
                let prod = self.0[i] as u128 * rj as u128 + out[i + j] as u128 + carry;
                out[i + j] = prod as u64;
                carry = prod >> 64;
            }
            if carry != 0 {
                return None;
            }
        }
        Some(U512(out))
    }

    /// Left shift by 64 bits; None on overflow.
    pub fn checked_shl64(&self) -> Option<U512> {
        if self.0[7] != 0 {
            return None;
        }
        let mut out = [0u64; 8];
        for i in (1..8).rev() {
            out[i] = self.0[i - 1];
        }
        Some(U512(out))
    }

    /// (quotient, remainder) of self / divisor. None if divisor is zero.
    /// Bit-by-bit long division: slow but exact.
    pub fn div_rem(&self, divisor: &U512) -> Option<(U512, U512)> {
        if divisor.is_zero() {
            return None;
        }
        let mut quotient = [0u64; 8];
        let mut rem = U512::ZERO;
        // Find highest set bit of self.
        let mut top = 0usize;
        for i in (0..8).rev() {
            if self.0[i] != 0 {
                top = i * 64 + (63 - self.0[i].leading_zeros() as usize);
                break;
            }
        }
        if self.is_zero() {
            return Some((U512::ZERO, U512::ZERO));
        }
        let mut bit = top as isize;
        while bit >= 0 {
            // rem = rem << 1 | self.bit(bit)
            let mut carry = (self.0[(bit as usize) / 64] >> ((bit as usize) % 64)) & 1;
            for limb in rem.0.iter_mut() {
                let new_carry = *limb >> 63;
                *limb = (*limb << 1) | carry;
                carry = new_carry;
            }
            // carry out of 512 bits cannot happen: rem < divisor <= 2^512-1
            if rem.cmp_u512(divisor) != core::cmp::Ordering::Less {
                rem = rem.checked_sub(divisor)?;
                quotient[(bit as usize) / 64] |= 1u64 << ((bit as usize) % 64);
            }
            bit -= 1;
        }
        Some((U512(quotient), rem))
    }
}

// ============================================================================
// mul_div primitives
// ============================================================================

/// floor(a * b / denom) over u128 with a 256-bit intermediate.
/// None if denom == 0 or the result overflows u128.
pub fn mul_div_floor(a: u128, b: u128, denom: u128) -> Option<u128> {
    let prod = U512::from_u128(a).checked_mul_u128(b)?;
    let (q, _r) = prod.div_rem(&U512::from_u128(denom))?;
    q.to_u128()
}

/// ceil(a * b / denom) over u128 with a 256-bit intermediate.
pub fn mul_div_ceil(a: u128, b: u128, denom: u128) -> Option<u128> {
    let prod = U512::from_u128(a).checked_mul_u128(b)?;
    let (q, r) = prod.div_rem(&U512::from_u128(denom))?;
    let q = q.to_u128()?;
    if r.is_zero() { Some(q) } else { q.checked_add(1) }
}

// ============================================================================
// Concentrated-liquidity swap primitives (Q64.64)
// ============================================================================
//
// Mirrors Raydium CLMM `sqrt_price_math.rs` / Meteora cp-amm `curve.rs`.
// token0 = "base of the sqrt price" (price = token1/token0); zero-for-one
// swaps push the sqrt price down, one-for-zero push it up.

/// Δtoken0 between two sqrt prices for `liquidity`:
/// amount0 = L * 2^64 * (sqrt_b - sqrt_a) / (sqrt_b * sqrt_a)
/// `sqrt_a`/`sqrt_b` in any order. None on overflow / zero price.
pub fn get_amount_0_delta(
    sqrt_a: u128,
    sqrt_b: u128,
    liquidity: u128,
    round_up: bool,
) -> Option<u128> {
    let (lo, hi) = if sqrt_a <= sqrt_b { (sqrt_a, sqrt_b) } else { (sqrt_b, sqrt_a) };
    if lo == 0 {
        return None;
    }
    let num1 = U512::from_u128(liquidity).checked_shl64()?; // L << 64
    let num2 = hi - lo;
    let prod = num1.checked_mul_u128(num2)?;
    let (q1, r1) = prod.div_rem(&U512::from_u128(hi))?;
    if round_up {
        // ceil(ceil(prod / hi) / lo)
        let q1 = if r1.is_zero() { q1 } else { q1.checked_add(&U512::from_u128(1))? };
        let (q2, r2) = q1.div_rem(&U512::from_u128(lo))?;
        let q2 = q2.to_u128()?;
        if r2.is_zero() { Some(q2) } else { q2.checked_add(1) }
    } else {
        let (q2, _) = q1.div_rem(&U512::from_u128(lo))?;
        q2.to_u128()
    }
}

/// Δtoken1 between two sqrt prices for `liquidity`:
/// amount1 = L * (sqrt_b - sqrt_a) / 2^64
pub fn get_amount_1_delta(
    sqrt_a: u128,
    sqrt_b: u128,
    liquidity: u128,
    round_up: bool,
) -> Option<u128> {
    let (lo, hi) = if sqrt_a <= sqrt_b { (sqrt_a, sqrt_b) } else { (sqrt_b, sqrt_a) };
    if round_up {
        mul_div_ceil(liquidity, hi - lo, Q64)
    } else {
        mul_div_floor(liquidity, hi - lo, Q64)
    }
}

/// Next sqrt price after adding `amount` of token0 (price moves DOWN).
/// sqrt_new = L * 2^64 * sqrt_p / (L * 2^64 + amount * sqrt_p), rounded up.
pub fn next_sqrt_price_from_amount_0_in(
    sqrt_price: u128,
    liquidity: u128,
    amount: u128,
) -> Option<u128> {
    if amount == 0 {
        return Some(sqrt_price);
    }
    let numerator = U512::from_u128(liquidity).checked_shl64()?; // L << 64
    let product = U512::from_u128(amount).checked_mul_u128(sqrt_price)?;
    let denominator = numerator.checked_add(&product)?;
    // ceil(numerator * sqrt_price / denominator)
    let n = numerator.checked_mul_u128(sqrt_price)?;
    let (q, r) = n.div_rem(&denominator)?;
    let q = q.to_u128()?;
    if r.is_zero() { Some(q) } else { q.checked_add(1) }
}

/// Next sqrt price after adding `amount` of token1 (price moves UP).
/// sqrt_new = sqrt_p + amount * 2^64 / L, rounded down.
pub fn next_sqrt_price_from_amount_1_in(
    sqrt_price: u128,
    liquidity: u128,
    amount: u128,
) -> Option<u128> {
    if amount == 0 {
        return Some(sqrt_price);
    }
    if liquidity == 0 {
        return None;
    }
    let quotient = mul_div_floor(amount, Q64, liquidity)?;
    sqrt_price.checked_add(quotient)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mul_div_basic() {
        assert_eq!(mul_div_floor(10, 10, 3), Some(33));
        assert_eq!(mul_div_ceil(10, 10, 3), Some(34));
        assert_eq!(mul_div_floor(u128::MAX, u128::MAX, u128::MAX), Some(u128::MAX));
        assert_eq!(mul_div_floor(1, 1, 0), None);
        // Overflow: MAX * MAX / 1 does not fit u128.
        assert_eq!(mul_div_floor(u128::MAX, 2, 1), None);
    }

    #[test]
    fn test_mul_div_large_intermediate() {
        // (2^100 * 2^100) / 2^100 = 2^100 — intermediate is 200 bits.
        let v = 1u128 << 100;
        assert_eq!(mul_div_floor(v, v, v), Some(v));
    }

    #[test]
    fn test_amount_deltas_closed_form() {
        // L = 2^70, sqrt from 1.0 to 1.25 (Q64.64):
        // amount1 = L * 0.25 = 2^68; amount0 = L * 2^64 * 0.25 / 1.25 = L * 0.2
        let l = 1u128 << 70;
        let sa = Q64;
        let sb = Q64 + Q64 / 4;
        assert_eq!(get_amount_1_delta(sa, sb, l, false), Some(l / 4));
        assert_eq!(get_amount_0_delta(sa, sb, l, false), Some(l / 5));
        // round_up variants must be >= floor variants
        assert!(get_amount_0_delta(sa, sb, l, true).unwrap() >= l / 5);
    }

    #[test]
    fn test_next_sqrt_price_round_trip() {
        // Push price down with token0, verify the consumed amount matches.
        let l: u128 = 5_000_000_000_000; // realistic liquidity
        let sqrt_p: u128 = 5078790298593760744; // ~sqrt(0.0758) in Q64.64
        let amount: u128 = 1_000_000_000; // 1 SOL
        let sqrt_next = next_sqrt_price_from_amount_0_in(sqrt_p, l, amount).unwrap();
        assert!(sqrt_next < sqrt_p);
        let consumed = get_amount_0_delta(sqrt_next, sqrt_p, l, true).unwrap();
        // Rounding: consumed within 1 unit of requested.
        assert!(consumed >= amount - 1 && consumed <= amount + 1, "consumed={consumed}");

        // Push price up with token1.
        let sqrt_up = next_sqrt_price_from_amount_1_in(sqrt_p, l, amount).unwrap();
        assert!(sqrt_up > sqrt_p);
        let consumed1 = get_amount_1_delta(sqrt_p, sqrt_up, l, true).unwrap();
        assert!(consumed1 >= amount - 1 && consumed1 <= amount + 1, "consumed1={consumed1}");
    }

    #[test]
    fn test_u512_div_rem() {
        let a = U512::from_u128(u128::MAX).checked_mul_u128(u128::MAX).unwrap();
        let (q, r) = a.div_rem(&U512::from_u128(u128::MAX)).unwrap();
        assert_eq!(q.to_u128(), Some(u128::MAX));
        assert!(r.is_zero());
    }
}
