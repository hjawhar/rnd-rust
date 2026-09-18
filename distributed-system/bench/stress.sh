#!/usr/bin/env bash
set -euo pipefail

GATEWAY_URL="${1:-http://127.0.0.1:3000}"
DURATION="${2:-10m}"
RPS="${3:-500}"
RESULTS_DIR="bench/results/$(date +%Y%m%d-%H%M%S)-stress"
mkdir -p "$RESULTS_DIR"

PAYLOAD='{"task_type":"compute","data":{"value":42}}'

echo "==> Stress test: ${RPS} RPS for ${DURATION}"
oha -z "$DURATION" \
    -q "$RPS" \
    -m POST \
    -H "Content-Type: application/json" \
    -d "$PAYLOAD" \
    --output-format json \
    "${GATEWAY_URL}/tasks" \
    > "${RESULTS_DIR}/stress.json" 2>&1

echo "==> Stress test complete. Results in ${RESULTS_DIR}/stress.json"
