# Thunder Fast Swap Execution Engine (sub-200 ms)

- **Date:** 2026-07-18
- **Status:** Design (approved for planning)
- **Supersedes:** the earlier arbitrage-loop design of the same date. Thunder is **not** an arb/MEV bot; it is the **execution engine** a bot calls instead of Jupiter. The bot brings the opportunity; the engine executes the swap faster than Jupiter.

## 1. Purpose & North Star

A Solana slot is ~400 ms. To be included when an opportunity exists, a swap must be quoted, built, signed, and submitted **well inside that window** — leaving slack for network propagation and block inclusion. Jupiter cannot serve this: its quote is an external API round-trip over a periodically-refreshed index (latency + staleness). Thunder is local, colocatable, and streams per-slot on-chain state.

**North star:** turn `(inputMint, outputMint, amount)` into a landable swap with **p99 wall-clock < 200 ms**, in either of two caller-selected modes:
- **return** — respond with a signed, landable `VersionedTransaction`; the caller submits.
- **submit** — the engine submits (Jito bundle / TPU) and returns the signature / bundle id.

The engine is infrastructure: a bot/searcher/MM calls it in place of Jupiter and wins on latency and freshness. This spec is purely a **latency-engineering** project. Arbitrage detection, cycle search, and strategy live in the *caller*, not here.

## 2. Scope

### In scope
- **C1 — Quote hot path:** decoded pool-state snapshot cache + warm route-topology cache; zero clone / zero re-parse / no full route rediscovery per request.
- **C2 — Build/sign, zero-RPC:** guarantee no RPC on the fast path (accounts pre-warmed; miss = fail fast); local signing; drop RPC pre-simulation (on-chain `min_out` guards).
- **C3 — Submission & landing (submit mode):** Jito `sendBundle` + direct TPU/RPC; **dynamic priority-fee & tip oracle**, **pre-warmed persistent connections**, **leader-aware TPU**, and optional multi-endpoint spray for fastest inclusion.
- **C4 — Latency instrumentation & budget enforcement:** per-stage timings on every response + metrics (p50/p99); a configured budget with breach counting.
- **C5 — Benchmark vs Jupiter:** measurement harness comparing Thunder local latency against Jupiter's public quote API (read-only, reference only).
- **C6 — Concurrency, runtime & scaling:** parallel sharded ingest isolated from request-serving; per-account slot-monotonic upserts; thread pools auto-sized from detected cores; backpressure via per-pool coalescing; scaling observability.
- **C7 — General performance engineering:** Pubkey-keyed maps (drop `String` addresses), fast non-crypto hasher, allocator swap, buffer reuse / no per-request allocation, cached ATA derivations, optional core pinning, startup warmup. Cross-cutting micro-architecture for the whole loop.
- **C8 — DEX coverage expansion (companion spec):** add high-volume venues currently missing (Orca Whirlpool, Raydium CPMM, Phoenix, SolFi, Obric, Lifinity, pump.fun bonding curve, …). Coverage-agnostic by design — each DEX is a `Market` impl + snapshot decoder + router adapter; tracked here, specced separately.
- Verification harness (latency bench + surfpool-fork correctness + zero-RPC assertion).

### Out of scope (caller's job or separate specs)
- Opportunity/arbitrage detection, cycle search, strategy, sizing.
- External subscription/streaming quote transport.
- Closed-loop tip/fee auto-tuning (leave a hook only).

### Non-goals / hard constraints
- **No agent or test path ever sends to mainnet.** Real Jito/TPU submission is human-run. Automated verification is simulate-only against a surfpool mainnet fork + LiteSVM (mirrors `mainnet_sim`).
- **No regression of the existing safe `/swap` default.** The fast path is opt-in; the current pre-simulated Jupiter-mirror behavior remains the default.

## 3. Latency Budget

Target p99, warm engine (cache populated, accounts hot), inside the 400 ms slot:

```
request received
  → quote      (route on fresh snapshots, warm topology)     ≤ ~40 ms
  → build+sign+compile (zero RPC, cached blockhash+ALT)       ≤ ~10 ms
  → submit     (Jito/TPU) [submit mode only]                  ≤ ~100 ms
  ────────────────────────────────────────────────────────────────────
  return mode total   < ~50 ms
  submit mode total   < ~200 ms   →  ~200 ms slack for propagation+inclusion
```

The budget is a **measured acceptance criterion**, not an aspiration (C4). Each stage is timed and its p99 recorded.

## 4. Component Design

### 4.1 C1 — Quote hot path

**Problems today** (from `prepare_swap` / `router.find_routes`):
- Routing runs **full multi-hop rediscovery on every request** (`find_routes(..., 5)`).
- `AccountDataProvider::pool_account_data` / `AccountStore::get_data` **clone the full `Vec<u8>`** per hop, per candidate; the router then re-deserializes.
- `registry.read().await.swappable_set()` and ALT read add lock/clone overhead.

**Design.**
- **Snapshot cache:** `DashMap<Pubkey, PoolStateSnapshot>` of compact `Copy` structs holding only what the output math needs per DEX family (constant-product reserves; CLMM/DAMM-V2 `sqrt_price`+`liquidity`; DLMM `active_id`+bin handles; fee bps). Streaming decodes the changed pool **once** per update and upserts. The quote path reads snapshots — no clone, no re-parse, no lock held across simulation.
- **Warm route-topology cache:** cache the top-N candidate hop sets per `(inputMint, outputMint)` pair. A request re-simulates those cached candidates against **fresh snapshots** at the requested amount and picks the best — it does **not** run full graph discovery. The candidate set is refreshed in the background and invalidated when a streaming update touches a pool on a cached route. Amount-dependence (split/route changes with size) is handled by keeping several candidates and re-ranking live, not by re-discovering.
- **Lock/clone removal:** consume the already-`Arc`'d swappable set without cloning; avoid async `RwLock` on the hot path where a snapshot/atomic read suffices.

**Acceptance.** A warm-cache quote performs zero heap allocation on the read/sim path beyond the result; p50/p99 of a warm quote is recorded and within the §3 budget. Cold pair (no cached topology) falls back to full discovery and is flagged as slow-path.

### 4.2 C2 — Build / sign, zero-RPC

**Problems today:** `ensure_route_accounts` issues **blocking RPC `get_account`** on any store miss; the default path does an **RPC pre-simulation** (up to 3×).

**Design.**
- **Zero-RPC guarantee:** on the fast path, `ensure_route_accounts` becomes assert-present — accounts are kept hot by streaming + cold_start. A miss **fails fast** (structured error → caller retries or uses slow path); it never blocks on RPC.
- **No pre-simulation** on the fast path. On-chain `min_out` (router `ERR_SLIPPAGE`) is the correctness guard. Optional zero-RPC **local LiteSVM simulation** is feature-gated and only enabled if its measured cost fits the budget.
- Cached blockhash (exists, `state.recent_blockhash`) with background refresh; cached ALT (exists, `state.alt`).
- Sign locally with `PRIVATE_KEY` when the caller requests a signed/submitted result.
- **Bare fast mode:** an opt-in `bare` flag emits a **swap-only** tx — no idempotent ATA-create, no WSOL wrap/close — for callers whose ATAs are pre-created and WSOL pre-funded. Fewer CU, fewer bytes (headroom against the 1232-byte limit on multi-hop), better landing. The default keeps the safe setup ixs.

**Acceptance.** With all route accounts warm, the fast path issues **zero RPC calls** (verified by a panicking RPC stub in tests). A missing account returns a fast structured error, not a hang.

### 4.3 C3 — Submission & landing (submit mode)

**Landing quality is where inclusion is won or lost.** Latency delivers the tx in time; fee/tip and delivery decide whether it is included.

- **Dynamic priority-fee & tip oracle.** Today `compute_unit_price` is a static `DEFAULT_COMPUTE_UNIT_PRICE = 100_000` — it loses contested slots. A background task polls `getRecentPrioritizationFees` for the route's writable accounts and maintains a percentile CU price; the Jito tip is derived the same way (market percentile, caller-overridable). Read from cache at build time — no RPC on the hot path.
- **Pre-warmed persistent connections.** Jito block-engine gRPC and TPU/QUIC connections are established at startup and kept alive; a cold connect (tens of ms) would blow the budget. Staked-connection QUIC where available for priority.
- **Leader-aware TPU.** Direct TPU submission tracks the leader schedule (`getLeaderSchedule`) + validator TPU addresses (cluster contact info) and sends to the current and next few leaders. Jito is the default landing path; leader-aware TPU is a bounded, clearly-scoped addition.
- **Multi-endpoint spray (optional).** Submit the same tx concurrently across Jito + multiple RPCs + TPU leaders; first inclusion wins (idempotent — identical signature). Configurable.
- **Jito region selection.** Prefer the lowest-latency block-engine region.
- **return mode** skips this stage and responds with the signed tx.

**Acceptance.** Submit mode returns a signature/bundle id within the §3 budget in local benchmark (clients stubbed); connections are warm before first submit; fee/tip reflect the current market percentile. Real submission is human-run.

### 4.4 C4 — Latency instrumentation & budget enforcement

- Every response carries per-stage timings: `quoteMicros`, `buildMicros`, `signMicros`, `submitMicros`, `totalMicros`.
- Exported as metrics with p50/p99 histograms per stage and per mode.
- A configured `LATENCY_BUDGET_MS`; breaches are counted and logged with the offending stage. This is the primary, continuously-measured acceptance signal.
- Exposed at a Prometheus `/metrics` endpoint (per-stage/mode latency histograms, ingest lag, queue depth, drop counts) for long-run server monitoring.

### 4.5 C5 — Benchmark vs Jupiter

- A harness measuring, for a fixed pair set: Thunder warm quote latency, quote+build latency, and (stubbed) end-to-end; compared against Jupiter's public `/quote` API round-trip measured from the same host (read-only, reference only — never used for execution).
- Reports the latency delta and notes the freshness delta (Thunder per-slot snapshots vs Jupiter indexed). Demonstrates the "beat Jupiter" claim with numbers, not assertions.

## 5. API

Extend `POST /swap` (keep Jupiter-mirror default intact):

| Field | Values | Default | Effect |
|---|---|---|---|
| `fast` | bool | `false` | Enable the zero-RPC fast path: skip pre-sim, assert-present accounts, warm route cache. |
| `mode` | `"return"` \| `"submit"` | `"return"` | Return signed tx, or submit and return signature/bundle id. |
| `submitVia` | `"jito"` \| `"tpu"` \| `"rpc"` | env default | Submission target (submit mode only). |
| `tipLamports` | u64 | env default | Jito tip (submit + jito only). |
| `bare` | bool | `false` | Emit swap-only tx (no ATA-create/WSOL wrap); caller guarantees accounts ready. |
| `spray` | bool | `false` | Submit concurrently across all configured endpoints; first inclusion wins. |
| `maxPriorityFeeLamports` | u64 | env cap | Per-tx cap for the dynamic priority-fee oracle. |

Response gains the C4 timing fields and, in submit mode, `signature` / `bundleId`.

## 6. Configuration (new env vars)

| Variable | Default | Purpose |
|---|---|---|
| `LATENCY_BUDGET_MS` | `200` | Budget for breach counting/alerts. |
| `JITO_BLOCK_ENGINE_URL` | (none) | Required for `submitVia=jito`. |
| `TPU_ENABLED` / TPU config | (none) | Direct TPU/QUIC submission. |
| `DEFAULT_SUBMIT_VIA` | `rpc` | Default submission target. |
| `DEFAULT_TIP_LAMPORTS` | (tbd) | Default Jito tip. |
| `INGEST_WORKERS` | auto (cores) | Sharded decode/upsert worker count. |
| `INGEST_QUEUE_DEPTH` | auto | Per-shard bounded queue capacity. |
| `GEYSER_STREAM_SHARDS` | `1` | Parallel gRPC subscriptions sharded by owner program. |
| `PRIORITY_FEE_PERCENTILE` | `75` | Percentile of recent prioritization fees to bid. |
| `PRIORITY_FEE_MAX_LAMPORTS` | (tbd) | Hard cap on dynamic priority fee per tx. |
| `TIP_PERCENTILE` | `75` | Percentile for the dynamic Jito tip. |
| `SUBMIT_SPRAY` | `0` | Default multi-endpoint spray on submit. |
| `JITO_REGION` | auto | Preferred Jito block-engine region. |

`PRIVATE_KEY` (existing) signs when the caller requests a signed/submitted result.

## 7. Verification

Mainnet is human-only; all automated verification is simulate-only.

1. **Latency bench (primary):** on a warm engine, assert p99 of `quote` and `quote+build` are within the §3 budget; record the numbers. Cold-pair fallback measured separately.
2. **Zero-RPC guarantee:** inject a panicking/asserting RPC client; assert the warm fast path makes no RPC calls; assert a missing account yields a fast structured error, not a block.
3. **Correctness parity:** for identical inputs, assert the fast-path tx equals the existing pre-simulated slow-path tx (same instructions/accounts), and that the on-chain `min_out` reverts on a surfpool fork when the quote is stale.
4. **Warm-cache correctness:** assert a snapshot-based quote matches a from-scratch `calculate_output_live` quote on the same state (no drift from caching).
5. **Submit mode:** Jito/TPU clients stubbed in tests; assert correct tip/priority construction and response shape. Real submission human-run per the mainnet runbook.
6. **Bench vs Jupiter (C5):** report the measured latency delta; informational, not a pass/fail gate.
7. **Ingest isolation & ordering (C6):** under synthetic high-rate update load, warm-path quote p99 stays within the §3 budget (proves ingest↔request isolation); per-account slot ordering holds under parallel sharded ingest; overload sheds via coalescing with bounded memory.
8. **Landing quality (C3):** fee/tip reflect the configured percentile of recent fees on a fixture; connections are warm before first submit; spray de-dupes on signature.
9. **Bare mode (C2):** `bare=true` tx omits ATA-create/wrap ixs, is smaller, and still simulates successfully on a surfpool fork when accounts are pre-created.
10. **Performance (C7):** micro-benchmarks show Pubkey-keyed maps + fast hasher cut per-quote lookup cost vs the `String`-keyed baseline; the warm path allocates nothing per request.

## 8. Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Warm route cache goes stale / picks a worse route | Re-simulate cached candidates on **fresh** snapshots every request; invalidate on streaming updates to route pools; keep N candidates and re-rank live. |
| Fast path hits a cold account and blocks | Assert-present + fail-fast; streaming/cold_start keep accounts hot; caller falls back to slow path. |
| Dropping pre-sim lands a bad tx | On-chain `min_out` guards; optional zero-RPC LiteSVM sim; return mode lets cautious callers sim themselves. |
| Submission latency dominates the budget | Jito/TPU direct paths; colocation; submit stage measured separately so regressions surface. |
| Snapshot decode drift vs full deserialize | Parity test (§7.4) against `calculate_output_live` on the same bytes. |
| Regressing the safe default | Fast path is strictly opt-in (`fast=true`); default `/swap` unchanged. |
| Ingest load stalls quotes | Isolate ingest CPU from request threads; remove the per-update global registry write lock; lock-free snapshot handoff; §7.7 load test gates p99. |
| Parallel ingest applies stale-over-fresh | Shard by `pubkey` (per-account in-order) + per-account slot-monotonic upsert guard. |
| Static fee loses contested slots | Dynamic priority-fee & tip oracle (percentile of recent fees); per-tx caller cap. |
| Cold submission connection blows budget | Pre-warmed persistent Jito/TPU connections at startup. |
| Missing high-volume DEXs → missed best route/opportunity | C8 coverage expansion (companion spec); latency design is coverage-agnostic. |
| `String`-keyed hot maps add hashing/alloc cost | C7: Pubkey-keyed maps + fast hasher. |

## 9. Concurrency, Runtime & Horizontal Scaling (C6)

**Design principle: ingest load must never degrade request latency.** The engine runs on multi-core hardware; ingest CPU and request-serving CPU are isolated, communicating only through the lock-free snapshot cache.

**Runtime.** Multi-threaded tokio (already `#[tokio::main]`, worker threads = cores); HTTP/quote serving scales across workers via work-stealing. CPU-bound decode is kept **off** the async reactor (a dedicated worker pool) so it cannot starve request latency.

**Parallel sharded ingest.** Today `run_stream` decodes + `store.upsert` + `registry.write().await` inline in one task — a single-core bottleneck, and the per-update registry write lock directly contends with quote reads. Redesign:
- The gRPC consumer only demuxes frames; decode + snapshot upsert fan out to **N decode workers, sharded by `pubkey`** (hash → shard), so all updates for a given account are handled in order by one worker (preserves per-account slot ordering) while different accounts run in parallel.
- Add a **per-account slot-monotonic guard**: apply an update only if `incoming_slot >= stored_slot`; drop stale. Defense against any reordering.
- **Remove the global `registry.write()` from the hot ingest path.** Swappable status becomes a per-pool atomic flag (or sharded lock-free structure) updated without a global write lock, so vault updates never block quote reads; bulk re-validation stays out-of-band.

**Backpressure & graceful degradation.** Bounded per-shard queues. Under overload, coalesce per account — the snapshot cache is last-write-wins by slot, so intermediate updates for the same account are naturally dropped. Overload degrades to **freshness lag, not unbounded memory**; drops/coalesces are counted.

**Auto-configuration from hardware.** Thread-pool sizes, shard counts, and channel capacities are derived at startup from `std::thread::available_parallelism()` (env-overridable, with sane caps). Moving from the MacBook to a bigger server needs **no manual tuning** — the engine uses the cores it finds.

**Scaling the stream itself.** When a single Yellowstone subscription saturates (bandwidth/CPU), the subscription can be **sharded across owner programs into multiple parallel gRPC connections** (`GEYSER_STREAM_SHARDS`), each feeding the same shared store. Optional; single-stream is the default.

**Observability for scaling.** Export ingest lag (slots behind chain tip), per-shard queue depth + throughput, decode-worker utilization, drop/coalesce counts, updates/sec. These make saturation visible and are the signals a future **dynamic autoscaler** (adjusting worker/shard counts by queue depth) would consume — that closed loop is a documented future hook, not built here.

**Acceptance.** Under synthetic high-rate load: warm-path quote p99 stays within the §3 budget (ingest↔request isolation); ingest throughput scales with configured worker count; per-account slot ordering holds under parallel ingest; overload sheds via coalescing with bounded memory.

## 10. General Performance Engineering (C7)

Cross-cutting techniques applied across quote → build → submit. None change behavior; all cut latency or allocation.

- **Pubkey-keyed maps.** `PoolIndex` keys pools by `String` address and stores `pool_address: String` on every edge; the router threads `String` addresses through the hot path. Switch to `Pubkey` keys (32-byte, `Copy`, cheap hash) — removes per-lookup string hashing and allocation.
- **Fast hasher.** Replace SipHash (default `HashMap`/`DashMap` hasher) with a fast non-cryptographic hasher (`ahash` / `rustc-hash`) for internal hot maps; keys are trusted on-chain data.
- **Allocator.** Swap the global allocator for `mimalloc`/`jemalloc` — measurable throughput win under the high allocation rate of ingest + quoting.
- **No per-request allocation.** Reuse buffers (pooled / thread-local `Vec`s, `SmallVec` for account-meta and hop lists); build instruction/account-meta templates once per route topology and clone-in cheaply.
- **Cached ATA derivations.** `get_associated_token_address` per (user, mint) is recomputed per build; cache per user session.
- **CPU affinity (optional).** Pin ingest decode workers and request-serving threads to disjoint core sets on the big server to avoid cache thrash and tail-latency interference; off by default, env-enabled.
- **Startup warmup.** Pre-populate the warm route-topology cache for a configured hot-pair list so the first request on those pairs is already fast.

**Acceptance.** Micro-benchmarks show reduced per-quote lookup cost and zero per-request heap allocation on the warm path; end-to-end p99 improves versus the pre-C7 baseline (recorded).

## 11. DEX Coverage Roadmap (C8)

More venues = more routes, better prices, more opportunities for the caller. The current six (Raydium V4/CLMM, Meteora DAMM V1/V2, DLMM, Pumpfun AMM) miss major liquidity. Highest-impact additions, roughly by volume:

- **Orca Whirlpool** — concentrated liquidity; one of the largest Solana DEXs. Biggest single gap.
- **Raydium CPMM** — the newer standard constant-product program (distinct from V4).
- **Phoenix** — on-chain central limit order book.
- **SolFi, Obric, Lifinity** — high-volume proactive/oracle AMMs frequently on best-price routes.
- **pump.fun bonding curve** — pre-graduation curve (distinct from the graduated pumpfun-amm), for earliest long-tail coverage.

**Architecture fit.** The latency design is coverage-agnostic: each DEX is a `Market` implementation + a `PoolStateSnapshot` decoder + a router adapter. Adding venues does **not** touch C1–C7; it widens the graph they operate on. Each DEX is its own crate (no cross-DEX imports, per the workspace rule) and its own sub-spec. Tracked here so coverage stays a first-class roadmap item, planned separately from this latency spec.

## 12. Dependency Order

C7 (performance primitives: Pubkey keys, hasher, allocator, buffer reuse) underlies everything and lands with C1/C6 since they define the hot data structures. C1 (snapshot cache + warm route cache) + C6 (parallel ingest that writes it) are co-designed and land together. C2 (zero-RPC build, bare mode) depends on the fast path C1 establishes. C3 (submission & landing: fee/tip oracle, warm connections, leader tracking) is largely independent — the fee oracle is a background service like the existing blockhash refresh — and proceeds in parallel. C4 (instrumentation + `/metrics`) threads through all stages and lands early. C5 (Jupiter benchmark) is last and informational. C8 (coverage) is a parallel, independently-specced workstream that widens the graph without touching C1–C7.
