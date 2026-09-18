//! Split routing: allocate one large swap across multiple pool-disjoint routes.
//!
//! With exact (concave) per-pool curves, splitting a large order across
//! several independent routes reduces total price impact. The algorithm:
//!
//! 1. Candidate paths from `find_routes` at the full amount AND at a quarter
//!    of it (small-size probing surfaces routes that are good at fractional
//!    size but starved at full size), deduped by pool-address sequence.
//! 2. Greedy pool-disjoint selection (the best full-size route is always
//!    kept; later candidates are admitted only if they share no pool with
//!    already-admitted ones).
//! 3. Greedy marginal-output allocation of K equal buckets (last bucket
//!    takes the remainder). Concave curves make greedy optimal for the
//!    discretized problem. Simulations are memoized on (path, cumulative),
//!    so each round costs ~1 fresh path simulation.
//! 4. Dust paths (below `min_path_bps` of the input or one bucket) are
//!    folded into the best path.
//! 5. Every surviving path is re-simulated exactly at its final allocation,
//!    producing real per-hop amounts.
//!
//! A split that does not beat the best single route collapses to one path —
//! callers never see a worse-than-single split. Deterministic: no
//! randomness, ties always resolve to the lowest candidate index.

use std::collections::{HashMap, HashSet};

use solana_pubkey::Pubkey;
use thunder_core::{AccountDataProvider, GenericError};

use crate::pool_index::PoolIndex;
use crate::router::{simulate_path, simulate_path_output, Router};
use crate::types::{Route, SplitPath, SplitQuote};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

pub struct SplitConfig {
    /// Maximum number of concurrently funded paths (tx account budget).
    pub max_paths: usize,
    /// Allocation granularity: the input is divided into this many buckets.
    pub buckets: usize,
    /// Paths allocated less than this share of the input (bps) are folded
    /// into the best path.
    pub min_path_bps: u16,
    /// Maximum candidate routes considered for the disjoint set.
    pub candidates: usize,
}

impl Default for SplitConfig {
    fn default() -> Self {
        Self {
            max_paths: 3,
            buckets: 20,
            min_path_bps: 500,
            candidates: 12,
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Hop list in the shape the router's simulation helpers consume.
type PathHops = Vec<(String, Pubkey, Pubkey)>;

fn route_hops(route: &Route) -> PathHops {
    route
        .hops
        .iter()
        .map(|h| (h.pool_address.clone(), h.input_mint, h.output_mint))
        .collect()
}

/// Dedup key: the pool-address sequence of a route.
fn seq_key(route: &Route) -> String {
    route
        .hops
        .iter()
        .map(|h| h.pool_address.as_str())
        .collect::<Vec<_>>()
        .join("|")
}

/// Greedily select pool-disjoint candidates, preserving order (candidate 0 —
/// the best full-size route — is always admitted first). A candidate is
/// admitted only if it shares no pool with already-admitted ones.
fn select_pool_disjoint(candidates: &[PathHops]) -> Vec<usize> {
    let mut used: HashSet<&str> = HashSet::new();
    let mut selected = Vec::new();
    for (i, hops) in candidates.iter().enumerate() {
        if hops.iter().any(|(addr, _, _)| used.contains(addr.as_str())) {
            continue;
        }
        for (addr, _, _) in hops {
            used.insert(addr.as_str());
        }
        selected.push(i);
    }
    selected
}

/// Simulation context shared by the allocation loop and dust folding.
struct SimCtx<'a> {
    index: &'a PoolIndex,
    swappable: Option<&'a HashSet<String>>,
    live: Option<&'a dyn AccountDataProvider>,
    /// (path index, cumulative amount) -> simulated output. `None` = the
    /// path is unquotable at that size (curve exhausted / missing arrays).
    memo: HashMap<(usize, u64), Option<u64>>,
}

impl<'a> SimCtx<'a> {
    /// Memoized end-to-end output of `paths[p]` at cumulative `amount`.
    fn path_out(&mut self, paths: &[PathHops], p: usize, amount: u64) -> Option<u64> {
        *self.memo.entry((p, amount)).or_insert_with(|| {
            simulate_path_output(self.index, &paths[p], amount, self.swappable, self.live)
        })
    }
}

// ---------------------------------------------------------------------------
// Router entry point
// ---------------------------------------------------------------------------

impl<'a> Router<'a> {
    /// Find a split route for `amount_in`, allocating across up to
    /// `cfg.max_paths` pool-disjoint paths by greedy marginal output.
    ///
    /// Returns an empty `paths` vec when no route exists at all. Otherwise
    /// `paths` is ordered by allocated amount descending, amounts sum to
    /// `amount_in` exactly, percents sum to 10_000, and `total_output >=
    /// single_path_output` (collapses to the best single route when
    /// splitting does not help).
    pub fn find_split_routes(
        &self,
        input_mint: Pubkey,
        output_mint: Pubkey,
        amount_in: u64,
        cfg: &SplitConfig,
    ) -> Result<SplitQuote, GenericError> {
        let max_candidates = cfg.candidates.max(1);
        // Phase timing, printed when THUNDER_SPLIT_PROFILE is set.
        let profile = std::env::var_os("THUNDER_SPLIT_PROFILE").is_some();
        let t0 = std::time::Instant::now();
        let mut t_last = t0;
        let mut phase = |name: &str, t_last: &mut std::time::Instant| {
            if profile {
                eprintln!("[splitter] {name}: {:.2}ms", t_last.elapsed().as_secs_f64() * 1e3);
                *t_last = std::time::Instant::now();
            }
        };

        // --- (a) candidates at full size and quarter size ------------------
        // The two discovery passes are independent read-only computations
        // over the index/store, so run them concurrently: the quarter-size
        // probe then costs ~zero wall time on top of the full-size pass.
        let quarter = amount_in / 4;
        let (full, quarter_quote) = if quarter > 0 {
            std::thread::scope(|s| {
                let qh =
                    s.spawn(|| self.find_routes(input_mint, output_mint, quarter, max_candidates));
                let full = self.find_routes(input_mint, output_mint, amount_in, max_candidates);
                let q = qh.join().expect("quarter-probe thread panicked");
                (full, q)
            })
        } else {
            (
                self.find_routes(input_mint, output_mint, amount_in, max_candidates),
                Ok(crate::types::Quote { routes: vec![] }),
            )
        };
        let full = full?;
        let quarter_routes = quarter_quote?.routes;
        phase("discovery (full ∥ quarter find_routes)", &mut t_last);

        let Some(best) = full.routes.first().cloned() else {
            return Ok(SplitQuote {
                input_amount: amount_in,
                total_output: 0,
                paths: vec![],
                single_path_output: 0,
            });
        };
        let single_path_output = best.output_amount;
        let single_path_fallback = |best: Route| SplitQuote {
            input_amount: amount_in,
            total_output: best.output_amount,
            single_path_output: best.output_amount,
            paths: vec![SplitPath {
                amount_in,
                percent_bps: 10_000,
                route: best,
            }],
        };

        let swappable = self.swappable_set.as_deref();
        let mut ctx = SimCtx {
            index: self.index,
            swappable,
            live: self.live_data,
            memo: HashMap::new(),
        };

        // Merge + dedup by pool sequence. The best full-size route is pinned
        // at index 0; the rest are ranked by probe output (output at quarter
        // size — the size splitting actually executes fractions at).
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(seq_key(&best));
        let mut ranked: Vec<(PathHops, u64)> = Vec::new();
        for r in &quarter_routes {
            if seen.insert(seq_key(r)) {
                ranked.push((route_hops(r), r.output_amount));
            }
        }
        for r in &full.routes {
            if seen.insert(seq_key(r)) {
                let hops = route_hops(r);
                let probe = if quarter > 0 {
                    simulate_path_output(self.index, &hops, quarter, swappable, self.live_data)
                        .unwrap_or(0)
                } else {
                    r.output_amount
                };
                ranked.push((hops, probe));
            }
        }
        // Stable sort: probe output desc, first-seen order breaks ties.
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        let mut candidates: Vec<PathHops> = Vec::with_capacity(1 + ranked.len());
        candidates.push(route_hops(&best));
        candidates.extend(ranked.into_iter().map(|(hops, _)| hops));
        candidates.truncate(max_candidates);

        phase("candidate probe + rank", &mut t_last);

        // --- (b) greedy pool-disjoint selection -----------------------------
        let selected = select_pool_disjoint(&candidates);
        let paths: Vec<PathHops> = selected.into_iter().map(|i| candidates[i].clone()).collect();
        let n = paths.len();
        if n <= 1 {
            return Ok(single_path_fallback(best));
        }

        // --- (c) greedy marginal-output bucket allocation --------------------
        let k = cfg.buckets.max(1);
        let bucket = amount_in / k as u64;
        if bucket == 0 {
            // Amount too small to discretize — not worth splitting.
            return Ok(single_path_fallback(best));
        }

        let mut alloc = vec![0u64; n];
        let mut outs = vec![0u64; n];
        let mut remaining = amount_in;
        for round in 0..k {
            let inc = if round == k - 1 { remaining } else { bucket };
            if inc == 0 {
                continue;
            }
            let funded = alloc.iter().filter(|a| **a > 0).count();
            // argmax gain; ties -> lowest candidate index (strict `>`).
            let mut winner: Option<(usize, u64, u64)> = None; // (idx, gain, new_out)
            for p in 0..n {
                if alloc[p] == 0 && funded >= cfg.max_paths {
                    continue;
                }
                // Err ("curve exhausted") -> path ineligible for this bucket.
                let Some(new_out) = ctx.path_out(&paths, p, alloc[p] + inc) else {
                    continue;
                };
                let gain = new_out.saturating_sub(outs[p]);
                if winner.as_ref().is_none_or(|(_, g, _)| gain > *g) {
                    winner = Some((p, gain, new_out));
                }
            }
            match winner {
                Some((p, _, new_out)) => {
                    alloc[p] += inc;
                    outs[p] = new_out;
                    remaining -= inc;
                }
                // No path can absorb another bucket; leftover handled below.
                None => break,
            }
        }

        // Fold any unallocated leftover into the path that can absorb it
        // (by current output desc, lowest index ties).
        if remaining > 0 {
            let mut targets: Vec<usize> = (0..n).filter(|&p| alloc[p] > 0).collect();
            targets.sort_by(|&a, &b| outs[b].cmp(&outs[a]).then(a.cmp(&b)));
            let mut folded = false;
            for t in targets {
                if let Some(new_out) = ctx.path_out(&paths, t, alloc[t] + remaining) {
                    alloc[t] += remaining;
                    outs[t] = new_out;
                    folded = true;
                    break;
                }
            }
            if !folded {
                // Nothing can absorb the full amount piecewise — the best
                // single route (already simulated at amount_in) always can.
                return Ok(single_path_fallback(best));
            }
        }

        // --- (d) dust fold ----------------------------------------------------
        let threshold =
            bucket.max((amount_in as u128 * cfg.min_path_bps as u128 / 10_000) as u64);
        loop {
            // Smallest dust path first (lowest alloc, ties -> lowest index).
            let dust = (0..n)
                .filter(|&p| alloc[p] > 0 && alloc[p] < threshold)
                .min_by_key(|&p| (alloc[p], p));
            let Some(d) = dust else { break };
            let mut targets: Vec<usize> = (0..n).filter(|&p| p != d && alloc[p] > 0).collect();
            targets.sort_by(|&a, &b| outs[b].cmp(&outs[a]).then(a.cmp(&b)));
            let mut folded = false;
            for t in targets {
                if let Some(new_out) = ctx.path_out(&paths, t, alloc[t] + alloc[d]) {
                    alloc[t] += alloc[d];
                    outs[t] = new_out;
                    alloc[d] = 0;
                    outs[d] = 0;
                    folded = true;
                    break;
                }
            }
            if !folded {
                // No other path can absorb this dust; keep it rather than
                // lose input (amounts must sum to amount_in exactly).
                break;
            }
        }

        phase("allocation + dust fold", &mut t_last);

        // --- (e) final re-simulation + assembly -------------------------------
        // Order paths by allocated amount desc (stable: lowest index ties).
        let mut order: Vec<usize> = (0..n).filter(|&p| alloc[p] > 0).collect();
        order.sort_by(|&a, &b| alloc[b].cmp(&alloc[a]).then(a.cmp(&b)));

        let mut split_paths: Vec<SplitPath> = Vec::with_capacity(order.len());
        let mut total_output: u64 = 0;
        let mut pct_sum: u16 = 0;
        for (i, &p) in order.iter().enumerate() {
            let Some(route) = simulate_path(self.index, &paths[p], alloc[p], swappable, self.live_data)
            else {
                // A path that simulated fine during allocation failed the
                // final pass — bail out to the known-good single route.
                return Ok(single_path_fallback(best));
            };
            let percent_bps = if i == order.len() - 1 {
                10_000 - pct_sum // last path takes the rounding remainder
            } else {
                (alloc[p] as u128 * 10_000 / amount_in as u128) as u16
            };
            pct_sum += percent_bps;
            total_output = total_output.saturating_add(route.output_amount);
            split_paths.push(SplitPath {
                amount_in: alloc[p],
                percent_bps,
                route,
            });
        }

        // Re-validate against a fresh single-path quote. Live balances can
        // shift during the split's multi-simulation window, so compare the
        // split total against a single route priced *now* and never return the
        // worse of the two (guards against transient bad allocations under a
        // live update stream).
        let fresh_single = self
            .find_routes(input_mint, output_mint, amount_in, 1)
            .ok()
            .and_then(|q| q.routes.into_iter().next());
        let fresh_output = fresh_single.as_ref().map_or(0, |r| r.output_amount);
        if split_paths.len() <= 1 || total_output <= single_path_output.max(fresh_output) {
            return Ok(match fresh_single {
                Some(fresh) if fresh.output_amount > best.output_amount => {
                    single_path_fallback(fresh)
                }
                _ => single_path_fallback(best),
            });
        }

        phase("final re-simulation", &mut t_last);
        if profile {
            eprintln!("[splitter] total: {:.2}ms", t0.elapsed().as_secs_f64() * 1e3);
        }

        Ok(SplitQuote {
            input_amount: amount_in,
            total_output,
            paths: split_paths,
            single_path_output,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests: synthetic constant-product pools, no RPC
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PoolEntry;
    use thunder_core::{
        constant_product_swap, Market, PoolFees, PoolFinancials, PoolMetadata, SwapDirection,
    };

    const FEE_BPS: u64 = 30;

    /// Minimal in-memory constant-product Market. Reserves are served through
    /// the live-data path (the same one production quoting uses).
    struct CpMarket {
        address: String,
        quote_mint: Pubkey,
        base_mint: Pubkey,
        quote_vault: Pubkey,
        base_vault: Pubkey,
        quote_reserve: u64,
        base_reserve: u64,
    }

    impl Market for CpMarket {
        fn metadata(&self) -> Result<PoolMetadata, GenericError> {
            Ok(PoolMetadata {
                address: self.address.clone(),
                dex_name: "TestCP".into(),
                quote_mint: self.quote_mint,
                base_mint: self.base_mint,
                quote_vault: self.quote_vault,
                base_vault: self.base_vault,
                fees: PoolFees {
                    trade_fee_bps: FEE_BPS,
                    protocol_fee_bps: None,
                },
            })
        }

        fn financials(&self) -> Result<PoolFinancials, GenericError> {
            Ok(PoolFinancials {
                quote_balance: self.quote_reserve,
                base_balance: self.base_reserve,
                quote_decimals: 9,
                base_decimals: 9,
            })
        }

        fn calculate_output(
            &self,
            amount_in: u64,
            direction: SwapDirection,
        ) -> Result<u64, GenericError> {
            match direction {
                SwapDirection::Buy => {
                    constant_product_swap(self.quote_reserve, self.base_reserve, amount_in, FEE_BPS)
                }
                SwapDirection::Sell => {
                    constant_product_swap(self.base_reserve, self.quote_reserve, amount_in, FEE_BPS)
                }
            }
        }

        fn calculate_output_live(
            &self,
            amount_in: u64,
            direction: SwapDirection,
            _pool_data: Option<&[u8]>,
            quote_vault_balance: u64,
            base_vault_balance: u64,
        ) -> Result<u64, GenericError> {
            match direction {
                SwapDirection::Buy => constant_product_swap(
                    quote_vault_balance,
                    base_vault_balance,
                    amount_in,
                    FEE_BPS,
                ),
                SwapDirection::Sell => constant_product_swap(
                    base_vault_balance,
                    quote_vault_balance,
                    amount_in,
                    FEE_BPS,
                ),
            }
        }

        fn calculate_price_impact(
            &self,
            _amount_in: u64,
            _direction: SwapDirection,
        ) -> Result<u64, GenericError> {
            Ok(0)
        }

        fn current_price(&self) -> Result<f64, GenericError> {
            Ok(self.quote_reserve as f64 / self.base_reserve as f64)
        }
    }

    /// Hand-rolled AccountDataProvider stub: vault balances from a map, no
    /// pool account bytes.
    struct StubProvider {
        balances: HashMap<Pubkey, u64>,
    }

    impl AccountDataProvider for StubProvider {
        fn pool_account_data(&self, _pubkey: &Pubkey) -> Option<Vec<u8>> {
            None
        }

        fn token_balance(&self, vault_pubkey: &Pubkey) -> u64 {
            self.balances.get(vault_pubkey).copied().unwrap_or(0)
        }
    }

    /// Deterministic pubkey from a single byte tag.
    fn pk(tag: u8) -> Pubkey {
        Pubkey::new_from_array([tag; 32])
    }

    struct Harness {
        index: PoolIndex,
        provider: StubProvider,
        next_vault: u8,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                index: PoolIndex::new(),
                provider: StubProvider {
                    balances: HashMap::new(),
                },
                next_vault: 100,
            }
        }

        /// Add a constant-product pool between `quote_mint` and `base_mint`.
        fn add_pool(
            &mut self,
            pool_tag: u8,
            quote_mint: Pubkey,
            base_mint: Pubkey,
            quote_reserve: u64,
            base_reserve: u64,
        ) -> String {
            let pool_pubkey = pk(pool_tag);
            let address = pool_pubkey.to_string();
            let quote_vault = pk(self.next_vault);
            let base_vault = pk(self.next_vault + 1);
            self.next_vault += 2;
            self.provider.balances.insert(quote_vault, quote_reserve);
            self.provider.balances.insert(base_vault, base_reserve);
            let market = CpMarket {
                address: address.clone(),
                quote_mint,
                base_mint,
                quote_vault,
                base_vault,
                quote_reserve,
                base_reserve,
            };
            self.index
                .add_pool(
                    address.clone(),
                    PoolEntry {
                        market: Box::new(market),
                        dex_name: "TestCP".into(),
                        quote_mint,
                        base_mint,
                        pool_pubkey,
                        quote_vault,
                        base_vault,
                        cached_data: vec![],
                    },
                )
                .unwrap();
            address
        }
    }

    fn assert_invariants(sq: &SplitQuote) {
        let amount_sum: u64 = sq.paths.iter().map(|p| p.amount_in).sum();
        assert_eq!(amount_sum, sq.input_amount, "sum of path amounts != input");
        let pct_sum: u32 = sq.paths.iter().map(|p| p.percent_bps as u32).sum();
        assert_eq!(pct_sum, 10_000, "sum of percent_bps != 10000");
        let out_sum: u64 = sq.paths.iter().map(|p| p.route.output_amount).sum();
        assert_eq!(out_sum, sq.total_output, "total_output != sum of path outputs");
        assert!(
            sq.total_output >= sq.single_path_output,
            "split worse than single path"
        );
        for p in &sq.paths {
            assert_eq!(p.route.input_amount, p.amount_in, "route not simulated at alloc");
        }
        // Paths ordered by amount descending.
        for w in sq.paths.windows(2) {
            assert!(w[0].amount_in >= w[1].amount_in, "paths not ordered by amount");
        }
        // Pool-disjoint across paths.
        let mut used = HashSet::new();
        for p in &sq.paths {
            for h in &p.route.hops {
                assert!(used.insert(h.pool_address.clone()), "pool shared across paths");
            }
        }
    }

    const R: u64 = 1_000_000_000_000; // 1e12 base reserve unit

    // (a) Two identical CP pools: optimal split is 50/50; greedy must land
    //     within one bucket and beat the single path.
    #[test]
    fn two_equal_pools_split_50_50() {
        let mut h = Harness::new();
        let (a, b) = (pk(1), pk(2));
        h.add_pool(10, a, b, R, R);
        h.add_pool(11, a, b, R, R);

        let amount = R / 5; // 20% of one pool's reserve — heavy impact
        let cfg = SplitConfig::default();
        let router = Router::new(&h.index, 1).with_live_data(&h.provider);
        let sq = router.find_split_routes(a, b, amount, &cfg).unwrap();

        assert_invariants(&sq);
        assert_eq!(sq.paths.len(), 2, "expected a 2-way split");
        assert!(
            sq.total_output > sq.single_path_output,
            "split ({}) must beat single path ({})",
            sq.total_output,
            sq.single_path_output
        );
        let bucket = amount / cfg.buckets as u64;
        let diff = sq.paths[0].amount_in.abs_diff(sq.paths[1].amount_in);
        assert!(
            diff <= bucket,
            "expected ~50/50, got {} / {} (bucket {})",
            sq.paths[0].amount_in,
            sq.paths[1].amount_in,
            bucket
        );
    }

    // (b) 9:1 liquidity depth: allocation lands within one bucket of 90/10.
    #[test]
    fn nine_to_one_depth_splits_90_10() {
        let mut h = Harness::new();
        let (a, b) = (pk(1), pk(2));
        h.add_pool(10, a, b, 9 * R, 9 * R);
        h.add_pool(11, a, b, R, R);

        let amount = 2 * R / 5; // 4% of combined liquidity
        let cfg = SplitConfig::default();
        let router = Router::new(&h.index, 1).with_live_data(&h.provider);
        let sq = router.find_split_routes(a, b, amount, &cfg).unwrap();

        assert_invariants(&sq);
        assert_eq!(sq.paths.len(), 2);
        assert!(sq.total_output > sq.single_path_output);
        let share_bps = sq.paths[0].amount_in as u128 * 10_000 / amount as u128;
        let bucket_bps = 10_000 / cfg.buckets as u128;
        assert!(
            share_bps.abs_diff(9_000) <= bucket_bps,
            "expected ~90% on the deep pool, got {share_bps} bps"
        );
    }

    // (c) Disjoint selection: a candidate sharing a pool with an admitted one
    //     is rejected.
    #[test]
    fn pool_overlap_rejects_second_candidate() {
        let p1 = "P1".to_string();
        let p2 = "P2".to_string();
        let p3 = "P3".to_string();
        let p4 = "P4".to_string();
        let (a, b, c) = (pk(1), pk(2), pk(3));
        let candidates: Vec<PathHops> = vec![
            vec![(p1.clone(), a, b)],                       // best route
            vec![(p2.clone(), a, c), (p3.clone(), c, b)],   // admitted
            vec![(p2.clone(), a, c), (p4.clone(), c, b)],   // shares P2 -> rejected
        ];
        let selected = select_pool_disjoint(&candidates);
        assert_eq!(selected, vec![0, 1], "candidate sharing P2 must be rejected");
    }

    // (c-bis) End-to-end with a 2-hop path: direct pool + disjoint 2-hop
    //     route both get funded.
    #[test]
    fn multihop_split_end_to_end() {
        let mut h = Harness::new();
        let (a, b, c) = (pk(1), pk(2), pk(3));
        h.add_pool(10, a, b, R, R); // direct A-B
        h.add_pool(11, a, c, 10 * R, 10 * R); // deep A-C
        h.add_pool(12, c, b, 10 * R, 10 * R); // deep C-B

        let amount = R / 4; // 25% of the direct pool's reserve
        let cfg = SplitConfig::default();
        let router = Router::new(&h.index, 2).with_live_data(&h.provider);
        let sq = router.find_split_routes(a, b, amount, &cfg).unwrap();

        assert_invariants(&sq);
        assert_eq!(sq.paths.len(), 2, "expected direct + 2-hop split");
        assert!(sq.total_output > sq.single_path_output);
        let hop_counts: Vec<usize> = sq.paths.iter().map(|p| p.route.hops.len()).collect();
        assert!(hop_counts.contains(&1) && hop_counts.contains(&2));
    }

    // (d) Dust fold: a path allocated below min_path_bps is folded away.
    #[test]
    fn dust_path_folds_into_best() {
        let mut h = Harness::new();
        let (a, b) = (pk(1), pk(2));
        h.add_pool(10, a, b, 10 * R, 10 * R);
        h.add_pool(11, a, b, 10 * R, 10 * R);
        h.add_pool(12, a, b, R, R); // shallow: gets ~1 bucket (~5%)

        let amount = 4 * R; // ~19% of combined liquidity
        let router = Router::new(&h.index, 1).with_live_data(&h.provider);

        // With the default 500 bps floor the shallow path survives...
        let sq_default = router
            .find_split_routes(a, b, amount, &SplitConfig::default())
            .unwrap();
        assert_invariants(&sq_default);
        assert_eq!(sq_default.paths.len(), 3, "default cfg should fund all 3 pools");

        // ...with a 1000 bps floor it is dust and gets folded.
        let cfg = SplitConfig {
            min_path_bps: 1000,
            ..Default::default()
        };
        let sq = router.find_split_routes(a, b, amount, &cfg).unwrap();
        assert_invariants(&sq);
        assert_eq!(sq.paths.len(), 2, "dust path must be folded");
        assert!(sq.total_output > sq.single_path_output);
    }

    // (e) Single candidate: exactly one path == the best route, 100%.
    #[test]
    fn single_candidate_returns_one_path() {
        let mut h = Harness::new();
        let (a, b) = (pk(1), pk(2));
        let addr = h.add_pool(10, a, b, R, R);

        let amount = R / 10;
        let router = Router::new(&h.index, 1).with_live_data(&h.provider);
        let sq = router
            .find_split_routes(a, b, amount, &SplitConfig::default())
            .unwrap();

        assert_invariants(&sq);
        assert_eq!(sq.paths.len(), 1);
        assert_eq!(sq.paths[0].percent_bps, 10_000);
        assert_eq!(sq.paths[0].amount_in, amount);
        assert_eq!(sq.paths[0].route.hops.len(), 1);
        assert_eq!(sq.paths[0].route.hops[0].pool_address, addr);
        assert_eq!(sq.total_output, sq.single_path_output);

        // Matches the unsplit router's best route exactly.
        let unsplit = router.find_routes(a, b, amount, 1).unwrap();
        assert_eq!(
            sq.paths[0].route.output_amount,
            unsplit.best().unwrap().output_amount
        );
    }

    // (e-bis) No route at all: empty paths, zero outputs.
    #[test]
    fn no_route_returns_empty() {
        let h = Harness::new();
        let router = Router::new(&h.index, 2).with_live_data(&h.provider);
        let sq = router
            .find_split_routes(pk(1), pk(2), 1_000_000, &SplitConfig::default())
            .unwrap();
        assert!(sq.paths.is_empty());
        assert_eq!(sq.total_output, 0);
        assert_eq!(sq.single_path_output, 0);
    }
}
