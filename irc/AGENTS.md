# Agent & contributor guide

Working reference for anyone (human or AI) modifying this repository.

## Project layout

```
irc/
├── PLAN.md                 # Canonical plan; source of truth for scope + design
├── README.md               # User-facing overview + quickstart
├── AGENTS.md               # This file
├── CLAUDE.md               # Claude-specific pointer to AGENTS.md
├── CHANGELOG.md            # Release notes
├── Cargo.toml              # Workspace
├── rust-toolchain.toml     # Pinned to 1.90
├── rustfmt.toml clippy.toml deny.toml
├── crates/
│   ├── irc-proto/          # Protocol primitives — parser, codec, types
│   ├── irc-server/         # Daemon
│   ├── irc-client-core/    # Headless client library
│   ├── irc-client-gui/     # iced GUI
│   ├── irc-bnc/            # Bouncer
│   ├── irc-cli/            # ratatui CLI (stub)
│   └── irc-testkit/        # Test harness
├── ops/
│   ├── docker/             # Per-binary Dockerfiles
│   ├── compose/            # docker-compose dev stack
│   ├── prometheus/         # Scrape config
│   └── grafana/            # Dashboards JSON
├── docs/                   # Topic documentation
├── examples/               # Reference configs
└── .github/workflows/      # CI
```

## Dev setup

```bash
rustup show                  # picks up pinned toolchain from rust-toolchain.toml
cargo install cargo-nextest cargo-deny
```

## Commands

### Daily loop

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo deny check
```

### Running binaries

```bash
cargo run -p irc-server     -- --config examples/server-config.toml
cargo run -p irc-bnc        -- --config examples/bnc-config.toml
cargo run -p irc-client-gui
```

### Containers

```bash
docker compose -f ops/compose/docker-compose.yml up --build
```

### Fuzzing

```bash
cd crates/irc-proto/fuzz
cargo +nightly fuzz run decoder -- -max_total_time=60
cargo +nightly fuzz run casemap -- -max_total_time=60
cargo +nightly fuzz run codec   -- -max_total_time=60
```

## Coding standards

- `#![forbid(unsafe_code)]` on every crate.
- `#![warn(clippy::pedantic, clippy::nursery)]`; targeted `#[allow]` with a comment explaining why.
- No `.unwrap()` / `.expect()` outside `#[cfg(test)]` and `main.rs` bootstrap.
- Every protocol identifier is a newtype (`Nick`, `ChannelName`, `AccountName`).
- `#![deny(missing_docs)]` on `irc-proto` and `irc-client-core`.
- All network I/O behind timeouts. All queues bounded.
- `thiserror` in libraries, `anyhow` in binaries.
- `tracing` spans on every connection and command dispatch.
- Oper/admin actions emit events on the `audit` tracing target.

## Testing expectations

Every PR that touches behavior should land:
1. A unit or property test covering the new branch.
2. A regression test with a comment linking the issue when fixing a bug.

Run `cargo test --workspace --all-targets` before opening a PR.

## Hard rules

- Do **not** add dependencies without updating `deny.toml`.
- Do **not** weaken lints to silence warnings.
- Do **not** add `.unwrap()` / `.expect()` to production paths.
- Do **not** store secrets as plaintext in config when a `*_file` variant exists.

## For AI agents

- `PLAN.md` is canonical. If a request conflicts with it, surface the conflict.
- Before making changes, run `cargo fmt && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --all-targets`.
- Prefer editing existing files to creating new ones.
- When touching protocol code, add a test covering the change.
