# Contributing

Thanks for your interest in contributing.

## Development Setup

**Prerequisites:** Rust 1.90+, Node 20+, Docker, openssl.

```bash
git clone <your-fork>
cd dex-volume-app

# Generate keys and initialize env
./scripts/vm.sh init
./scripts/vm.sh genkeys  # paste output into .env, fill remaining vars

# Start infrastructure (postgres + nats + redis)
./scripts/vm.sh up
```

## Build & Test

```bash
# Backend (from repo root)
cd backend
cargo build --workspace
cargo test --workspace --lib --bins
cargo clippy --workspace

# Frontend
cd frontend
npm install
npm run build
npm test
```

## Pull Requests

- Branch from `master`
- Keep changes focused — one feature or fix per PR
- Update `AGENTS.md` and `README.md` if you change architecture, NATS subjects, DB schema, or crate structure
- Run `cargo clippy --workspace` and `cargo fmt --all` before pushing
- CI runs lint + build + tests on push; all three must pass

## Code Conventions

See [`AGENTS.md`](AGENTS.md) for the full reference:
- Rust 2024 edition, `Box<dyn Error + Send + Sync>` for errors (no `anyhow`/`thiserror`)
- All NATS subjects live in `backend/crates/vm-nats/src/subjects.rs`
- Wallet private keys always AES-256-GCM encrypted before DB storage
- Chain-agnostic data layer (`vm-data` has zero Solana/EVM SDK deps)

## Reporting Security Issues

**Do not open public issues for security vulnerabilities.** See [`SECURITY.md`](SECURITY.md).
