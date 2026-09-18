# CLAUDE.md

Project-specific instructions for Claude Code and Claude-based agents.

## Build & Run

```bash
# Build
cargo build

# Build release
cargo build --release

# Run (requires .env with valid credentials)
cargo run
```

There is no test suite yet. Always run `cargo build` to verify compilation.

## Project Context

This is a Solana RPC testing tool. It tests transaction submission latency and confirmation by:
- Sending self-transfer transactions on a timed interval
- Routing through either Jito bundles (private) or SWQoS (public)
- Tracking confirmation via Yellowstone/Geyser gRPC subscription

## Code Style

- Rust 2024 edition idioms
- `tracing` for all logging -- never `println!` (except for Jito response debugging, which should be migrated to `tracing::debug!`)
- Async-first: everything runs on Tokio multi-thread runtime
- Prefer `Arc` for shared ownership across tasks over cloning large structs
- Use `thiserror` or typed error enums for new error types; `Box<dyn Error>` only at service boundaries

## Architecture Notes

- `main.rs`: orchestration, transaction building, event handling
- `yellowstone.rs`: Geyser gRPC stream with automatic reconnection via `async_recursion`
- `NewEvent` enum is the message type between the stream and the event handler

## Environment Variables

All config comes from `.env` (loaded via `dotenv`). See `.env.example` for the full list.

**Never hardcode or commit secrets.**

## Known Technical Debt

- `println!` used for Jito response on line 215 of main.rs -- should be `tracing::debug!`
- `handle_events` takes too many parameters -- consider a context struct
- No graceful shutdown -- tasks run until process is killed
- No test coverage
- `mul_f64_and_u64_to_u64` uses lossy float-to-int conversion -- acceptable for tip amounts but not for precise financial math
- `insecure_clone()` on Keypair is used for task distribution -- this is intentional (Solana SDK requires it for cross-task Keypair use)
