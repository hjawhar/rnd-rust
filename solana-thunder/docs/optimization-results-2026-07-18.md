# Thunder Engine — Latency & Correctness Optimization (2026-07-18)

Work done on branch `fast-engine`, executed and validated on the `launchoor`
server (8-core / 62 GB, Ubuntu 24.04) against **live mainnet data** (Helius RPC
+ Yellowstone gRPC), using the repo's existing 210-case `thunder-bench` harness
comparing Thunder quotes against Jupiter (restricted to our DEXs and
unrestricted).

## TL;DR

The engine had a **hub-pair quote blow-up (~900 ms)** and returned
**catastrophic garbage quotes** on thin pools. Three targeted, env-tunable
changes fixed both. The internal before/after below is a contamination-free
same-data A/B; the "vs Jupiter" numbers are a separate same-time run (further
down).

| Metric (maxHops=2, comparable cases) | Before | After |
|---|---|---|
| SOL→USDC quote latency (direct, warm) | ~900 ms | **~43 ms** (≈20×) |
| Mean quote RTT across 210 cases | 77.8 ms | **46.4 ms** |
| Win rate vs Jupiter (restricted) | 26.3% | **46.8%** |
| Median delta vs Jupiter | −103.9 bps | **−1.6 bps** |
| Mean delta vs Jupiter | −2929 bps | **−225 bps** |
| Loss rate | 70.2% | 51.9% |
| Catastrophic quotes (e.g. −98% on $100k majors) | present | **eliminated** |

On a same-time run, Thunder's quote **matches Jupiter on the DEXs it covers**
(median 0.00 bps, 76% win+tie, 54% identical route selection). Raw RTT is
competitive but not dominant — Jupiter's edge-cached API answered repeated
queries in ~11 ms vs our ~20 ms p50 from this host; Thunder's structural edge
is **locality** (no third-party API dependency / rate limits) and **live
per-slot freshness** rather than raw milliseconds in this test.

## What was wrong

1. **`PoolIndex::neighbors()` cloned the entire adjacency list per quote.** For a
   hub mint (SOL/USDC connect to hundreds of thousands–millions of pools) this
   allocated millions of `(Pubkey, String)` entries on *every* 2-hop quote.
2. **Leg simulation was unbounded.** `try_2hop`/`try_3hop` simulated *every*
   pool on hub-pair legs (SOL-USDT, USDT-USDC, …) — thousands of dust pools per
   leg, each a full curve recompute.
3. **No sanity bound on output.** Thin/drained/honeypot pools produced quotes
   like `D1wZHkfk` → 842 (−99.99%), `aF9kpuo8` → 5.8e14 (Jupiter returns *no
   route*), and single-pool $100k major swaps draining a pool for −98.9%. A bot
   executing these loses money.

## The fixes (all in `crates/aggregator/src`)

1. **Borrowed, hub-aware neighbor iteration** (`pool_index.rs`, `router.rs`).
   New `neighbors_iter` (zero-clone) and `neighbor_count`; neighbor exploration
   is skipped for hub-degree mints (their paths are already covered by explicit
   `HUB_MINTS` routing). Env: `ROUTER_HUB_DEGREE_SKIP` (default 2000).
2. **Liquidity leg-cap** (`router.rs`). Each pair-leg is pre-ranked by a cheap
   vault-balance read and only the top-K deepest pools are simulated. This also
   *fixed* the large-major draining (deep pool now always chosen). Env:
   `ROUTER_MAX_LEG_POOLS` (default 24).
3. **End-to-end price-impact filter** (`router.rs`). Route impact is measured
   once over the whole path (single probe), not summed per hop (which
   double-counts and rejected legit multi-hop routes). Routes above the
   threshold are dropped — matching Jupiter, which returns *no route* rather
   than a catastrophic quote. Env: `ROUTER_MAX_IMPACT_BPS` (default 9000 = 90%).
4. **Adaptive hop escalation** (`engine/src/api.rs`). `/quote` runs the fast
   2-hop search first and only retries once at 3 hops when it finds *no* route.
   With-route quotes keep their ~15 ms path; only otherwise-no-route cases pay
   the deeper search. Recovered `comparable` 77→86 (`ours_no_route` 76→67),
   mostly as ties matching Jupiter.

All three are runtime-tunable via env vars (set very high to disable), which is
also how the A/B baseline below was produced on identical data.

## Validation methodology

- Engine loaded **2,713,664 pools** fresh from RPC, cold-started **5.4M vault +
  auxiliary accounts**, and streamed **~1,800–2,000 live account updates/sec**.
- **Same-data A/B**: the engine was restarted twice from the identical on-disk
  cache (cold-start verified deterministic: 0 vault errors, identical swappable
  counts). Run 1 with the three thresholds *disabled* (original behavior), run 2
  with defaults. Jupiter responses held constant from cache so the delta is
  purely our-side. This removes the live-data/cold-start drift that contaminates
  naive before/after runs.
- Latency numbers are the harness's in-process `timeTakenMs` and localhost RTT,
  plus direct `curl` timing of SOL→USDC.

## Coverage trade-off (honest)

`comparable` dropped 114→77 and `ours_no_route` rose 39→76. The delta is almost
entirely the impact filter converting **catastrophic quotes into an honest
`no_route`** (validated case-by-case: Hotwre2J −9152 bps, D1wZHkfk −10000 bps,
sol-usdt-$100k −9890 bps, aF9kpuo8 fake-pool, etc.). Two +1.6 bps near-ties and
a handful of mild stable-pair losses were also filtered — acceptable, since a
bot prefers no quote over a dangerous one. Recovering those specific cases is a
coverage problem (see below), not a latency one.

`maxHops=3` is now affordable (was too slow pre-fix) and recovers ~9 of the
`no_route` cases (comparable 77→86) at a p95 latency cost (48 ms p50 / ~1 s p95);
it is exposed per-request via the `maxHops` param rather than forced as default.

## Not done (documented, deliberately not shipped half-built)

- **New DEX crates** (Orca Whirlpool, Raydium CPMM, Phoenix, …). Each requires
  correct on-chain account-layout + swap-math validation against live data;
  shipping unvalidated math would be worse than the current honest `no_route`.
  This is the main remaining lever for the `ours_no_route` gap.
- **`/swap` execution path.** Non-functional in this environment because
  `ROUTER_PROGRAM_ID`/`SWAP_ALT_ADDRESS` are unset — the on-chain router is a
  manual, cost-bearing mainnet deploy (per the runbook). The quote path (what a
  bot consumes to decide) is fully optimized.

## Fresh-Jupiter benchmark (same-time comparison)

210 cases, maxHops=2, Jupiter fetched live at quote time (`--max-cache-age 1`),
run `runs/final_fresh.jsonl`.

**Ours vs Jupiter restricted (our DEXs):**

| | value |
|---|---|
| comparable | 91 |
| win / tie / loss | 25 (27.5%) / 44 (48.4%) / 22 (24.2%) |
| median delta | **0.00 bps** |
| mean delta | −227 bps (dragged by a few large-size losses) |
| route agreement (Jaccard) | mean 0.60, **median 1.00**, identical 53.8% |
| ours_no_route / jup_no_route / both | 62 / 7 / 50 |

**Where we win/tie/lose (restricted, tier × size):** majors are win/tie up to
~$1k, and the liquidity leg-cap made single-path **near-parity even at $100k on
true majors** (sol-usdc / sol-usdt $100k ≈ −8 bps). The remaining losses are
**thin tokens at large size** (e.g. `4TyZGqRL` −60→−720 bps at $10k–$100k,
`USDSwr9A` −440 bps at $1k) where the liquidity is fragmented across many small
pools. Tails we cover, we win outright.

**Latency (same-time, ms):**

| column | p50 | p95 | mean |
|---|---|---|---|
| ours RTT (localhost) | 20.0 | 226.4 | 54.8 |
| ours in-process quote | 31.0 | 505.0 | 77.8 |
| Jupiter restricted RTT (network) | 11.1 | 26.4 | 12.7 |
| Jupiter unrestricted RTT (network) | 11.3 | 104.9 | 21.4 |

**Takeaways:** (1) On covered DEXs, quote *quality* is at parity with Jupiter
(median 0 bps, identical route half the time). (2) The remaining losses are a
**coverage/fragmentation** gap on thin tokens, not a pricing-math gap. (3) The
existing `?splits=` splitter is **buggy** — measured against fresh Jupiter it
returns *worse-than-single-path* output in most large cases (e.g.
sol-usdc-$100k: single −8 bps → split −4494 bps), while correctly fixing a few
(usdc-USDSwr9A-$1k-rev → parity). It violates its documented
`total_output >= single_path_output` invariant and must be fixed before it can
be enabled by default. (4) Highest-leverage next steps: **fix the splitter**
(to win large thin-token swaps) and **add DEX coverage** (to close
`ours_no_route`).
