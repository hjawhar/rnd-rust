//! Benchmark case matrix generation.
//!
//! `gen-cases` loads the pool cache (never RPC), picks the major pairs
//! (SOL/USDC, SOL/USDT, USDC/USDT) plus N tokens per liquidity tier sampled
//! from the pool index by vault balance, and emits both directions across a
//! USD size ladder. Raw amounts are derived from a SOL/USD price supplied by
//! the caller (`--sol-price`) so no engine has to be running.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use thunder_aggregator::pool_index::PoolIndex;
use thunder_aggregator::{cache, price};
use thunder_core::{GenericError, USDC, USDT, WSOL, is_quote_mint};

/// USD trade sizes for every pair/direction.
pub const USD_LADDER: [f64; 7] = [0.1, 1.0, 10.0, 100.0, 1_000.0, 10_000.0, 100_000.0];

/// Liquidity tier boundaries (estimated pool USD TVL).
const TIER_MAJOR_MIN: f64 = 1_000_000.0;
const TIER_MID_MIN: f64 = 100_000.0;
const TIER_TAIL_MIN: f64 = 10_000.0;

/// One benchmark case (`bench/cases.json` entry).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchCase {
    pub id: String,
    pub input_mint: String,
    pub output_mint: String,
    /// Raw input amount (lamports / raw token units).
    pub amount: u64,
    pub usd_size: f64,
    /// "major" | "mid" | "tail"
    pub liquidity_tier: String,
    pub slippage_bps: u64,
}

/// Load the pool index from the on-disk cache. Never falls back to RPC.
pub fn load_index(cache_path: &Path) -> Result<PoolIndex, GenericError> {
    if !cache_path.exists() {
        return Err(format!(
            "pool cache not found at '{}'. gen-cases never fetches from RPC; \
             run the engine (or thunder-agg) once to build the cache, or point \
             CACHE_PATH at an existing pools.cache",
            cache_path.display()
        )
        .into());
    }
    let (index, ts) = cache::load_cache(cache_path)?;
    eprintln!(
        "loaded {} pools from {} (cache timestamp {ts})",
        index.pool_count(),
        cache_path.display()
    );
    Ok(index)
}

/// Assign a liquidity tier from estimated pool USD TVL.
pub fn tier_for_usd(pool_usd: f64) -> Option<&'static str> {
    if pool_usd >= TIER_MAJOR_MIN {
        Some("major")
    } else if pool_usd >= TIER_MID_MIN {
        Some("mid")
    } else if pool_usd >= TIER_TAIL_MIN {
        Some("tail")
    } else {
        None
    }
}

/// USD price per whole token for the counterpart (quote) side of a case.
/// Only WSOL / USDC / USDT are used as counterparts.
fn counterpart_usd(mint: &Pubkey, sol_price: f64) -> Option<f64> {
    if *mint == Pubkey::from_str_const(WSOL) {
        Some(sol_price)
    } else if *mint == Pubkey::from_str_const(USDC) || *mint == Pubkey::from_str_const(USDT) {
        Some(1.0)
    } else {
        None
    }
}

/// Short slug for case ids: known symbols, else the first 8 base58 chars.
pub fn mint_slug(mint: &str) -> String {
    match mint {
        WSOL => "sol".into(),
        USDC => "usdc".into(),
        USDT => "usdt".into(),
        other => other.chars().take(8).collect(),
    }
}

/// "0.1" / "1" / "100000" — ladder values format cleanly via Display.
pub fn format_usd(usd: f64) -> String {
    format!("{usd}")
}

/// Convert a USD size into raw input units given the input token's USD price
/// per whole token. None when the result is zero, non-finite, or overflows u64.
pub fn raw_amount(usd: f64, price_usd_per_whole: f64, decimals: u8) -> Option<u64> {
    if !(price_usd_per_whole.is_finite() && price_usd_per_whole > 0.0) {
        return None;
    }
    let raw = usd / price_usd_per_whole * 10f64.powi(decimals as i32);
    if raw.is_finite() && raw >= 1.0 && raw <= u64::MAX as f64 {
        Some(raw.round() as u64)
    } else {
        None
    }
}

/// One side of a tradable pair with everything needed for the USD conversion.
struct PairSide {
    mint: String,
    usd_per_whole: f64,
    decimals: u8,
}

/// Sampled long-tail token candidate.
struct TokenCand {
    mint: Pubkey,
    best_pool_usd: f64,
    counterpart: Pubkey,
    decimals: u8,
}

/// Generate the full case matrix.
pub fn generate(
    index: &PoolIndex,
    sol_price: f64,
    tokens_per_tier: usize,
    slippage_bps: u64,
) -> Result<Vec<BenchCase>, GenericError> {
    if !(sol_price.is_finite() && sol_price > 0.0) {
        return Err("SOL/USD price must be a positive number".into());
    }

    let wsol = Pubkey::from_str_const(WSOL);
    let usdc = Pubkey::from_str_const(USDC);
    let usdt = Pubkey::from_str_const(USDT);

    let mut cases = Vec::new();
    let mut used_ids: HashSet<String> = HashSet::new();

    // -- Major pairs -------------------------------------------------------
    let majors: [(&Pubkey, f64, u8, &Pubkey, f64, u8); 3] = [
        (&wsol, sol_price, 9, &usdc, 1.0, 6),
        (&wsol, sol_price, 9, &usdt, 1.0, 6),
        (&usdc, 1.0, 6, &usdt, 1.0, 6),
    ];
    for (a_mint, a_usd, a_dec, b_mint, b_usd, b_dec) in majors {
        let a = PairSide { mint: a_mint.to_string(), usd_per_whole: a_usd, decimals: a_dec };
        let b = PairSide { mint: b_mint.to_string(), usd_per_whole: b_usd, decimals: b_dec };
        push_pair_cases(&mut cases, &mut used_ids, &a, &b, "major", slippage_bps);
    }

    // -- Long-tail tokens sampled by vault-balance tier ---------------------
    let mut cands: HashMap<Pubkey, TokenCand> = HashMap::new();
    for (_addr, entry) in index.iter_pools() {
        let Some(cp_usd) = counterpart_usd(&entry.quote_mint, sol_price) else {
            continue;
        };
        // Base side must be a real long-tail token, not another quote currency.
        if is_quote_mint(&entry.base_mint) {
            continue;
        }
        let Ok(fin) = entry.market.financials() else { continue };
        let quote_whole = fin.quote_balance as f64 / 10f64.powi(fin.quote_decimals as i32);
        let pool_usd = 2.0 * quote_whole * cp_usd; // both sides of a balanced pool
        if !pool_usd.is_finite() || pool_usd < TIER_TAIL_MIN {
            continue;
        }
        let cand = cands.entry(entry.base_mint).or_insert_with(|| TokenCand {
            mint: entry.base_mint,
            best_pool_usd: 0.0,
            counterpart: entry.quote_mint,
            decimals: fin.base_decimals,
        });
        if pool_usd > cand.best_pool_usd {
            cand.best_pool_usd = pool_usd;
            cand.counterpart = entry.quote_mint;
            cand.decimals = fin.base_decimals;
        }
    }

    // Partition into tiers and sample deterministically (evenly spaced across
    // the tier sorted by liquidity descending — reproducible, no RNG).
    let mut tiers: HashMap<&'static str, Vec<&TokenCand>> = HashMap::new();
    for cand in cands.values() {
        if let Some(tier) = tier_for_usd(cand.best_pool_usd) {
            tiers.entry(tier).or_default().push(cand);
        }
    }

    let mut skipped_unpriced = 0usize;
    for tier in ["major", "mid", "tail"] {
        let mut list = tiers.remove(tier).unwrap_or_default();
        list.sort_by(|a, b| {
            b.best_pool_usd
                .partial_cmp(&a.best_pool_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.mint.to_string().cmp(&b.mint.to_string()))
        });
        for cand in sample_evenly(&list, tokens_per_tier) {
            // USD price of the token itself (needed for the token->counterpart
            // direction), derived from the pool index at gen time.
            let token_usd = price::get_token_price(index, &cand.mint, Some(sol_price))
                .ok()
                .and_then(|p| p.price_usd)
                .filter(|p| p.is_finite() && *p > 0.0);
            let Some(token_usd) = token_usd else {
                skipped_unpriced += 1;
                continue;
            };
            let cp_usd = counterpart_usd(&cand.counterpart, sol_price)
                .expect("counterpart is always WSOL/USDC/USDT");
            let cp = PairSide {
                mint: cand.counterpart.to_string(),
                usd_per_whole: cp_usd,
                decimals: if cand.counterpart == wsol { 9 } else { 6 },
            };
            let token = PairSide {
                mint: cand.mint.to_string(),
                usd_per_whole: token_usd,
                decimals: cand.decimals,
            };
            push_pair_cases(&mut cases, &mut used_ids, &cp, &token, tier, slippage_bps);
        }
    }
    if skipped_unpriced > 0 {
        eprintln!("warning: skipped {skipped_unpriced} sampled tokens with no derivable USD price");
    }

    Ok(cases)
}

/// Evenly spaced deterministic sample of up to `n` items.
fn sample_evenly<'a, T>(list: &'a [T], n: usize) -> Vec<&'a T> {
    if n == 0 || list.is_empty() {
        return Vec::new();
    }
    if list.len() <= n {
        return list.iter().collect();
    }
    (0..n).map(|i| &list[i * list.len() / n]).collect()
}

/// Both directions x USD ladder for one pair.
fn push_pair_cases(
    cases: &mut Vec<BenchCase>,
    used_ids: &mut HashSet<String>,
    a: &PairSide,
    b: &PairSide,
    tier: &str,
    slippage_bps: u64,
) {
    let slug_a = mint_slug(&a.mint);
    let slug_b = mint_slug(&b.mint);
    for &usd in USD_LADDER.iter() {
        // fwd: a -> b
        if let Some(amount) = raw_amount(usd, a.usd_per_whole, a.decimals) {
            let id = unique_id(used_ids, &slug_a, &slug_b, usd, "fwd");
            cases.push(BenchCase {
                id,
                input_mint: a.mint.clone(),
                output_mint: b.mint.clone(),
                amount,
                usd_size: usd,
                liquidity_tier: tier.into(),
                slippage_bps,
            });
        }
        // rev: b -> a
        if let Some(amount) = raw_amount(usd, b.usd_per_whole, b.decimals) {
            let id = unique_id(used_ids, &slug_a, &slug_b, usd, "rev");
            cases.push(BenchCase {
                id,
                input_mint: b.mint.clone(),
                output_mint: a.mint.clone(),
                amount,
                usd_size: usd,
                liquidity_tier: tier.into(),
                slippage_bps,
            });
        }
    }
}

fn unique_id(
    used: &mut HashSet<String>,
    slug_a: &str,
    slug_b: &str,
    usd: f64,
    dir: &str,
) -> String {
    let base = format!("{slug_a}-{slug_b}-{}usd-{dir}", format_usd(usd));
    let mut id = base.clone();
    let mut n = 2;
    while !used.insert(id.clone()) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Read a cases file.
pub fn load_cases(path: &Path) -> Result<Vec<BenchCase>, GenericError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read cases file '{}': {e}", path.display()))?;
    let cases: Vec<BenchCase> = serde_json::from_str(&raw)
        .map_err(|e| format!("failed to parse cases file '{}': {e}", path.display()))?;
    Ok(cases)
}

/// Write a cases file (pretty JSON), creating parent dirs.
pub fn save_cases(cases: &[BenchCase], path: &Path) -> Result<(), GenericError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(cases)?;
    std::fs::write(path, json + "\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usd_formatting() {
        assert_eq!(format_usd(0.1), "0.1");
        assert_eq!(format_usd(1.0), "1");
        assert_eq!(format_usd(10.0), "10");
        assert_eq!(format_usd(100_000.0), "100000");
    }

    #[test]
    fn slugs() {
        assert_eq!(mint_slug(WSOL), "sol");
        assert_eq!(mint_slug(USDC), "usdc");
        assert_eq!(mint_slug(USDT), "usdt");
        assert_eq!(mint_slug("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263"), "DezXAZ8z");
    }

    #[test]
    fn raw_amount_conversion() {
        // $100 of SOL at $160: 0.625 SOL = 625_000_000 lamports.
        assert_eq!(raw_amount(100.0, 160.0, 9), Some(625_000_000));
        // $100 of USDC at $1: 100_000_000 raw.
        assert_eq!(raw_amount(100.0, 1.0, 6), Some(100_000_000));
        // Sub-unit results are rejected, not rounded to zero.
        assert_eq!(raw_amount(0.1, 1e12, 6), None);
        // Bad prices are rejected.
        assert_eq!(raw_amount(100.0, 0.0, 6), None);
        assert_eq!(raw_amount(100.0, f64::NAN, 6), None);
        // Overflow is rejected.
        assert_eq!(raw_amount(1e20, 1e-9, 9), None);
    }

    #[test]
    fn tier_assignment() {
        assert_eq!(tier_for_usd(5_000_000.0), Some("major"));
        assert_eq!(tier_for_usd(1_000_000.0), Some("major"));
        assert_eq!(tier_for_usd(500_000.0), Some("mid"));
        assert_eq!(tier_for_usd(100_000.0), Some("mid"));
        assert_eq!(tier_for_usd(50_000.0), Some("tail"));
        assert_eq!(tier_for_usd(10_000.0), Some("tail"));
        assert_eq!(tier_for_usd(9_999.0), None);
    }

    #[test]
    fn even_sampling_is_deterministic_and_spread() {
        let items: Vec<u32> = (0..100).collect();
        let s = sample_evenly(&items, 5);
        assert_eq!(s, vec![&0, &20, &40, &60, &80]);
        let small: Vec<u32> = (0..3).collect();
        assert_eq!(sample_evenly(&small, 5).len(), 3);
        assert!(sample_evenly(&items, 0).is_empty());
    }

    #[test]
    fn ids_are_unique() {
        let mut used = HashSet::new();
        let a = unique_id(&mut used, "sol", "usdc", 100.0, "fwd");
        let b = unique_id(&mut used, "sol", "usdc", 100.0, "fwd");
        assert_eq!(a, "sol-usdc-100usd-fwd");
        assert_eq!(b, "sol-usdc-100usd-fwd-2");
    }

    #[test]
    fn cases_file_round_trip() {
        let dir = std::env::temp_dir().join("thunder-bench-test-cases");
        let path = dir.join("cases.json");
        let cases = vec![BenchCase {
            id: "sol-usdc-100usd-fwd".into(),
            input_mint: WSOL.into(),
            output_mint: USDC.into(),
            amount: 625_000_000,
            usd_size: 100.0,
            liquidity_tier: "major".into(),
            slippage_bps: 50,
        }];
        save_cases(&cases, &path).unwrap();
        let loaded = load_cases(&path).unwrap();
        assert_eq!(loaded, cases);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
