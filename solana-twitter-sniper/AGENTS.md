# Repository Guidelines

## Project Overview

Solana Twitter Sniper is a multi-service trading system that monitors Twitter for new Solana token launches and executes automated buy orders using multiple block builders (Jito, Nextblock, Bloxroute, Zero Slot). The system extracts contract addresses from tweets — including from images via OCR — and submits bundled transactions for fast execution.

The workspace contains four independent sub-projects, each with its own git history:

| Project | Language | Purpose |
|---|---|---|
| `sts/` | Rust | Core backend: trading bot + web API |
| `sts-ocr/` | Python | OCR microservice (PaddleOCR + Flask) |
| `sts-ocr-playground/` | Rust + Python | OCR experiments and prototyping |
| `sts-ui/` | Angular 19 | Frontend dashboard |

## Architecture & Data Flow

```
Twitter (Axsys WebSocket)
    │
    ▼
┌──────────────────────────────────────────────────┐
│  axsys-sol-bot (Rust binary)                     │
│                                                  │
│  ┌─────────────┐   mpsc    ┌──────────────────┐  │
│  │ 6x AxsysWS  │────────► │ Tweet Processor   │  │
│  │ connections  │          │ (text + OCR)      │  │
│  └─────────────┘          └───────┬───────────┘  │
│                                   │              │
│                          ┌────────▼────────┐     │
│                          │  Buy Engine      │     │
│                          │  (multi-builder) │     │
│                          └────────┬────────┘     │
│                                   │              │
│  ┌──────────────┐        ┌────────▼────────┐     │
│  │ Yellowstone  │◄──────►│ Tx Monitor &    │     │
│  │ gRPC stream  │        │ Retry Logic     │     │
│  └──────────────┘        └────────┬────────┘     │
│                                   │              │
│  ┌──────────────┐        ┌────────▼────────┐     │
│  │ WsClient     │◄───────│ Log Broadcaster │     │
│  │ (to webapp)  │        └─────────────────┘     │
│  └──────────────┘                                │
└──────────────────────────────────────────────────┘
          │                            │
          ▼                            ▼
┌─────────────────┐        ┌────────────────┐
│  webapp (Rust)  │        │  sts-ocr       │
│  Axum REST API  │        │  Flask HTTP    │
│  + WebSocket    │        │  PaddleOCR     │
│  + PostgreSQL   │        └────────────────┘
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  sts-ui         │
│  Angular 19     │
│  Dashboard      │
└─────────────────┘
```

### Core Data Flow

1. **Tweet ingestion**: 6 concurrent Axsys WebSocket connections receive real-time tweet events
2. **Token detection**: Tweets are parsed for Solana contract addresses (CA) in text; images are sent to the OCR service for CA extraction
3. **Buy execution**: Matched tasks trigger simultaneous buy transactions across multiple block builders (Jito, Nextblock, Bloxroute, Zero Slot)
4. **Transaction monitoring**: Yellowstone gRPC streams slot updates and transaction confirmations
5. **Retry logic**: Failed transactions are retried with configurable attempt limits and spam-buy mode
6. **Real-time UI**: Transaction logs, tweet events, and slot numbers are broadcast to the frontend via WebSocket

### Inter-Component Communication

| Channel | Type | Purpose |
|---|---|---|
| `tx_twitter` / `rx_twitter` | `mpsc<Value>` | Raw tweet events from Axsys connections |
| `tx_task` / `rx_task` | `mpsc<MpscTask>` | Central task dispatch (tweets, tx confirmations, retries, slots) |
| `tx_logs` / `rx_logs` | `mpsc<MpscLogs>` | Log events broadcast to frontend |
| `tx_geyser_tracker` | `mpsc<GeyserTracker>` | Start/stop gRPC subscription tracking |
| `tx_client` / `rx_client` | `mpsc<String>` | Outbound WebSocket messages to central server |

## Key Directories

### sts/ (Rust Backend)

```
sts/
├── Cargo.toml                      # Workspace root
├── bin/
│   ├── axsys-sol-bot/              # Trading bot binary (main entry point)
│   │   └── src/
│   │       ├── main.rs             # Tokio runtime, channel setup, task spawning
│   │       ├── api/                # External service integrations
│   │       │   ├── twitter.rs      # Tweet processing & CA extraction
│   │       │   ├── solana.rs       # Transaction building & bundle submission
│   │       │   ├── yellowstone.rs  # Geyser gRPC streaming
│   │       │   ├── jupiter.rs      # Jupiter swap quote fetching
│   │       │   ├── ocr.rs          # OCR service client
│   │       │   ├── nextblock.rs    # Nextblock bundle submission
│   │       │   ├── bloxroute.rs    # Bloxroute bundle submission
│   │       │   ├── zero_slot.rs    # Zero Slot bundle submission
│   │       │   ├── durable_nonce.rs# Durable nonce transaction support
│   │       │   ├── pushover.rs     # Push notification alerts
│   │       │   └── tasks.rs        # Task management from central server
│   │       ├── http/               # Bot WebSocket server (port 3001)
│   │       ├── models/             # Bot-specific data types
│   │       │   ├── state.rs        # AppState (shared state with Arc<RwLock<>>)
│   │       │   ├── mpsc.rs         # Channel message types (MpscTask, MpscLogs)
│   │       │   ├── buy.rs          # Buy request payloads
│   │       │   ├── monitor.rs      # Transaction monitoring types
│   │       │   └── bundle_types.rs # Block builder enum (JITO, NEXTBLOCK, etc.)
│   │       └── utils/              # Helpers (time, HTTP, OCR text analysis)
│   ├── webapp/                     # Web API binary
│   │   └── src/
│   │       ├── main.rs             # Axum server setup, DB migrations
│   │       ├── db/                 # Diesel ORM (PostgreSQL)
│   │       │   ├── mod.rs          # Database struct with async connection
│   │       │   └── schema.rs       # Auto-generated Diesel schema
│   │       ├── http/               # REST API routes (port 3000)
│   │       │   ├── mod.rs          # Router definition
│   │       │   ├── auth.rs         # EVM signature-based authentication
│   │       │   ├── tasks.rs        # Task CRUD endpoints
│   │       │   ├── wallet.rs       # Wallet management endpoints
│   │       │   ├── users.rs        # User management endpoints
│   │       │   ├── buy.rs          # Manual buy endpoint
│   │       │   ├── blockchain.rs   # Server/pool info endpoints
│   │       │   ├── ws.rs           # WebSocket handler
│   │       │   └── middleware.rs   # JWT auth guard
│   │       ├── models/             # Web API data types
│   │       └── utils/
│   │           ├── encryption.rs   # AES-256-GCM encryption + JWT (Ed25519)
│   │           └── constants.rs    # JWT key loading
│   └── build_protos/               # Protobuf build utility
├── crates/
│   ├── axsys/                      # Axsys WebSocket client (Twitter feed)
│   ├── client/                     # Generic WebSocket client
│   ├── models/                     # Shared broadcast message types
│   └── jupiter-swap-api-client/    # Jupiter swap API client library
├── devops/
│   ├── nginx/                      # Nginx reverse proxy configs
│   └── services/jupiter.service    # Systemd service for self-hosted Jupiter API
├── docker-compose-main.yaml        # PostgreSQL + webapp
├── docker-compose-bot.yaml         # Trading bot container
├── .env.example                    # Environment variable template
└── bin/webapp/migrations/          # Diesel SQL migrations
```

### sts-ocr/ (OCR Microservice)

```
sts-ocr/
├── main.py              # Flask app: POST /ocr → PaddleOCR → JSON results
├── requirements.txt     # paddlepaddle, paddleocr, Flask, waitress
├── Dockerfile           # Python 3.10 container
└── docker-compose.yaml  # Service definition (port 6999)
```

### sts-ui/ (Angular Frontend)

```
sts-ui/
├── src/
│   ├── app/
│   │   ├── modules/
│   │   │   ├── login/              # EVM wallet signature auth
│   │   │   ├── dashboard-overview/ # Main layout
│   │   │   ├── tasks/              # Task CRUD (list + add/edit)
│   │   │   ├── wallets/            # Wallet management
│   │   │   ├── users/              # User management (admin)
│   │   │   └── manual-buy/         # Manual token buy
│   │   ├── core/                   # Base component with lifecycle
│   │   ├── shared/
│   │   │   ├── auth/               # AuthGuard
│   │   │   └── interceptors/       # JWT HTTP interceptor
│   │   ├── app.routes.ts           # Route definitions
│   │   └── app.config.ts           # Bootstrap config
│   └── environments/               # Dev/prod API URLs
├── angular.json
├── package.json                    # Angular 19, Material, Bootstrap 5
└── tsconfig.json
```

## Development Commands

### Rust Backend (sts/)

```sh
# Build
cargo build                                    # Debug build
cargo build --release                          # Release build
cargo build -p webapp                          # Single binary
cargo build -p axsys-sol-bot                   # Single binary

# Run
cargo run --release --bin webapp               # Web API (port 3000)
cargo run --release --bin axsys-sol-bot        # Trading bot (port 3001)

# Development
cargo watch -c -w src -x run                   # Auto-reload (requires cargo-watch)
cargo clippy -- -D warnings                    # Lint
cargo fmt                                      # Format

# Database
diesel migration run                           # Run pending migrations (requires diesel_cli)
diesel migration generate <name>               # Create new migration

# Infrastructure
docker compose -f docker-compose-main.yaml up -d   # Start PostgreSQL + webapp
docker compose -f docker-compose-bot.yaml up -d     # Start bot
```

### OCR Service (sts-ocr/)

```sh
docker compose up -d                           # Start OCR service (port 6999)
python main.py                                 # Run locally (requires PaddleOCR)
```

### Angular Frontend (sts-ui/)

```sh
npm install                                    # Install dependencies
ng serve                                       # Dev server (port 4200)
ng build --configuration production             # Production build
ng test                                        # Run tests (Jasmine + Karma, if needed)
```

## Code Conventions & Common Patterns

### Rust Edition & Toolchain

- **Rust 2021 edition** (see `Cargo.toml`)
- **Runtime**: Tokio multi-threaded (`#[tokio::main]`)
- **Linting**: `#![allow(clippy::all)]` is set in both binaries — clippy warnings are suppressed

### Async Architecture

The bot uses a channel-based message passing pattern. All concurrent work is coordinated through `tokio::mpsc` channels rather than shared mutable state:

```rust
// Central dispatch pattern
let (tx_task, mut rx_task) = mpsc::channel::<MpscTask>(1000);

// Producers send events
tx_task.send(MpscTask::Tweet(tweet)).await;
tx_task.send(MpscTask::TxSent(monitored_tx)).await;

// Single consumer loop processes all event types
while let Some(ref incoming_event) = rx_task.recv().await {
    match incoming_event {
        MpscTask::Tweet(tweet) => { /* spawn handler */ }
        MpscTask::TxConfirmation(tx) => { /* update state */ }
        MpscTask::Retry(task) => { /* retry buy */ }
        MpscTask::Slot(slot) => { /* update slot */ }
        _ => {}
    }
}
```

### Shared State Pattern

```rust
// AppState holds all shared resources, wrapped in Arc for thread-safe sharing
let shared_state = Arc::new(AppState {
    // Immutable config fields (no locking needed)
    rpc_endpoint,
    server_name,
    // Mutable state guarded by RwLock
    tasks: Arc::new(RwLock::new(vec![])),
    tweets_detected: Arc::new(RwLock::new(HashMap::new())),
    // Channel senders for cross-task communication
    tx_task,
    tx_logs,
});
```

### Error Handling

- **Binaries**: `Result<(), Box<dyn Error>>` or `Box<dyn Error + Send + Sync>` at entry points
- **Library code**: Direct `.unwrap()` is common in database operations (crash on DB errors)
- **Transactions**: Errors are logged via `tracing::info!` and retried through the retry loop
- No `thiserror` or `anyhow` crates for structured errors; error handling is ad-hoc

### Logging

- **Crate**: `tracing` + `tracing-subscriber` + `tracing-appender`
- **Output**: Daily rolling file logs in `./logs/logger.log`
- **Level**: `INFO` (hardcoded in subscriber init)
- **Pattern**: `tracing::info!("message {:#?}", data)` for debug-formatted values

### Database Access (webapp only)

- **ORM**: Diesel 2.x with `diesel-async` (`AsyncPgConnection`)
- **Connection**: Single `Arc<Mutex<AsyncPgConnection>>` — all queries acquire the mutex
- **Schema**: Auto-generated by Diesel CLI in `db/schema.rs`
- **Migrations**: In `bin/webapp/migrations/`, embedded via `embed_migrations!()`
- **Tables**: `users`, `wallets`, `tasks`

### Authentication

- **Method**: EVM wallet signature verification (Ethereum addresses, not Solana)
- **Flow**: Request nonce → Sign with wallet → Verify signature → Issue JWT (Ed25519)
- **JWT**: EdDSA algorithm, Ed25519 keypair from env var `ED25519_KEY`
- **Middleware**: Axum layer-based guards (`guard` for users, `guard_admin` for admins)
- **User groups**: `group_id` field (1 = admin, 3 = trader)

### Encryption

- **Private keys**: AES-256-GCM encryption via `ring` crate
- **Key**: Symmetric key from `AES256_GCM_KEY` env var (hex-encoded)
- **Wallet storage**: Private keys encrypted before database storage

### Block Builder Integration

Each builder has its own submission module in `api/`:

| Builder | Module | Protocol | Tip Account |
|---|---|---|---|
| Jito | `solana.rs` | JSON-RPC bundle | `96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5` |
| Nextblock | `nextblock.rs` | REST API | `NextbLoCkVtMGcV47JzewQdvBpLqT9TxQFozQkN98pE` |
| Bloxroute | `bloxroute.rs` | REST API | `HWEoBxYs7ssKuudEjzjmpfJVX7Dvi7wescFsVx2L5yoY` |
| Zero Slot | `zero_slot.rs` | REST API | (via Bloxroute) |

All builders follow the same pattern: build instructions → add tip → compile V0 message → sign → serialize → submit.

### Angular Frontend

- **Angular 19** with standalone components (`bootstrapApplication`)
- **Styling**: SCSS + Bootstrap 5 + Angular Material 19
- **Auth**: JWT stored client-side, attached via HTTP interceptor
- **Wallet connection**: Uses `@auth0/angular-jwt`, `bs58`, `buffer` for EVM signature flow
- **Routing**: Protected by `AuthGuard`, lazy-loaded modules
- **Base class**: `BasePageComponent` with `componentDestroyed$` for RxJS cleanup

## Important Files

| File | Purpose |
|---|---|
| `sts/bin/axsys-sol-bot/src/main.rs` | Bot entry point — all channel setup and task spawning |
| `sts/bin/axsys-sol-bot/src/api/solana.rs` | Core buy logic — transaction building and multi-builder submission |
| `sts/bin/axsys-sol-bot/src/api/twitter.rs` | Tweet processing — text analysis, OCR dispatch, CA extraction |
| `sts/bin/axsys-sol-bot/src/models/mpsc.rs` | All channel message types (MpscTask, MpscLogs, etc.) |
| `sts/bin/axsys-sol-bot/src/models/state.rs` | Bot AppState — shared state definition |
| `sts/bin/webapp/src/http/mod.rs` | All REST API route definitions |
| `sts/bin/webapp/src/db/mod.rs` | All database queries (users, tasks, wallets) |
| `sts/bin/webapp/src/db/schema.rs` | Diesel auto-generated schema (do not edit manually) |
| `sts/bin/webapp/src/utils/encryption.rs` | AES-256-GCM + JWT utilities |
| `sts/crates/axsys/src/lib.rs` | Axsys WebSocket client with reconnect logic |
| `sts/crates/client/src/lib.rs` | Generic WebSocket client |
| `sts/crates/jupiter-swap-api-client/src/lib.rs` | Jupiter API client (quote, swap, swap-instructions) |
| `sts/.env.example` | All required environment variables with descriptions |
| `sts-ocr/main.py` | OCR service — single-file Flask app |
| `sts-ui/src/app/app.routes.ts` | Frontend route definitions |
| `sts-ui/src/environments/` | API URL configuration per environment |

## Runtime & Tooling

| Component | Requirement |
|---|---|
| Rust backend | Rust 1.83+, Cargo |
| Database | PostgreSQL (via Docker or native) |
| Diesel CLI | `cargo install diesel_cli --no-default-features --features postgres` |
| OCR service | Python 3.10+, PaddleOCR (or Docker) |
| Frontend | Node.js 18+, npm, Angular CLI |
| Infrastructure | Docker, Docker Compose |

### System Dependencies (Ubuntu)

```sh
sudo apt install build-essential libpq-dev libssl-dev pkg-config
```

## Testing

No automated tests exist for the Rust backend. The Angular frontend has the default Jasmine + Karma setup (`ng test`) but no custom test suites.

The GitHub Actions workflow (`.github/workflows/build.yml`) runs `cargo build --release` for Rust and `ng build` for Angular.

## Configuration

All runtime configuration is via environment variables loaded from `.env` files using `dotenv`.

Key configuration categories:
- **Database**: `DATABASE_URL`
- **Encryption**: `AES256_GCM_KEY`, `ED25519_KEY`
- **Twitter feed**: `AXSYS_*` variables
- **Solana RPC**: `RPC_ENDPOINT`, `GEYSER_ENDPOINT`, `GEYSER_TOKEN`, `GPA_ENDPOINT`
- **Block builders**: `JITO_*`, `NEXTBLOCK_*`, `BLOXROUTE_*`, `ZERO_SLOT_*`
- **Jupiter**: `JUPITER_ENDPOINT`, `JUPITER_API_KEY`
- **OCR**: `OCR_API_ENDPOINT`
- **Server identity**: `SERVER_NAME`, `SERVER_IP`, `X_API_KEY`, `SERVERS`
- **Alerts**: `PUSHOVER_APP_TOKEN`, `PUSHOVER_USER_KEYS`

See `sts/.env.example` for the full template.

## Deployment

### Docker (Development)

```sh
# Start database + web API
docker compose -f sts/docker-compose-main.yaml up -d

# Start trading bot
docker compose -f sts/docker-compose-bot.yaml up -d

# Start OCR service
docker compose -f sts-ocr/docker-compose.yaml up -d
```

### Production

- Nginx reverse proxy (configs in `devops/nginx/`)
- Systemd service file for Jupiter self-hosted API (`devops/services/jupiter.service`)
- SSL via Let's Encrypt / Certbot
- Docker containers with `rust:1.83.0` base image
- Webapp on port 3000, bot on port 3001, OCR on port 6999
