#!/usr/bin/env bash
# Build the thunder-router SBF program.
# The router crate is workspace-excluded (solana-program 2.x vs workspace sdk 3.x)
# and builds with the agave toolchain's cargo-build-sbf.
set -euo pipefail

AGAVE_BIN="${AGAVE_BIN:-$HOME/.local/share/solana/install/active_release/bin}"
export PATH="$AGAVE_BIN:$PATH"

cd "$(dirname "$0")/../crates/router-program"
cargo-build-sbf "$@"
echo "Built: $(pwd)/target/deploy/thunder_router.so"
