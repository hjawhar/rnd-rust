# CLAUDE.md

Claude: the canonical working guide for this repository is [`AGENTS.md`](AGENTS.md). Read that first.

## Ground truth

- [`PLAN.md`](PLAN.md) — source of truth for scope, architecture, and phasing.
- [`AGENTS.md`](AGENTS.md) — commands, conventions, and hard rules.
- [`README.md`](README.md) — user-facing; keep in sync with changes.
- [`CHANGELOG.md`](CHANGELOG.md) — release notes.

## Before yielding any task

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

If any step fails, fix it or mark the failure as blocked with a reason.

## What this repo is not

- Not a fork of an existing IRC daemon. Implementation is from scratch.
- Not a compatibility shim. Favor clean cutover; no backwards-compat wrappers unless explicitly requested.
