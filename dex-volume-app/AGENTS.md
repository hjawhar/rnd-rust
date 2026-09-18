# Volume Maker Monorepo

## Project Overview

Volume Maker is a high-frequency distributed trading system for automated volume-making on Solana DEXs and EVM chains, plus an Angular 19 dashboard frontend. Three Rust services communicate exclusively via NATS message bus. Speed is the primary design constraint — worker-sol uses in-memory DashMap caches with embedded Geyser gRPC streaming; Redis is reserved for cross-service state only.

**Domain**: `YOUR_DOMAIN` | **API**: `YOUR_API_DOMAIN`

### Monorepo Layout

```
backend/     Rust workspace — api-server, worker-sol, worker-evm
frontend/    Angular 19 — Material + Tailwind, Phantom wallet auth, WebSocket real-time
```

## Backend Architecture & Data Flow

```
                                  Clients
                                     │
                                 HTTP / WS
                                     │
                    ┌────────────────┴─────────────────┐
                    │     api-server  (Axum :3000)     │──► Discord
                    └────────────────┬─────────────────┘
                                     │
  ┌──────────────────────────────────┴───────────────────────────────┐
  │                         NATS  Message Bus                        │
  │               RPC  ·  JetStream  ·  Pub/Sub  ·  Events           │
  └───────┬──────────────────────────────────────────────┬───────────┘
          │                                              │
  ┌───────┴──────────────────────┐              ┌────────┴─────────┐
  │       worker-sol             │              │   worker-evm     │
  │  Volume Maker · 6 DEXs       │              │  Uniswap V2/V3/V4│
  │  Jito bundles · Geyser gRPC  │              │  5 EVM chains    │
  │  In-memory DashMap caches    │              │  Erigon batching │
  └───────┬──────────────────────┘              └────────┬─────────┘
          │                                              │
          ▼                                              ▼
     Solana RPC + Jito                              EVM RPCs
     Geyser gRPC                            Base · ETH · BSC · ARB · AVAX

  ┌─────────────────────────────────────────────────────────────────┐
  │                     Shared  Infrastructure                      │
  │  PostgreSQL 17.5 (Diesel + deadpool)     Redis (cross-service)  │
  │  NATS 2.10 (JetStream)                                         │
  └─────────────────────────────────────────────────────────────────┘
```

### Service Roles

- **api-server** — REST API + WebSocket + Discord bot. Chain-agnostic (zero Solana/EVM SDK deps). Routes to the correct worker based on project `network` field via NATS subject selection.
- **worker-sol** — Solana trading: 6 DEX Market trait implementations, Jito bundle submission, embedded Geyser gRPC streaming, DashMap in-memory caches.
- **worker-evm** — EVM trading: Uniswap V2/V3/V4 across 5 chains, Erigon trace_callMany batching, Redis-backed caches.

### NATS Messaging

All communication is via NATS. No direct HTTP calls between services. Subjects defined in `backend/crates/vm-nats/src/subjects.rs`. Chain-scoped pattern: `vm.{type}.{chain}.{domain}.{action}`

| Pattern | Subjects | Delivery | Use |
|---|---|---|---|
| Request-Reply | `vm.rpc.{sol,evm}.*` | Queue group (1 worker) | Synchronous queries |
| JetStream Commands | `vm.cmd.{sol,evm}.*` | WorkQueue (1 worker) | Fire-and-forget trade ops |
| Pub/Sub Events | `vm.events.{sol,evm}.*` | All subscribers | Real-time broadcasts (WS) |
| Internal | `vm.internal.{sol,evm}.*` | All subscribers | Internal coordination |

JetStream streams: `VM_SOL_COMMANDS` (subscribes to `vm.cmd.sol.>`), `VM_EVM_COMMANDS` (subscribes to `vm.cmd.evm.>`). System subjects: `vm.events.system.alert` — cross-chain system alerts, consumed by Discord service.

Subject constants live in `backend/crates/vm-nats/src/subjects.rs`. Use `vm_nats::request_with_timeout()` for all request-reply calls — it handles short/long timeouts and auto-injects `trace-id` headers.

**RPC subjects (sol)**: `TOKEN_INFO`, `POOL_FINANCIALS`, `WALLETS_FINANCIALS`, `WALLETS_IMPORT`, `WALLETS_GENERATE`, `WALLETS_VIEW`, `WALLETS_DELETE`, `PROJECT_NEW`, `PROJECT_START_TASK`, `PROJECT_STOP_TASK`, `PROJECT_DELETE`, `PROJECT_COLLECT_SOL`, `PROJECT_COLLECT_TOKENS`, `PROJECT_DISPERSE_SOL`, `PROJECT_DISPERSE_TOKENS`, `PRICE`, `PRICE_REFRESH`, `PAIR_INFO`, `INIT_DATA`, `DAILY_VOLUME`, `VERIFY_SIGNATURE`

**Command subjects (sol)**: `TASK_START/STOP/RESTART`, `TRADE_BUY/SELL/BUNDLE_BUY_SELL`, `TRACK_TOKEN/ADDRESS/UNTRACK/DELETE`, `WALLET_STORE`, `COLLECT_SOL/TOKENS`, `DISPERSE_SOL/TOKENS`, `CLMM_FETCH`

**Event subjects (sol)**: `BALANCE_UPDATE`, `TRANSACTION_NEW`, `PRICE`, `DAILY_VOLUME`, `TASK_STATUS`, `WALLETS_FINANCIALS`

**EVM RPC subjects**: `TOKEN_INFO`, `POOL_FINANCIALS`, `WALLETS_FINANCIALS`, `WALLETS_IMPORT`, `WALLETS_GENERATE`, `WALLETS_VIEW`, `WALLETS_DELETE`, `PROJECT_NEW`, `PROJECT_START_TASK`, `PROJECT_STOP_TASK`, `PROJECT_DELETE`, `PROJECT_COLLECT_ETH`, `PROJECT_COLLECT_TOKENS`, `PROJECT_DISPERSE_ETH`, `PROJECT_DISPERSE_TOKENS`, `PRICE`, `PRICE_REFRESH`, `INIT_DATA`, `DAILY_VOLUME`, `VERIFY_SIGNATURE`

**EVM command subjects**: `TASK_START/STOP/RESTART`, `TRADE_BUY/SELL/BUNDLE_BUY_SELL`, `TRACK_ADDRESS/UNTRACK/DELETE`, `WALLET_STORE`, `COLLECT_ETH/TOKENS`, `DISPERSE_ETH/TOKENS`

**EVM event subjects**: `BALANCE_UPDATE`, `TRANSACTION_NEW`, `DISCORD_WARNING/STOPPED`, `PRICE`, `DAILY_VOLUME`, `WALLETS_FINANCIALS`

## Backend Key Directories

```
backend/
  bin/
    api-server/          Axum REST API + WebSocket + Discord (port 3000)
    worker-sol/          Solana trading worker (has examples/ for manual integration tests)
    worker-evm/          EVM trading worker
    migrate-db/          One-off DB consolidation tool (tokio-postgres, not Diesel)

  crates/
    vm-data/      Chain-agnostic data layer: Diesel ORM, models, encryption, JWT
    vm-solana/    Solana chain logic: 6 DEX Market impls, pool discovery, RPC, instructions
    vm-evm/       EVM chain logic: Alloy ABI bindings, Uniswap V2/V3/V4, simulation
    vm-nats/      NATS client: connection, JetStream setup, subject constants, tracing
    vm-redis/     Redis connection pool + generic cache ops (JSON serde, hashes, TTL)
    vm-volume/    Pure computation: hourly volume distribution weights, daily budget capping

  docs/                  Internal documentation (instructions, cache guide, optimization plan)
```

## Backend Development Commands

### Build

```bash
cd backend
cargo build                              # Debug build, all crates
cargo build --release                    # Release build
cargo build --release --bin api-server   # Single binary
```

### Run Services

```bash
cd backend
cargo run --bin api-server               # API on port 3000
cargo run --bin worker-sol               # Solana worker
cargo run --bin worker-evm               # EVM worker
```

### Development (hot reload)

```bash
cd backend
cargo watch -x 'run --bin api-server'
cargo watch -x 'run --bin worker-sol'
```

### Docker (launch order: nats → redis → api-server → workers)

```bash
# Unified wrapper (handles .env loading, launch order, status)
./scripts/vm.sh init       # Create .env from .env.example
./scripts/vm.sh up         # Start all: nats → redis → db → api-server → workers
./scripts/vm.sh status     # Inspect container state
./scripts/vm.sh logs vm_backend
./scripts/vm.sh restart    # Restart app containers (truncates logs first)
./scripts/vm.sh down       # Stop and remove via compose
```

Raw compose access for ad-hoc work:
```bash
docker compose --env-file .env -f infra/docker/compose.yaml up -d
```

### Database Migrations

```bash
cd backend/crates/vm-data
diesel migration generate <name>         # Create new migration
diesel migration run                     # Apply (auto-updates schema.rs)
diesel migration revert                  # Roll back last migration
```

Migration files: `backend/crates/vm-data/migrations/` (18 migrations, timestamped `up.sql`/`down.sql` pairs).
Schema output: `backend/crates/vm-data/src/db/schema.rs` (auto-generated, do not edit).

### Solana Integration Test Examples

```bash
cd backend
cargo run --example test_pools                                    # Pool deserialization
cargo run --example test_buy_pool -- <pool_address> <amount_sol>  # Simulate buy
cargo run --example test_sell_pool -- <pool_address> <amount>     # Simulate sell
cargo run --example test_bundle_buy_sell -- <pool_addr> <amount>  # Simulate bundle
```

These hit live Solana RPC — standalone, no DB/NATS/Redis required.

### Tests

```bash
cd backend
cargo test                               # All workspace tests
cargo test -p vm-solana           # Single crate
cargo test -p worker-sol                 # Single binary crate
```

### Operational Scripts

All service management is routed through `scripts/vm.sh` (see Docker section above). Subcommands: `init`, `up`, `down`, `restart`, `stop`, `remove`, `status`, `logs`.

## Backend Code Conventions

### Rust Edition & Toolchain

- **Edition**: 2024 (all crates)
- **Resolver**: 3 (workspace-level)
- **Rust toolchain**: 1.90.0 (per Docker images)
- **Async runtime**: tokio (rt-multi-thread), manual `runtime::Builder` with configurable `TOKIO_WORKER_THREADS`

### Workspace Dependency Management

All dependencies are declared in `[workspace.dependencies]` in the root `backend/Cargo.toml` and referenced via `.workspace = true` in member crates. Internal crates referenced by path.

### Error Handling

`Box<dyn Error + Send + Sync>` everywhere. No `thiserror` or `anyhow`.

```rust
pub type DbResult<T> = Result<T, Box<dyn Error + Send + Sync>>;
// Also aliased as GenericError in vm-redis
```

Never `.expect()` or `.unwrap()` in handlers. Propagate errors up with `?`.

### Derive Macro Patterns

| Type | Derives |
|---|---|
| DB model (queryable) | `Debug, Serialize, Deserialize, Clone, Queryable, Selectable` |
| DB model (insertable) | `Debug, Deserialize, Insertable` with `#[diesel(table_name = ...)]` |
| API payload (input) | `Debug, Deserialize` (often with `validator::Validate`) |
| API payload (output) | `Debug, Serialize` |
| Message type | `Debug, Clone, Serialize, Deserialize` |

### Model File Convention

Each entity has queryable struct + `New*` insertable struct + API payload types in the same file:

```rust
// crates/vm-data/src/models/user.rs
pub struct User { ... }           // Queryable, Selectable
pub struct NewUser { ... }        // Insertable
pub struct LoginPayload { ... }   // API input (Deserialize)
pub struct LoginResponse { ... }  // API output (Serialize)
```

### Serialization

All serialization is serde JSON — NATS payloads, Redis values, WebSocket messages. No binary protocols.

### Message Protocol

`StreamType` enum in `backend/crates/vm-data/src/models/streams.rs` (~70 variants) is the primary inter-service message protocol. CQRS-style request/response pairs wrapped in `StreamInfo { user_id, stream_type }`.

Trading task types (ProcessBuy, ProcessSell, etc.) live in `backend/crates/vm-data/src/models/task.rs`.

### Chain-Agnostic Data Layer

`vm-data` has zero Solana/EVM SDK dependencies. Chain-specific types stay in their respective crates. `ProcessBuy`/`ProcessSell.market_pair` is a JSON `String` — workers serialize/deserialize their native `MarketPair` into it.

### Chain-Aware Routing

api-server routes to the correct chain worker based on `project.network` field:
- NATS **subject** carries chain identity (`rpc::sol::*` vs `rpc::evm::*`), not the variant name
- `is_evm_network()` checks for "ethereum", "base", "arbitrum", "bsc", "avalanche"
- EVM uses separate Redis key prefix (`evm:*`, `h:evm:*`)
- Both chains share the same StreamType variants for RPC operations (e.g., `RequestNewProject`, `RequestStartTask`, `RequestCollectNative`) — chain-specific variants only exist where payload types differ: trading (`EvmStartTask`/`EvmBuy`/`EvmSell` vs `StartTask`/`Buy`/`Sell`) and JetStream commands (`ProcessCollectETH` vs `ProcessCollectSOL`)

### Database Access

Single `Database` struct in `backend/crates/vm-data/src/db/mod.rs` (~1550 lines) with ~80+ async methods organized by entity. Uses `deadpool` for async connection pooling with `AsyncDieselConnectionManager<AsyncPgConnection>`. Pool pre-warming via `warm_pool()` at startup.

Access control pattern: most project/wallet queries filter by `user_id OR project_id IN (SELECT project_id FROM project_access WHERE user_id = $1)`.

### Solana DEX Abstraction (Market Trait)

Six DEXs implement the `Market` trait (`backend/crates/vm-solana/src/markets/traits.rs`):
- Raydium AMM V4, Raydium CLMM
- Meteora DAMM v1, Meteora DAMM v2, Meteora DLMM
- Pumpfun AMM

Key design: `build_swap_instruction_pure()` takes pre-fetched `SwapContext` (no I/O). `required_accounts()` declares data needs for batch-fetching. `MarketPair.to_market()` produces `Box<dyn Market>` for polymorphic dispatch.

### EVM Trading

Alloy `sol!` macro generates typed Rust bindings from JSON ABI files in `backend/crates/vm-evm/src/contracts/`. Trading engine in `backend/crates/vm-evm/src/simulation/uniswap.rs` handles V2 (swapExactETHForTokens) and V4 (UniversalRouter with V4Planner encoding + Permit2 approval). Erigon `trace_callMany` used for batch simulation.

### Wallet Security

All private keys AES-256-GCM encrypted before DB storage. Key from `AES256_GCM_KEY` env var. Encrypt/decrypt in `backend/crates/vm-data/src/utils/encryption.rs`.

### Worker Self-Initialization

Workers self-initialize on startup via NATS RPC (`rpc::sol::INIT_DATA` / `rpc::evm::INIT_DATA`) to api-server. Each worker gets the full project/wallet dataset.

### AppState Pattern

Each binary has a global `AppState` singleton via `static OnceLock<Arc<AppState>>`, initialized once at startup, accessed via `get_state()`.

- **api-server**: DB pool, NATS client, WS broadcast channel
- **worker-sol**: DashMap caches (13 caches), RPC client, NATS, DB, JetStream, Geyser mpsc sender
- **worker-evm**: NATS, DB, JetStream

### Cache Naming Convention (worker-sol)

| Prefix | Behavior | Latency |
|---|---|---|
| `get_*` | Cache-only, synchronous | <1us |
| `set_*` | Cache write, synchronous | <1us |
| `fetch_*` | Cache-first + RPC fallback, async | 100-500ms on miss |

### Volume Distribution

`vm-volume` provides hourly weight multipliers modeling real crypto market patterns (peak 14:00-17:00 UTC, trough 02:00-04:00 UTC) plus daily budget capping. Used by both workers' volume_maker strategies.

### Task Lifecycle

1. Task heartbeats: Workers write `task:hb:{chain}:{project_id}` to Redis with 30s TTL
2. api-server sweeps every 60s, auto-restarts tasks with missing heartbeats (excludes locked projects, acquires `task:restart_lock:{id}` SET NX 60s TTL)
3. Volume maker runs a self-contained loop with generation counter for graceful restarts — on TASK_START/RESTART, `commands.rs` bumps the generation and `tokio::spawn`s a new loop; old loop exits when it detects `get_task_generation() != generation` during `sleep_with_heartbeat()`
4. 15 consecutive failures trigger auto-stop
5. SOL uses in-memory DashMap (`state.task_generations`); EVM uses Redis (`evm:task_gen:{id}`)

### Bundle Buy+Sell

When `bundle_enabled` is true and a wallet has both native currency and tokens, the volume maker dispatches a bundle buy+sell. **Solana**: `BundleBuySell` uses Jito bundles for atomic execution (buy + sell as separate txs in one bundle). **EVM**: `EvmBundleBuySell` uses V4 Universal Router `execute_1()` for atomic buy+sell in a single tx (two V4Swap commands). V2/V3 falls back to sequential `process_buy()` then `process_sell()`.

### Multi-User Project Access

`project_access` junction table grants users full control over specific projects. All authorization queries include `OR project_id IN (SELECT project_id FROM project_access WHERE user_id = $1)`. Admin endpoints: `POST/DELETE /users/{uid}/projects/{pid}/access`, `GET /projects/{pid}/access`, `GET /projects/all`. WebSocket events resolve authorized user IDs via `db.get_project_authorized_user_ids()`.

### Project Killswitch

Admin can lock/unlock any project via `POST/DELETE /admin/projects/{id}/lock`. When locked: all mutating operations are rejected with 403. Read-only operations still work. Locking auto-stops any running task. The heartbeat sweep excludes locked projects from auto-restart. Enforced at api-server level before NATS dispatch. `projects.locked` column (`BOOLEAN NOT NULL DEFAULT FALSE`).

### Typed Config & Multi-Env Dotenv

Each binary has a `Config` struct loaded once at startup via `Config::from_env()`. Validates all required env vars (fail-fast, lists all missing), parses types, applies environment-specific defaults based on `APP_ENV`. Multi-env dotenv loading: `.env.{APP_ENV}` first, then `.env` fallback. `APP_ENV` must be set in the real environment (shell, Docker), not in `.env`.

### Connection Pool & Resource Scaling

DB pool pre-warming via `Database::warm_pool()` creates `pool_size/2 + 1` connections eagerly at startup. Redis uses round-robin pool of N `MultiplexedConnection`s (`REDIS_POOL_SIZE`, default 8). NATS subscription buffers tuned via `NATS_SUBSCRIPTION_CAPACITY` (default 8192). All 3 binaries use manual `tokio::runtime::Builder` with configurable `TOKIO_WORKER_THREADS`.

### Logging

`tracing` + `tracing-subscriber` throughout. Structured logging with `tracing::{info, warn, error, debug}`. Distributed trace IDs propagated via NATS headers.

## Backend Important Files

### Entry Points

| Binary | Entry | Config |
|---|---|---|
| api-server | `backend/bin/api-server/src/main.rs` | `backend/bin/api-server/src/models/state.rs` |
| worker-sol | `backend/bin/worker-sol/src/main.rs` | `backend/bin/worker-sol/src/state.rs` |
| worker-evm | `backend/bin/worker-evm/src/main.rs` | `backend/bin/worker-evm/src/cache/mod.rs` |

### Core Data Files

| File | Purpose |
|---|---|
| `backend/crates/vm-data/src/db/mod.rs` | All DB queries (~80 methods, ~1550 lines) |
| `backend/crates/vm-data/src/db/schema.rs` | Diesel auto-generated schema (9 tables) |
| `backend/crates/vm-data/src/models/streams.rs` | StreamType enum (~70 variants) — inter-service protocol |
| `backend/crates/vm-data/src/models/task.rs` | Trading task message types (Solana + EVM) |
| `backend/crates/vm-data/src/utils/encryption.rs` | AES-256-GCM wallet encryption |
| `backend/crates/vm-data/src/utils/constants.rs` | Global keys, Solana program addresses |

### Trading Logic

| File | Purpose |
|---|---|
| `backend/crates/vm-solana/src/markets/traits.rs` | Market trait (unified DEX abstraction, ~800 lines) |
| `backend/crates/vm-solana/src/markets/generic/pools.rs` | Pool discovery (all 6 DEXs in parallel) |
| `backend/crates/vm-evm/src/simulation/uniswap.rs` | EVM swap execution (V2/V4, ~900 lines) |
| `backend/crates/vm-evm/src/constants.rs` | Network enum, contract addresses, RPC providers |
| `backend/bin/worker-sol/src/requests/swap_orchestrator.rs` | Cache-first swap builder |
| `backend/bin/worker-sol/src/requests/strategies/volume_maker.rs` | Solana volume generation strategy |
| `backend/bin/worker-evm/src/requests/strategies/volume_maker.rs` | EVM volume generation strategy |

### Messaging & Infrastructure

| File | Purpose |
|---|---|
| `backend/crates/vm-nats/src/subjects.rs` | All NATS subject constants |
| `backend/crates/vm-nats/src/lib.rs` | NATS connection, JetStream setup, request_with_timeout |
| `backend/crates/vm-redis/src/lib.rs` | Redis pool + generic cache ops |
| `backend/crates/vm-volume/src/distribution.rs` | Hourly volume weights + budget capping |

### Configuration

| File | Purpose |
|---|---|
| `backend/Cargo.toml` | Workspace root, all shared dependencies |
| `backend/diesel.toml` | Diesel ORM config (schema output path, migration dir) |
| `.env.example` | Template for all environment variables (copy to `.env` at root) |
| `infra/docker/compose*.yaml` | 5 compose files: main + nats + redis + worker-sol + worker-evm |

## Backend Environment Variables

Loaded via multi-env dotenv: `.env.{APP_ENV}` first, then `.env` fallback. `APP_ENV` must be set in the real environment (shell/Docker), not in `.env`.

### Required (all services)

| Variable | Purpose |
|---|---|
| `APP_ENV` | `development` / `production` / `staging` |
| `DATABASE_URL` | PostgreSQL connection string |
| `NATS_URL` | NATS server (default: `nats://localhost:4222`) |
| `REDIS_URL` | Redis connection string |
| `AES256_GCM_KEY` | 256-bit hex key for wallet AES-256-GCM encryption (64 hex chars) |
| `ED25519_KEY` | Ed25519 PKCS8 DER keypair for JWT signing, hex-encoded (v1: 96 chars / v2: 170 chars) |

Generate both via `./scripts/vm.sh genkeys` (requires `openssl`). Keys are load-bearing: rotating `AES256_GCM_KEY` corrupts all stored encrypted wallets; rotating `ED25519_KEY` invalidates all active JWTs. The `vm-data` crate accepts both PKCS8 v1 (openssl default) and v2 Ed25519 keys via `Ed25519KeyPair::from_pkcs8_maybe_unchecked`.

### Service-Specific

| Variable | Service | Purpose |
|---|---|---|
| `PORT` | api-server | HTTP port (default: 3000) |
| `DISCORD_TOKEN` | api-server | Discord bot token |
| `API_KEY` | api-server | API authentication key |
| `RPC_ENDPOINT` | worker-sol | Solana RPC URL |
| `JITO_API_ENDPOINT` | worker-sol | Jito bundle submission |
| `GEYSER_ENDPOINT` | worker-sol | Yellowstone gRPC endpoint |
| `GEYSER_TOKEN` | worker-sol | Geyser auth token |
| `ETH_HTTP`, `BASE_HTTP`, `BSC_HTTP`, `ARB_HTTP`, `AVAX_HTTP` | worker-evm | Per-chain RPC endpoints |

### Tuning

| Variable | Default | Purpose |
|---|---|---|
| `DB_POOL_SIZE` | varies | Deadpool connection pool size |
| `REDIS_POOL_SIZE` | 8 | Round-robin MultiplexedConnection count |
| `NATS_SUBSCRIPTION_CAPACITY` | 8192 | NATS subscription buffer |
| `TOKIO_WORKER_THREADS` | CPU count | Tokio runtime thread count |
| `LOG_LEVEL` | info | tracing subscriber filter |

## Database Schema

PostgreSQL via Diesel (async with deadpool). 9 tables:

| Table | Purpose | Key Fields |
|---|---|---|
| `users` | User accounts | address, nonce, whitelisted, group_id, session_id |
| `projects` | Trading projects | network, strategy, daily_volume, pool, pool_type, locked |
| `wallets` | Encrypted wallets | project_id, address, pk (AES-256-GCM), is_main |
| `transactions` | Trade history | project_id, tx_hash (unique), tx_type, value, tokens |
| `project_access` | Multi-user sharing | project_id, user_id (junction table) |
| `subscriptions` | Billing | project_id, monthly_rate, next_payment_due, status |
| `payments` | Payment ledger | subscription_id, amount, paid_at, recorded_by |
| `audit_logs` | Admin audit trail | user_id, action, details (JSONB) |
| `uniswap_v4_pools` | EVM V4 pool cache | network_id, pool_key, currency0, currency1 |

All PKs are `SERIAL` (i32). Timestamps are `Timestamp` (without timezone, maps to `SystemTime`). Monetary fields use `Numeric` (`BigDecimal`).

## Backend Testing

Test coverage is minimal (~0% on business logic). Existing tests:

| Location | Tests | What |
|---|---|---|
| `backend/crates/vm-solana/src/markets/raydium_clmm/utils/tick_arrays.rs` | 5 | CLMM tick array math |
| `backend/bin/worker-sol/src/requests/swap_orchestrator.rs` | 2 | SwapContext/RequiredAccounts struct construction |
| `backend/crates/vm-solana/src/rpc/batched_rpc.rs` | 2 | SPL token balance extraction |

Example binaries in `backend/bin/worker-sol/examples/` hit live Solana RPC — standalone, not automated CI tests.

CI: GitHub Actions (`.github/workflows/rust.yml`): `cargo build --verbose` on push/PR to master. No test step.

## Backend Common Tasks

### Add a New DEX (Solana)

1. Create `backend/crates/vm-solana/src/markets/<dex>/` with `instructions/`, `models/`, `market_impl.rs`
2. Add variant to `MarketEnum` in `markets/generic/market_pair.rs`
3. Implement `Market` trait in `market_impl.rs`
4. Add pool discovery function in `markets/generic/pools.rs`

### Add a New NATS Subject

1. Add constant in `backend/crates/vm-nats/src/subjects.rs` under correct module
2. Add subscriber in the consuming service's `main.rs`
3. Add publisher in the producing service
4. If JetStream cmd: auto-included via `cmd::{chain}::ALL` wildcard

### Add a Database Migration

1. `cd backend/crates/vm-data && diesel migration generate <name>`
2. Write `up.sql` / `down.sql`
3. `diesel migration run` (auto-updates `schema.rs`)
4. Update model structs in `backend/crates/vm-data/src/models/`
5. Add query methods in `backend/crates/vm-data/src/db/mod.rs`

### Add a New StreamType Variant

1. Add variant(s) to `StreamType` enum in `backend/crates/vm-data/src/models/streams.rs`
2. Add handler match arm in both the publisher and consumer
3. If the variant carries new data, add the payload struct in the appropriate models file

### Add New Trading Strategy

1. Create `backend/bin/worker-sol/src/requests/strategies/<name>.rs`
2. Add event type to `StreamType` in `backend/crates/vm-data/src/models/streams.rs`
3. Add NATS subject in `backend/crates/vm-nats/src/subjects.rs`
4. Add handler in `backend/bin/worker-sol/src/main.rs`
5. Add API endpoint in `backend/bin/api-server/src/routes/`

## Frontend (frontend/)

### Build & Development

```bash
cd frontend
npm start              # Start dev server (ng serve)
npm run build          # Production build with AOT and output hashing
npm test               # Run Karma/Jasmine tests
npm run watch          # Development build with watch mode
```

### Technology Stack

- **Framework:** Angular 19 with standalone components
- **Styling:** SCSS + Tailwind CSS v4 + Angular Material + Bootstrap
- **State Management:** RxJS Subjects (no NgRx/Redux)
- **Authentication:** Phantom Wallet (Solana) + JWT tokens
- **Real-time:** WebSocket for live updates

### Architecture

Solana-based crypto trading dashboard for managing projects, wallets, and trading strategies.

**Base Component Pattern:** All page components extend `BasePageComponent` which provides `componentDestroyed$` subject for automatic subscription cleanup on destroy.

**Service Layer:**
- `ApiService` - Centralized REST API calls, responses wrapped as `{ data: T }`
- `WatcherService` - WebSocket event distribution via RxJS Subjects (`$newTx`, `$balanceUpdate`, `$walletsFinancialsInfo`, `$projectTheoPrice`)
- `HelperService` - Auth utilities, wallet management
- `ProgressiveLoaderService` - Global loading state

**Authentication Flow:**
1. Phantom wallet connection in LoginComponent
2. JWT stored in localStorage as `vm_jwt`
3. `AuthInterceptor` injects token on all API requests
4. `AuthGuard` protects routes using CanMatch

**WebSocket Connection:** Established in `MainComponent` on init, distributes events through `WatcherService` subjects.

### Module Organization

```
frontend/src/app/
├── core/              # Base classes
├── shared/            # Services, models, interceptors, guards, shared components
└── modules/           # Feature modules (lazy-loaded via routing)
    ├── login/
    ├── main/          # Layout wrapper with WebSocket init
    ├── dashboard-overview/
    ├── projects/      # list, create, details with nested dialogs
    └── users/         # list with dialogs
```

### Routing

All protected routes are children of `MainComponent` (layout wrapper):
- `/login` - Public, Phantom wallet auth
- `/projects` - Project list (default redirect from `/`)
- `/projects/new` - Create project
- `/projects/:id` - Project details with wallet management
- `/users` - User management

### Environment Configuration

Both dev and prod environments point to:
- API: `https://YOUR_API_DOMAIN`
- WebSocket: `wss://YOUR_API_DOMAIN/ws`

### Key Domain Models

Located in `frontend/src/app/shared/models/`:
- `Project` - Trading project with token/pool configuration
- `Wallet` - Solana wallet with balance tracking
- `Transaction` - Trade history
- `Pool` - Liquidity pool data
- `Token` - Token metadata

