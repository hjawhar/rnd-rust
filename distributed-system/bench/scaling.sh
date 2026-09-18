#!/usr/bin/env bash
set -euo pipefail

GATEWAY_URL="${1:-http://127.0.0.1:3000}"
RESULTS_DIR="bench/results/$(date +%Y%m%d-%H%M%S)-scaling"
mkdir -p "$RESULTS_DIR"

PAYLOAD='{"task_type":"compute","data":{"value":42}}'
RPS=1000
DURATION="30s"

for REPLICAS in 1 3 5; do
    echo "==> Scaling processor to ${REPLICAS} replicas..."
    kubectl -n distributed-system scale deployment/processor --replicas="$REPLICAS"
    kubectl -n distributed-system rollout status deployment/processor --timeout=60s
    sleep 10
    echo "==> Testing at ${RPS} RPS with ${REPLICAS} processor replicas for ${DURATION}..."
    oha -z "$DURATION" \
        -q "$RPS" \
        -m POST \
        -H "Content-Type: application/json" \
        -d "$PAYLOAD" \
        --output-format json \
        "${GATEWAY_URL}/tasks" \
        > "${RESULTS_DIR}/replicas-${REPLICAS}.json" 2>&1
    echo "    Results saved."
    sleep 5
done

kubectl -n distributed-system scale deployment/processor --replicas=3
echo "==> Scaling benchmark complete. Results in ${RESULTS_DIR}/"
