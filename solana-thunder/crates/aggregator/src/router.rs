//! Multi-hop route discovery: finds optimal swap paths between any two tokens.
//!
//! Searches 1-hop through 4-hop routes using hub mints and bidirectional
//! neighbor exploration. All candidate paths are simulated end-to-end with
//! the actual input amount, then ranked by output amount descending.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use solana_pubkey::Pubkey;
use thunder_core::{AccountDataProvider, GenericError, SwapDirection, JITOSOL, MSOL, USDC, USDT, WSOL};

use crate::pool_index::PoolIndex;
use crate::types::{Quote, Route, RouteHop};

/// Hub mints — high-liquidity tokens used as intermediate routing nodes.
const HUB_MINTS: [&str; 5] = [WSOL, USDC, USDT, JITOSOL, MSOL];

/// Smaller hub set for higher-hop routes to bound search space.
const HUB_MINTS_CORE: [&str; 3] = [WSOL, USDC, USDT];

/// Max neighbors explored per side in bidirectional search.
const MAX_NEIGHBOR_CANDIDATES: usize = 50;

/// Mints with more incident pools than this are treated as hubs: their full
/// adjacency is enormous, and hub paths are already covered by explicit hub
/// routing. Env: `ROUTER_HUB_DEGREE_SKIP` (raise to disable the skip).
static HUB_DEGREE_SKIP: LazyLock<usize> =
    LazyLock::new(|| env_num("ROUTER_HUB_DEGREE_SKIP", 2000));

/// Minimum vault balance (raw units) for a pool to be routable.
const MIN_VAULT_BALANCE: u64 = 10_000_000; // 0.01 SOL

/// Routes whose measured price impact exceeds this are dropped: thin or
/// drained paths (often scam/honeypot pools) no caller should execute.
/// Env: `ROUTER_MAX_IMPACT_BPS` (raise to disable the filter).
static MAX_ROUTE_IMPACT_BPS: LazyLock<u64> =
    LazyLock::new(|| env_num("ROUTER_MAX_IMPACT_BPS", 9000));

/// Max pools simulated per pair-leg during routing (deepest by vault balance).
/// Env: `ROUTER_MAX_LEG_POOLS` (raise to disable the cap).
static MAX_LEG_POOLS: LazyLock<usize> =
    LazyLock::new(|| env_num("ROUTER_MAX_LEG_POOLS", 24));

/// Parse an env var into a number, falling back to `default`.
fn env_num<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

pub struct Router<'a> {
    pub(crate) index: &'a PoolIndex,
    max_hops: usize,
    pub(crate) swappable_set: Option<Arc<HashSet<String>>>,
    pub(crate) live_data: Option<&'a dyn AccountDataProvider>,
}

impl<'a> Router<'a> {
    pub fn new(index: &'a PoolIndex, max_hops: usize) -> Self {
        Self {
            index,
            max_hops,
            swappable_set: None,
            live_data: None,
        }
    }

    /// Restrict routing to only the given pool addresses.
    pub fn with_swappable_set(mut self, set: Arc<HashSet<String>>) -> Self {
        self.swappable_set = Some(set);
        self
    }

    /// Provide live on-chain data for routing calculations.
    pub fn with_live_data(mut self, provider: &'a dyn AccountDataProvider) -> Self {
        self.live_data = Some(provider);
        self
    }

    /// Find the best routes from `input_mint` to `output_mint` for `amount_in`.
    ///
    /// Returns up to `max_routes` routes sorted by output amount descending.
    pub fn find_routes(
        &self,
        input_mint: Pubkey,
        output_mint: Pubkey,
        amount_in: u64,
        max_routes: usize,
    ) -> Result<Quote, GenericError> {
        if input_mint == output_mint || amount_in == 0 {
            return Ok(Quote { routes: vec![] });
        }

        let mut candidates: Vec<Route> = Vec::new();

        let hubs: Vec<Pubkey> = HUB_MINTS
            .iter()
            .map(|s| Pubkey::from_str_const(s))
            .filter(|h| *h != input_mint && *h != output_mint)
            .collect();

        let core_hubs: Vec<Pubkey> = HUB_MINTS_CORE
            .iter()
            .map(|s| Pubkey::from_str_const(s))
            .filter(|h| *h != input_mint && *h != output_mint)
            .collect();

        // === 1-hop: direct pools ===
        if self.max_hops >= 1 {
            self.find_direct(&input_mint, &output_mint, amount_in, &mut candidates);
        }

        // === 2-hop ===
        if self.max_hops >= 2 {
            // Via hub mints
            for hub in &hubs {
                self.try_2hop(&input_mint, hub, &output_mint, amount_in, &mut candidates);
            }

            // Via neighbors of input_mint (forward search)
            self.neighbor_2hop_forward(
                &input_mint,
                &output_mint,
                amount_in,
                &hubs,
                &mut candidates,
            );

            // Via neighbors of output_mint (reverse search)
            self.neighbor_2hop_reverse(
                &input_mint,
                &output_mint,
                amount_in,
                &hubs,
                &mut candidates,
            );
        }

        // === 3-hop ===
        if self.max_hops >= 3 {
            // Hub-hub: input → hub1 → hub2 → output
            for (i, h1) in core_hubs.iter().enumerate() {
                for h2 in &core_hubs[i + 1..] {
                    self.try_3hop(&input_mint, h1, h2, &output_mint, amount_in, &mut candidates);
                    self.try_3hop(&input_mint, h2, h1, &output_mint, amount_in, &mut candidates);
                }
            }

            // Neighbor-hub: input → neighbor → hub → output
            // and: input → hub → neighbor → output
            self.neighbor_3hop(&input_mint, &output_mint, amount_in, &core_hubs, &mut candidates);
        }

        // === 4-hop ===
        if self.max_hops >= 4 {
            // input → neighbor_in → hub → neighbor_out → output
            self.neighbor_4hop(&input_mint, &output_mint, amount_in, &core_hubs, &mut candidates);
        }

        // Drop routes with implausible price impact (thin/drained/honeypot
        // pools). Prevents returning catastrophic quotes a bot could execute.
        candidates.retain(|r| r.price_impact_bps <= *MAX_ROUTE_IMPACT_BPS);

        // Sort by output amount descending, truncate to max_routes.
        candidates.sort_unstable_by(|a, b| b.output_amount.cmp(&a.output_amount));
        candidates.truncate(max_routes);

        Ok(Quote { routes: candidates })
    }

    /// Cheap depth proxy for a pool: the smaller of its two vault balances
    /// (live if a provider is set, else cached financials). Higher = deeper.
    fn pool_liquidity(&self, addr: &str) -> u64 {
        let Some(e) = self.index.get_pool(addr) else { return 0 };
        if let Some(live) = self.live_data {
            live.token_balance(&e.quote_vault)
                .min(live.token_balance(&e.base_vault))
        } else if let Ok(f) = e.market.financials() {
            f.quote_balance.min(f.base_balance)
        } else {
            0
        }
    }

    /// Keep only the `limit` deepest pools of a pair. Ranking by a cheap
    /// vault-balance read avoids simulating thousands of dust pools on hub
    /// pairs, which dominated quote latency.
    fn cap_by_liquidity(&self, mut addrs: Vec<String>, limit: usize) -> Vec<String> {
        if addrs.len() <= limit {
            return addrs;
        }
        addrs.sort_unstable_by_key(|a| std::cmp::Reverse(self.pool_liquidity(a)));
        addrs.truncate(limit);
        addrs
    }

    // =====================================================================
    // Search strategies
    // =====================================================================

    /// All direct (1-hop) routes.
    fn find_direct(
        &self,
        input: &Pubkey,
        output: &Pubkey,
        amount_in: u64,
        out: &mut Vec<Route>,
    ) {
        for addr in self.cap_by_liquidity(self.index.direct_pools(input, output), *MAX_LEG_POOLS) {
            if let Some(route) = simulate_path(self.index, &[(addr, *input, *output)], amount_in, self.swappable_set.as_deref(), self.live_data) {
                out.push(route);
            }
        }
    }

    /// 2-hop through a specific intermediate mint. Picks best pool per leg.
    fn try_2hop(
        &self,
        input: &Pubkey,
        mid: &Pubkey,
        output: &Pubkey,
        amount_in: u64,
        out: &mut Vec<Route>,
    ) {
        let leg1 = self.cap_by_liquidity(self.index.direct_pools(input, mid), *MAX_LEG_POOLS);
        let leg2 = self.cap_by_liquidity(self.index.direct_pools(mid, output), *MAX_LEG_POOLS);
        if leg1.is_empty() || leg2.is_empty() {
            return;
        }

        // Try top N pools per leg to find viable routes (not just the single best).
        let top_leg1 = top_pools(self.index, &leg1, *input, amount_in, 3, self.swappable_set.as_deref(), self.live_data);
        for (a1, mid_amount) in &top_leg1 {
            let top_leg2 = top_pools(self.index, &leg2, *mid, *mid_amount, 3, self.swappable_set.as_deref(), self.live_data);
            for (a2, _) in &top_leg2 {
                if let Some(route) = simulate_path(self.index, &[(a1.clone(), *input, *mid), (a2.clone(), *mid, *output)],
                amount_in, self.swappable_set.as_deref(), self.live_data) {
                    out.push(route);
                }
            }
        }
    }

    /// 2-hop: explore neighbors of input_mint (forward search).
    fn neighbor_2hop_forward(
        &self,
        input: &Pubkey,
        output: &Pubkey,
        amount_in: u64,
        skip: &[Pubkey],
        out: &mut Vec<Route>,
    ) {
        // Hub inputs have millions of neighbors; hub routing already covers
        // those paths. Skip to avoid cloning/scanning the whole adjacency.
        if self.index.neighbor_count(input) > *HUB_DEGREE_SKIP {
            return;
        }
        let skip_set: HashSet<Pubkey> = skip.iter().copied().collect();
        let mut tried = 0usize;

        for (mid, pool_addr) in self.index.neighbors_iter(input) {
            if tried >= MAX_NEIGHBOR_CANDIDATES {
                break;
            }
            if mid == *input || mid == *output || skip_set.contains(&mid) {
                continue;
            }

            let leg2 = self.index.direct_pools(&mid, output);
            if leg2.is_empty() {
                tried += 1;
                continue;
            }

            let Some(hop1) = simulate_hop(self.index, pool_addr, *input, amount_in, self.swappable_set.as_deref(), self.live_data, false) else {
                tried += 1;
                continue;
            };
            if hop1.output_amount == 0 {
                tried += 1;
                continue;
            }

            let Some((a2, _)) = best_pool(self.index, &leg2, mid, hop1.output_amount, self.swappable_set.as_deref(), self.live_data) else {
                tried += 1;
                continue;
            };

            if let Some(route) = simulate_path(self.index, &[(pool_addr.to_string(), *input, mid), (a2, mid, *output)],
            amount_in, self.swappable_set.as_deref(), self.live_data) {
                out.push(route);
            }

            tried += 1;
        }
    }

    /// 2-hop: explore neighbors of output_mint (reverse search).
    /// For each neighbor `mid` of output, check if input → mid has a pool.
    fn neighbor_2hop_reverse(
        &self,
        input: &Pubkey,
        output: &Pubkey,
        amount_in: u64,
        skip: &[Pubkey],
        out: &mut Vec<Route>,
    ) {
        // Hub outputs have millions of neighbors; skip (hub routing covers it).
        if self.index.neighbor_count(output) > *HUB_DEGREE_SKIP {
            return;
        }
        let skip_set: HashSet<Pubkey> = skip.iter().copied().collect();
        let mut tried = 0usize;

        for (mid, _pool_to_output) in self.index.neighbors_iter(output) {
            if tried >= MAX_NEIGHBOR_CANDIDATES {
                break;
            }
            if mid == *input || mid == *output || skip_set.contains(&mid) {
                continue;
            }

            let leg1 = self.index.direct_pools(input, &mid);
            if leg1.is_empty() {
                tried += 1;
                continue;
            }

            // Simulate: input → mid (best pool) → output (best pool)
            let Some((a1, mid_amount)) = best_pool(self.index, &leg1, *input, amount_in, self.swappable_set.as_deref(), self.live_data) else {
                tried += 1;
                continue;
            };

            let leg2 = self.index.direct_pools(&mid, output);
            let Some((a2, _)) = best_pool(self.index, &leg2, mid, mid_amount, self.swappable_set.as_deref(), self.live_data) else {
                tried += 1;
                continue;
            };

            if let Some(route) = simulate_path(self.index, &[(a1, *input, mid), (a2, mid, *output)],
            amount_in, self.swappable_set.as_deref(), self.live_data) {
                out.push(route);
            }

            tried += 1;
        }
    }

    /// 3-hop through two specific intermediates.
    fn try_3hop(
        &self,
        input: &Pubkey,
        h1: &Pubkey,
        h2: &Pubkey,
        output: &Pubkey,
        amount_in: u64,
        out: &mut Vec<Route>,
    ) {
        let l1 = self.cap_by_liquidity(self.index.direct_pools(input, h1), *MAX_LEG_POOLS);
        let l2 = self.cap_by_liquidity(self.index.direct_pools(h1, h2), *MAX_LEG_POOLS);
        let l3 = self.cap_by_liquidity(self.index.direct_pools(h2, output), *MAX_LEG_POOLS);
        if l1.is_empty() || l2.is_empty() || l3.is_empty() {
            return;
        }

        let Some((a1, amt1)) = best_pool(self.index, &l1, *input, amount_in, self.swappable_set.as_deref(), self.live_data) else { return };
        let Some((a2, amt2)) = best_pool(self.index, &l2, *h1, amt1, self.swappable_set.as_deref(), self.live_data) else { return };
        let Some((a3, _)) = best_pool(self.index, &l3, *h2, amt2, self.swappable_set.as_deref(), self.live_data) else { return };

        if let Some(route) = simulate_path(self.index, &[(a1, *input, *h1), (a2, *h1, *h2), (a3, *h2, *output)],
        amount_in, self.swappable_set.as_deref(), self.live_data) {
            out.push(route);
        }
    }

    /// 3-hop via neighbor + hub.
    /// Tries: input → neighbor → hub → output  AND  input → hub → neighbor → output.
    fn neighbor_3hop(
        &self,
        input: &Pubkey,
        output: &Pubkey,
        amount_in: u64,
        hubs: &[Pubkey],
        out: &mut Vec<Route>,
    ) {
        // Forward: input → neighbor_of_input → hub → output
        let mut tried = 0usize;
        if self.index.neighbor_count(input) <= *HUB_DEGREE_SKIP {
            for (mid, _) in self.index.neighbors_iter(input) {
                if tried >= MAX_NEIGHBOR_CANDIDATES / 2 {
                    break;
                }
                if mid == *input || mid == *output || hubs.contains(&mid) {
                    continue;
                }
                for hub in hubs {
                    self.try_3hop(input, &mid, hub, output, amount_in, out);
                }
                tried += 1;
            }
        }

        // Reverse: input → hub → neighbor_of_output → output
        tried = 0;
        if self.index.neighbor_count(output) <= *HUB_DEGREE_SKIP {
            for (mid, _) in self.index.neighbors_iter(output) {
                if tried >= MAX_NEIGHBOR_CANDIDATES / 2 {
                    break;
                }
                if mid == *input || mid == *output || hubs.contains(&mid) {
                    continue;
                }
                for hub in hubs {
                    self.try_3hop(input, hub, &mid, output, amount_in, out);
                }
                tried += 1;
            }
        }
    }

    /// 4-hop: input → neighbor_in → hub → neighbor_out → output.
    /// Meets in the middle at a hub mint.
    fn neighbor_4hop(
        &self,
        input: &Pubkey,
        output: &Pubkey,
        amount_in: u64,
        hubs: &[Pubkey],
        out: &mut Vec<Route>,
    ) {
        // Skip when either endpoint is a hub (adjacency too large).
        if self.index.neighbor_count(input) > *HUB_DEGREE_SKIP
            || self.index.neighbor_count(output) > *HUB_DEGREE_SKIP
        {
            return;
        }
        // Collect neighbors of input that connect to any hub.
        let in_neighbors: Vec<(Pubkey, String)> = self
            .index
            .neighbors_iter(input)
            .filter(|(mid, _)| *mid != *input && *mid != *output && !hubs.contains(mid))
            .take(MAX_NEIGHBOR_CANDIDATES / 4)
            .map(|(m, a)| (m, a.to_string()))
            .collect();

        // Collect neighbors of output that connect to any hub.
        let out_neighbors: Vec<(Pubkey, String)> = self
            .index
            .neighbors_iter(output)
            .filter(|(mid, _)| *mid != *input && *mid != *output && !hubs.contains(mid))
            .take(MAX_NEIGHBOR_CANDIDATES / 4)
            .map(|(m, a)| (m, a.to_string()))
            .collect();

        for hub in hubs {
            for (n_in, _) in &in_neighbors {
                // Check n_in connects to hub
                if self.index.direct_pools(n_in, hub).is_empty() {
                    continue;
                }
                for (n_out, _) in &out_neighbors {
                    if n_in == n_out {
                        continue;
                    }
                    // Check hub connects to n_out
                    if self.index.direct_pools(hub, n_out).is_empty() {
                        continue;
                    }

                    // input → n_in → hub → n_out → output
                    let l1 = self.cap_by_liquidity(self.index.direct_pools(input, n_in), *MAX_LEG_POOLS);
                    let l2 = self.cap_by_liquidity(self.index.direct_pools(n_in, hub), *MAX_LEG_POOLS);
                    let l3 = self.cap_by_liquidity(self.index.direct_pools(hub, n_out), *MAX_LEG_POOLS);
                    let l4 = self.cap_by_liquidity(self.index.direct_pools(n_out, output), *MAX_LEG_POOLS);

                    if l1.is_empty() || l2.is_empty() || l3.is_empty() || l4.is_empty() {
                        continue;
                    }

                    let Some((a1, amt1)) = best_pool(self.index, &l1, *input, amount_in, self.swappable_set.as_deref(), self.live_data)
                    else {
                        continue;
                    };
                    let Some((a2, amt2)) = best_pool(self.index, &l2, *n_in, amt1, self.swappable_set.as_deref(), self.live_data) else {
                        continue;
                    };
                    let Some((a3, amt3)) = best_pool(self.index, &l3, *hub, amt2, self.swappable_set.as_deref(), self.live_data) else {
                        continue;
                    };
                    let Some((a4, _)) = best_pool(self.index, &l4, *n_out, amt3, self.swappable_set.as_deref(), self.live_data) else {
                        continue;
                    };

                    if let Some(route) = simulate_path(self.index, &[
                        (a1, *input, *n_in),
                        (a2, *n_in, *hub),
                        (a3, *hub, *n_out),
                        (a4, *n_out, *output),
                    ],
                    amount_in, self.swappable_set.as_deref(), self.live_data) {
                        out.push(route);
                    }
                }
            }
        }
    }
}

// =============================================================================
// Simulation helpers
// =============================================================================

/// Simulate a single hop using live data when available.
///
/// `with_impact`: when true, run a second small "probe" quote to measure the
/// real price impact (spot execution price vs realized price). Only the
/// final path assembly asks for it — pool-scanning helpers skip the extra
/// quote for speed.
fn simulate_hop(
    index: &PoolIndex,
    pool_address: &str,
    input_mint: Pubkey,
    amount_in: u64,
    swappable: Option<&HashSet<String>>,
    live: Option<&dyn AccountDataProvider>,
    with_impact: bool,
) -> Option<RouteHop> {
    if let Some(set) = swappable {
        if !set.contains(pool_address) {
            return None;
        }
    }

    let entry = index.get_pool(pool_address)?;

    if swappable.is_none() {
        if !entry.market.is_active() {
            return None;
        }
        if let Ok(fin) = entry.market.financials() {
            if fin.quote_balance < MIN_VAULT_BALANCE && fin.base_balance < MIN_VAULT_BALANCE {
                return None;
            }
        }
    }

    let (direction, output_mint) = if input_mint == entry.quote_mint {
        (SwapDirection::Buy, entry.base_mint)
    } else if input_mint == entry.base_mint {
        (SwapDirection::Sell, entry.quote_mint)
    } else {
        return None;
    };

    // Quote helper over live or cached data. With a provider present this
    // uses `calculate_output_live_ex`, which reaches tick/bin arrays and fee
    // configs through the provider (real curve math). Pools whose auxiliary
    // accounts are missing for this size return Err -> hop rejected.
    let quote = |amount: u64| -> Option<u64> {
        if let Some(provider) = live {
            let pool_data = provider.pool_account_data(&entry.pool_pubkey);
            let quote_bal = provider.token_balance(&entry.quote_vault);
            let base_bal = provider.token_balance(&entry.base_vault);
            entry
                .market
                .calculate_output_live_ex(
                    amount, direction, pool_data.as_deref(), quote_bal, base_bal, Some(provider),
                )
                .ok()
        } else {
            entry.market.calculate_output(amount, direction).ok()
        }
    };

    let output_amount = quote(amount_in)?;

    if output_amount == 0 {
        return None;
    }
    if output_amount > amount_in.saturating_mul(1_000_000) {
        return None;
    }

    // Real price impact: realized execution price vs small-probe (spot)
    // execution price, in bps. Fees cancel (both quotes pay them).
    let price_impact_bps = if with_impact && amount_in >= 1_000 {
        let probe_in = (amount_in / 128).max(1);
        match quote(probe_in) {
            Some(probe_out) if probe_out > 0 => {
                let spot_price = probe_out as f64 / probe_in as f64;
                let exec_price = output_amount as f64 / amount_in as f64;
                if exec_price < spot_price {
                    thunder_core::calculate_price_impact_bps(spot_price, exec_price)
                } else {
                    0
                }
            }
            _ => 0,
        }
    } else {
        0
    };

    Some(RouteHop {
        pool_address: pool_address.to_string(),
        dex_name: entry.dex_name.clone(),
        input_mint,
        output_mint,
        input_amount: amount_in,
        output_amount,
        price_impact_bps,
    })
}

/// Simulate a full multi-hop path returning only the final output amount.
///
/// Skips the per-hop price-impact probe quotes, so it costs one quote per
/// hop — used by the splitter's allocation loop where only the terminal
/// output matters. `None` means some hop is unquotable at this size
/// (inactive pool, zero output, or curve exhausted / missing tick arrays).
pub(crate) fn simulate_path_output(
    index: &PoolIndex,
    hops: &[(String, Pubkey, Pubkey)],
    initial_amount: u64,
    swappable: Option<&HashSet<String>>,
    live: Option<&dyn AccountDataProvider>,
) -> Option<u64> {
    let mut current_amount = initial_amount;
    for (pool_address, input_mint, _) in hops {
        let hop = simulate_hop(index, pool_address, *input_mint, current_amount, swappable, live, false)?;
        current_amount = hop.output_amount;
    }
    Some(current_amount)
}

/// Simulate a full multi-hop path.
pub(crate) fn simulate_path(
    index: &PoolIndex,
    hops: &[(String, Pubkey, Pubkey)],
    initial_amount: u64,
    swappable: Option<&HashSet<String>>,
    live: Option<&dyn AccountDataProvider>,
) -> Option<Route> {
    if hops.is_empty() { return None; }
    let mut visited = HashSet::new();
    visited.insert(hops[0].1);
    for (_, _, out_mint) in hops {
        if !visited.insert(*out_mint) { return None; }
    }
    let mut result_hops = Vec::with_capacity(hops.len());
    let mut current_amount = initial_amount;
    for (pool_address, input_mint, _) in hops {
        let hop = simulate_hop(index, pool_address, *input_mint, current_amount, swappable, live, false)?;
        current_amount = hop.output_amount;
        result_hops.push(hop);
    }
    let final_output = current_amount;

    // End-to-end price impact: realized route price vs a small-probe (spot)
    // route price, in bps. Measured once over the whole path — summing per-hop
    // impacts double-counts and spuriously rejects legit multi-hop routes.
    let price_impact_bps = if initial_amount >= 128 {
        let probe_in = (initial_amount / 128).max(1);
        match simulate_path_output(index, hops, probe_in, swappable, live) {
            Some(probe_out) if probe_out > 0 => {
                let spot_price = probe_out as f64 / probe_in as f64;
                let exec_price = final_output as f64 / initial_amount as f64;
                if exec_price < spot_price {
                    thunder_core::calculate_price_impact_bps(spot_price, exec_price)
                } else {
                    0
                }
            }
            _ => 0,
        }
    } else {
        0
    };

    let first = result_hops.first()?;
    let last = result_hops.last()?;
    Some(Route {
        input_mint: first.input_mint,
        output_mint: last.output_mint,
        input_amount: initial_amount,
        output_amount: final_output,
        price_impact_bps,
        hops: result_hops,
    })
}

/// Among `pool_addresses`, pick the one yielding the highest output.
fn best_pool(
    index: &PoolIndex,
    pool_addresses: &[String],
    input_mint: Pubkey,
    amount_in: u64,
    swappable: Option<&HashSet<String>>,
    live: Option<&dyn AccountDataProvider>,
) -> Option<(String, u64)> {
    pool_addresses
        .iter()
        .filter_map(|addr| {
            let hop = simulate_hop(index, addr, input_mint, amount_in, swappable, live, false)?;
            Some((addr.clone(), hop.output_amount))
        })
        .max_by_key(|(_, out)| *out)
}

/// Return the top N pools by output amount.
fn top_pools(
    index: &PoolIndex,
    pool_addresses: &[String],
    input_mint: Pubkey,
    amount_in: u64,
    n: usize,
    swappable: Option<&HashSet<String>>,
    live: Option<&dyn AccountDataProvider>,
) -> Vec<(String, u64)> {
    let mut candidates: Vec<(String, u64)> = pool_addresses
        .iter()
        .filter_map(|addr| {
            let hop = simulate_hop(index, addr, input_mint, amount_in, swappable, live, false)?;
            Some((addr.clone(), hop.output_amount))
        })
        .collect();
    candidates.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    candidates.truncate(n);
    candidates
}