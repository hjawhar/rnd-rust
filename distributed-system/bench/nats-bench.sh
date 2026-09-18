#!/usr/bin/env bash
set -euo pipefail

NATS_URL="${1:-nats://localhost:4222}"
RESULTS_DIR="bench/results/$(date +%Y%m%d-%H%M%S)-nats"
mkdir -p "$RESULTS_DIR"

echo "==> NATS pub throughput (10k messages, 256 bytes)..."
nats bench tasks.bench --pub 1 --msgs 10000 --size 256 --server "$NATS_URL" 2>&1 | tee "${RESULTS_DIR}/pub-only.txt"

echo "==> NATS pub-sub (1 pub, 1 sub, 10k messages)..."
nats bench tasks.bench --pub 1 --sub 1 --msgs 10000 --size 256 --server "$NATS_URL" 2>&1 | tee "${RESULTS_DIR}/pub-sub-1.txt"

echo "==> NATS pub-qsub (1 pub, 3 queue subs, 10k messages)..."
nats bench tasks.bench --pub 1 --sub 3 --msgs 10000 --size 256 --qgroup processors --server "$NATS_URL" 2>&1 | tee "${RESULTS_DIR}/pub-qsub-3.txt"

echo "==> NATS benchmark complete. Results in ${RESULTS_DIR}/"
