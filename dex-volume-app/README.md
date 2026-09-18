# Volume Maker

[![CI](https://github.com/hjawhar/dex-volume-app/actions/workflows/rust.yml/badge.svg)](https://github.com/hjawhar/dex-volume-app/actions/workflows/rust.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.90+-orange.svg)](https://www.rust-lang.org)
[![Angular](https://img.shields.io/badge/angular-19-red.svg)](https://angular.dev)

High-frequency distributed trading system for automated volume-making on Solana DEXs and EVM chains.

**Domain**: `YOUR_DOMAIN` | **API**: `YOUR_API_DOMAIN`

## Architecture

Three Rust services communicate exclusively via NATS message bus, plus an Angular 19 dashboard frontend.

```
backend/     Rust workspace — api-server, worker-sol, worker-evm
frontend/    Angular 19 — Material + Tailwind, Phantom wallet auth, WebSocket
```

- **api-server** — REST API + WebSocket + Discord (Axum, port 3000). Chain-agnostic.
- **worker-sol** — Solana trading: 6 DEXs, Jito bundles, Geyser gRPC, in-memory DashMap caches.
- **worker-evm** — EVM trading: Uniswap V2/V3/V4 across Base, Ethereum, BSC, Arbitrum, Avalanche.

## Prerequisites

- Rust 1.90.0+
- Node.js 20+
- Docker & Docker Compose
- PostgreSQL 17.5, Redis, NATS 2.10

## Development

### Backend

```bash
cd backend
cargo build
cargo run --bin api-server
cargo run --bin worker-sol
cargo run --bin worker-evm
cargo test
```

Hot reload:
```bash
cargo watch -x 'run --bin api-server'
```

### Frontend

```bash
cd frontend
npm install
npm start              # Dev server on http://localhost:4200
npm run build          # Production build
npm test               # Karma/Jasmine tests
```

### Database Migrations

```bash
cd backend/crates/vm-data
diesel migration generate <name>
diesel migration run
diesel migration revert
```

## Docker Deployment

All operations go through the `vm.sh` wrapper (handles env, launch order, status):

```bash
./scripts/vm.sh init       # Create .env from .env.example
./scripts/vm.sh genkeys    # Generate AES256_GCM_KEY and ED25519_KEY (paste into .env)
# also fill in POSTGRES_PASSWORD, REDIS_PASSWORD, and chain RPC endpoints in .env
./scripts/vm.sh up         # Start nats → redis → db → api-server → workers
./scripts/vm.sh status     # Show container state
./scripts/vm.sh logs vm_backend
./scripts/vm.sh restart    # Restart app containers (truncates logs)
./scripts/vm.sh down       # Stop and remove everything
```

> **Do not rotate `AES256_GCM_KEY` or `ED25519_KEY` on a live deployment.** Changing the AES key makes all stored encrypted wallets unreadable (data loss). Changing the Ed25519 key invalidates all active JWT sessions.

Compose files live under `infra/docker/` (`compose.yaml`, `compose.nats.yaml`, `compose.redis.yaml`, `compose.worker-sol.yaml`, `compose.worker-evm.yaml`). The wrapper invokes them with the repo-root `.env` as the interpolation source.

## Server Setup

### Ubuntu Provisioning

All server setup is routed through `vm.sh setup`:

```bash
./scripts/vm.sh setup deps        # Install build deps (build-essential, libpq-dev, libssl-dev, pkg-config)
./scripts/vm.sh setup docker      # Install Docker CE + docker-compose-plugin
ADMIN_IP=1.2.3.4 \
  ./scripts/vm.sh setup firewall  # UFW: allow ssh/http/https, deny public redis, allow ADMIN_IP for redis
./scripts/vm.sh setup all         # Run all three in order
```

These require Ubuntu/Debian and `sudo`. On other platforms they exit with a clear message.

After `setup docker`, install Rust:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### TLS Certificates

```bash
sudo certbot certonly --agree-tos --email admin@YOUR_DOMAIN -d YOUR_API_DOMAIN -d www.YOUR_API_DOMAIN
```

## Operations

### Service Management

```bash
./scripts/vm.sh up         # Start all
./scripts/vm.sh stop       # Stop containers
./scripts/vm.sh restart    # Truncate logs + restart app containers
./scripts/vm.sh remove     # Stop + remove containers
./scripts/vm.sh status     # Show container state
./scripts/vm.sh logs <container>
```

### Database Backup & Restore

```bash
# Export
docker exec -t <container_id> pg_dump -c -U vm_user -d vm_db > dump_$(date +%d-%m-%Y_%H_%M_%S).sql

# Import
docker exec -i vm_db psql -d vm_db -U vm_user < backup.sql

# Single table export (data only)
docker exec -t <container_id> pg_dump -U vm_user -d vm_db -t uniswap_v4_pools --column-inserts --data-only > table_dump.sql
```

### Docker Log Cleanup

```bash
echo "" > $(docker inspect --format='{{.LogPath}}' worker_sol)
```

## Environment

Copy `.env.example` (repo root) to `.env` and configure. Required for docker-compose: `POSTGRES_PASSWORD`, `REDIS_PASSWORD`. Key app variables: `APP_ENV`, `DATABASE_URL`, `NATS_URL`, `REDIS_URL`, `AES256_GCM_KEY`, `ED25519_KEY`. See `AGENTS.md` for the full variable reference.

## License

[MIT](LICENSE)

## Disclaimer

This project was built while learning, testing, and prototyping different technologies and architectures — distributed messaging (NATS), multi-chain DEX interactions (Solana + EVM), high-frequency trading patterns, and cryptographic wallet management in Rust. It is **fully functional and production-ready**, and has been battle-tested against live markets.

The original intent was to extend this into a full market-making system, but that direction was never pursued. Rather than let the work sit unused, the codebase was scrubbed of proprietary configuration and internal bits and open-sourced in its current volume-making form.

That said, this software is released **as-is, without warranty of any kind**. Cryptocurrency trading carries substantial financial risk. You are solely responsible for any use of this code, including funds lost to bugs, misconfigurations, market conditions, or circumstances beyond the author's control. Review the code thoroughly before deploying against real funds.