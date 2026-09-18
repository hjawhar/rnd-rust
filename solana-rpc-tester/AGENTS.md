# AGENTS.md

Instructions for AI coding agents working on this project.

## Project Overview

**solana-rpc-tester** is a Rust-based tool for testing Solana RPC endpoints. It sends transactions via SWQoS (Stake-Weighted Quality of Service) or Jito bundles, then tracks their confirmation status through a Yellowstone gRPC (Geyser) subscription stream.

## Architecture

```
main.rs          - Entry point, transaction construction and dispatch logic
yellowstone.rs   - Yellowstone/Geyser gRPC streaming client with auto-reconnect
```

### Core Flow

1. `start_yellowstone_service` opens a gRPC stream to Geyser, subscribing to slots, transactions, and account updates for the configured wallet.
2. A periodic timer fires `NewEvent::SendTx` every 5 seconds.
3. `handle_events` processes events:
   - `SendTx` — builds a self-transfer transaction (0.001 SOL), sends via Jito bundle (private) or SWQoS (public), and records the signature + slot.
   - `Transaction` — checks if a confirmed transaction matches a previously sent signature.
   - `Slot` — tracks the current slot number.

### Key Dependencies

| Crate | Purpose |
|---|---|
| `yellowstone-grpc-client` / `yellowstone-grpc-proto` | Geyser gRPC streaming |
| `solana-sdk`, `solana-rpc-client` | Transaction building and RPC calls |
| `tokio` | Async runtime |
| `tonic` | gRPC transport |
| `reqwest` | Jito bundle HTTP API |
| `tracing` | Structured logging |

## Conventions

- **Rust edition**: 2024
- **Async runtime**: Tokio multi-thread
- **Error handling**: `Box<dyn Error + Send + Sync>` for service boundaries; prefer typed errors for new modules.
- **Logging**: Use `tracing` macros (`tracing::info!`, `tracing::error!`), not `println!`. Exception: none.
- **Config**: All secrets and endpoints via `.env` / environment variables loaded with `dotenv`.
- **No hardcoded secrets**: Never commit `.env`. Use `.env.example` for placeholder reference.

## File Structure

```
.
├── Cargo.toml
├── src/
│   ├── main.rs           # Entry point + transaction logic
│   └── yellowstone.rs    # Geyser gRPC stream manager
├── .env.example          # Environment variable template
├── .gitignore
├── LICENSE               # MIT
├── README.md
├── AGENTS.md             # This file
└── CLAUDE.md             # Claude-specific instructions
```

## Rules

1. **Never commit secrets.** `.env` is gitignored. If you see credentials in code, extract them to env vars.
2. **Keep modules focused.** `main.rs` handles orchestration and transaction logic. `yellowstone.rs` handles the gRPC stream. New concerns get new modules.
3. **Preserve reconnection behavior.** The Yellowstone service auto-reconnects on stream failures. Any changes to the streaming client must maintain this.
4. **Test transactions are self-transfers.** The tester sends SOL from the configured wallet back to itself. Do not change the destination without explicit instruction.
5. **Jito vs SWQoS is toggled by `private_tx`.** When `true`, transactions go through Jito bundles with a tip. When `false`, they go through SWQoS with a priority fee. Both paths must remain functional.
6. **Run `cargo build` before claiming work is complete.** This project has no test suite yet -- at minimum, verify compilation.
