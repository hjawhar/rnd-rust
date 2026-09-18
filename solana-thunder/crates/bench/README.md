# thunder-bench

Benchmark harness comparing the local Thunder engine (`/quote`) against
Jupiter's swap API, per the roadmap's Workstream 2. One binary, four
subcommands, no CLI framework:

```
thunder-bench gen-cases [--sol-price <f64>] [--tokens-per-tier <n>] [--out <path>] [--slippage-bps <n>]
thunder-bench run --cases bench/cases.json --out runs/<name>.jsonl [--resume] [--max-cache-age <secs>] [--max-hops <n>]
thunder-bench report <run.jsonl>
thunder-bench diff <runA.jsonl> <runB.jsonl> [--threshold-bps <f64>]   # exit 1 on regression
```

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `CACHE_PATH` | `pools.cache` | Pool cache read by `gen-cases` (never RPC) |
| `THUNDER_URL` | `http://localhost:8080` | Local engine base URL |
| `JUPITER_BASE_URL` | lite-api / api.jup.ag | Jupiter quote API base; set for self-hosted |
| `JUPITER_API_KEY` | (none) | portal.jup.ag key, sent as `x-api-key`; switches base to `https://api.jup.ag/swap/v1` |
| `BENCH_JUP_DEXES` | (none) | Comma-separated Jupiter DEX labels; skips runtime label discovery |
| `BENCH_JUP_RPS` | 0.45 keyless / 0.9 keyed | Jupiter pacing override (raise for self-hosted) |

## gen-cases

Loads the **full pool cache (~1.6 GB)** via `thunder_aggregator` — always use
`--release`, debug builds take minutes:

```
cargo run --release -p thunder-bench -- gen-cases --sol-price 160
```

- Pairs: SOL/USDC, SOL/USDT, USDC/USDT plus N tokens per liquidity tier
  (`--tokens-per-tier`, default 5) sampled deterministically from the pool
  index by vault balance. Tiers by estimated pool TVL: major > $1M,
  mid $100k–1M, tail $10k–100k.
- Both directions x USD ladder 0.1, 1, 10, 100, 1k, 10k, 100k, converted to
  raw units with the given SOL/USD price. `--sol-price` exists precisely so
  no engine has to be running; without it the command falls back to
  `GET {THUNDER_URL}/price?mint=SOL` and fails with a clear message if the
  engine is down.
- If the cache file is absent it exits with an error — it never falls back
  to RPC.
- Output: `bench/cases.json`, format:

```json
{ "id": "sol-usdc-100usd-fwd", "input_mint": "...", "output_mint": "...",
  "amount": 625000000, "usd_size": 100.0, "liquidity_tier": "major", "slippage_bps": 50 }
```

## run

Per case, three quotes are recorded as one JSONL line
(`{case, ours, jup_restricted, jup_unrestricted, ts}`):

1. **Thunder** — `GET /quote?...&maxHops=2`. Records best-route
   `routes[0].outputAmount`, server `timeTakenMs`, wall RTT (separately —
   they measure different things), hop pool addresses, and a `/health`
   `lastSlot` hint. A refused connection (engine not running) becomes a
   recorded per-case error, not a crash.
2. **Jupiter restricted** — `dexes=<our labels>` so both sides route over
   the same pool universe (`swapMode=ExactIn&restrictIntermediateTokens=false`).
3. **Jupiter unrestricted** — same minus `dexes`; the absolute-market column.

Note: the lite-api free tier rejects `restrictIntermediateTokens=false`
(HTTP 400 `NOT_SUPPORTED`, observed July 2026). The client detects this,
retries once without the param (the server default — a restricted
intermediate-token set — then applies) and omits it for the rest of the run,
printing a note. Keyed and self-hosted setups get the full
`restrictIntermediateTokens=false` behavior.

"No route" (empty Thunder `routes`, or Jupiter's `COULD_NOT_FIND_ANY_ROUTE` /
`TOKEN_NOT_TRADABLE` errors) is an explicit recorded outcome.

**Label discovery** at startup: one unrestricted SOL→USDC quote at 1000 SOL
collects `routePlan[].swapInfo.label` values (enriched by
`GET /program-id-to-label` when available), then fuzzy-matches our six
adapters (Raydium V4/CLMM, Meteora DAMM v1/v2, Meteora DLMM, Pump.fun AMM).
The mapping is printed; unresolved labels warn and are skipped. Override
with `BENCH_JUP_DEXES=Raydium,Raydium CLMM,...`.

**Rate limits**: token-bucket pacing at 0.45 RPS keyless (lite-api) or
0.9 RPS with `JUPITER_API_KEY`; on 429 exponential backoff x2, max 3 tries
(honors `Retry-After`). Every Jupiter response is appended to
`runs/jupiter_cache.jsonl` keyed by an FNV-1a hash of the request params;
`--max-cache-age <secs>` reuses fresh-enough entries without network calls.
`--resume` skips case ids already present in the out file, so a 100-case run
survives interruptions on the free tier.

## report

```
cargo run -p thunder-bench -- report runs/baseline.jsonl
```

Prints plain aligned text tables: overall win/tie/loss vs Jupiter restricted
(tie = |delta| <= 1 bp, delta_bps = (ours-jup)/jup*1e4), the same vs
unrestricted, a win/tie/loss matrix by usd_size x liquidity tier, a latency
table (our server ms and RTT, Jupiter RTT and self-reported ms — separate,
honestly-labeled columns), and route agreement (Jaccard over pool sets).
Writes `summary.json` (aggregates) and `summary.csv` (per-case rows) next to
the run file.

## diff

Regression gate between two runs of OURS, joined on case id:

```
cargo run -p thunder-bench -- diff runs/before.jsonl runs/after.jsonl --threshold-bps 5
```

Exit code 1 if any case's out_amount regresses by more than the threshold
(a lost route always counts as a regression), else 0.

## Self-hosted jupiter-swap-api

For volume runs and apples-to-apples latency, self-host Jupiter's quote
engine (github.com/jup-ag/jupiter-swap-api docs) fed by the same
`GEYSER_ENDPOINT` the engine uses, then:

```
JUPITER_BASE_URL=http://localhost:8081 BENCH_JUP_RPS=20 \
  cargo run --release -p thunder-bench -- run --cases bench/cases.json --out runs/selfhosted.jsonl
```

No `x-api-key` is sent unless `JUPITER_API_KEY` is set. The response cache
key includes the base URL, so lite/pro/self-hosted responses never mix.

## Future: `simulate` (feature = "sim")

Ground-truth mode for divergent cases (|delta| > 20 bps): build both
transactions, snapshot the union of touched accounts at one slot, execute
both in LiteSVM against the identical snapshot, and compare actual out
amounts. Blocked on Workstream 1 M3 (`/swap-instructions`); see the stub
comment in `src/main.rs`.
