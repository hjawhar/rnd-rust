#!/usr/bin/env bash
set -euo pipefail

GATEWAY_URL="${1:-http://127.0.0.1:3000}"
RESULTS_DIR="bench/results/$(date +%Y%m%d-%H%M%S)-baseline"
mkdir -p "$RESULTS_DIR"

PAYLOAD='{"task_type":"compute","data":{"value":42}}'

for RPS in 100 500 1000 5000; do
    echo "==> Testing at ${RPS} RPS for 30 seconds..."
    oha -z 30s \
        -q "$RPS" \
        -m POST \
        -H "Content-Type: application/json" \
        -d "$PAYLOAD" \
        --output-format json \
        "${GATEWAY_URL}/tasks" \
        > "${RESULTS_DIR}/rps-${RPS}.json" 2>&1
    echo "    Results saved to ${RESULTS_DIR}/rps-${RPS}.json"
    sleep 5
done

echo "==> Baseline benchmark complete. Results in ${RESULTS_DIR}/"
