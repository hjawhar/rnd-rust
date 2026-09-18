# Solana Twitter Sniper

![Rust](https://img.shields.io/badge/Rust-000000?style=flat&logo=rust&logoColor=white)
![no status](https://img.shields.io/badge/no%20status-inactive-lightgrey)
![license](https://img.shields.io/badge/license-MIT-blue)
![rust](https://img.shields.io/badge/rust-1.83%2B-orange)
![angular](https://img.shields.io/badge/angular-19-red)
![python](https://img.shields.io/badge/python-3.10%2B-blue)

A multi-service trading system that monitors Twitter for new Solana token launches and executes automated buy orders using multiple block builders.

## How It Works

1. **Monitor** - 6 concurrent WebSocket connections stream real-time tweets from watched accounts via the Axsys Twitter API
2. **Detect** - Contract addresses (CA) are extracted from tweet text and images (via OCR)
3. **Execute** - Buy transactions are built and submitted simultaneously across Jito, Nextblock, Bloxroute, and Zero Slot block builders
4. **Confirm** - Yellowstone gRPC streams transaction confirmations; failed transactions are automatically retried

## Architecture

```
Twitter (Axsys WebSocket)
    |
    v
+--------------------------------------------------+
|  axsys-sol-bot (Rust)                            |
|                                                  |
|  6x AxsysWS  --mpsc-->  Tweet Processor          |
|  connections             (text + OCR)            |
|                              |                   |
|                         Buy Engine               |
|                         (multi-builder)          |
|                              |                   |
|  Yellowstone  <--------> Tx Monitor &            |
|  gRPC stream             Retry Logic             |
|                              |                   |
|  WsClient  <------------ Log Broadcaster         |
+--------------------------------------------------+
       |                           |
       v                           v
+-----------------+     +----------------+
|  webapp (Rust)  |     |  sts-ocr       |
|  Axum REST API  |     |  Flask + OCR   |
|  + PostgreSQL   |     +----------------+
+---------+-------+
          |
          v
+-----------------+
|  sts-ui         |
|  Angular 19     |
+-----------------+
```

## Sub-Projects

| Directory | Language | Description |
|---|---|---|
| `sts/` | Rust | Core backend: trading bot (`axsys-sol-bot`) + web API (`webapp`) |
| `sts-ocr/` | Python | OCR microservice using PaddleOCR + Flask |
| `sts-ocr-playground/` | Rust + Python | OCR experiments and prototyping |
| `sts-ui/` | Angular 19 | Dashboard frontend (tasks, wallets, users, real-time logs) |

## Prerequisites

- **Rust** 1.83+ with Cargo
- **PostgreSQL** (or Docker)
- **Node.js** 18+ with npm (for the frontend)
- **Docker** and Docker Compose
- **Diesel CLI**: `cargo install diesel_cli --no-default-features --features postgres`

### Ubuntu System Dependencies

```sh
sudo apt install build-essential libpq-dev libssl-dev pkg-config
```

## Quick Start

### 1. Configure Environment

```sh
cd sts
cp .env.example .env
# Edit .env with your API keys, RPC endpoints, and wallet configuration
```

### 2. Start Infrastructure

```sh
# Start PostgreSQL + web API
docker compose -f sts/docker-compose-main.yaml up -d

# Start OCR service
docker compose -f sts-ocr/docker-compose.yaml up -d
```

### 3. Run the Trading Bot

```sh
cd sts
cargo run --release --bin axsys-sol-bot
```

### 4. Run the Frontend (optional)

```sh
cd sts-ui
npm install
ng serve
# Open http://localhost:4200
```

## Development

### Rust Backend

```sh
cd sts
cargo build                              # Debug build
cargo build --release                    # Release build
cargo run --release --bin webapp          # Web API (port 3000)
cargo run --release --bin axsys-sol-bot   # Trading bot (port 3001)
cargo clippy -- -D warnings              # Lint
cargo fmt                                # Format
diesel migration run                     # Run DB migrations
```

### Frontend

```sh
cd sts-ui
npm install
ng serve                                 # Dev server (port 4200)
ng build --configuration production      # Production build
```

## Configuration

All runtime configuration is via environment variables. See [`sts/.env.example`](sts/.env.example) for the full template.

Key categories:
- **Solana RPC** - Helius, HelloMoon, or other RPC endpoints + Yellowstone gRPC for streaming
- **Block builders** - Jito, Nextblock, Bloxroute, Zero Slot endpoints and API keys
- **Twitter feed** - Axsys WebSocket credentials for real-time tweet streaming
- **Jupiter** - Swap API endpoint and key for quote fetching
- **Database** - PostgreSQL connection string
- **Encryption** - AES-256-GCM key for wallet private key storage, Ed25519 key for JWT signing

## Key Technologies

| Component | Technology |
|---|---|
| Runtime | Tokio (async Rust) |
| HTTP framework | Axum |
| Database | PostgreSQL + Diesel (async) |
| Blockchain streaming | Yellowstone gRPC (Geyser plugin) |
| Transaction bundling | Jito, Nextblock, Bloxroute |
| Swap routing | Jupiter Aggregator |
| OCR | PaddleOCR (Python) |
| Frontend | Angular 19 + Material + Bootstrap 5 |
| Auth | EVM wallet signature + JWT (Ed25519) |
| Encryption | AES-256-GCM (ring) |

## License

MIT

## Disclaimer

This project was built while learning, testing, and prototyping real-time data pipelines, multi-service Rust architectures, Solana transaction bundling, and OCR-based contract address extraction. It was written before the era of Claude and AI-assisted development, as a hands-on exercise in high-performance async systems and blockchain integration.

The system is **deprecated and no longer functional** in its current form. External dependencies it relied on (Axsys Twitter feed, specific block builder APIs, Jupiter swap endpoints) have since changed or been discontinued. The codebase was scrubbed of proprietary configuration, credentials, and internal tooling, then open-sourced as a reference implementation.

That said, this software is released as-is, without warranty of any kind. Cryptocurrency trading carries substantial financial risk. You are solely responsible for any use of this code, including funds lost to bugs, misconfigurations, market conditions, or circumstances beyond the author's control. Review the code thoroughly before deploying against real funds.
