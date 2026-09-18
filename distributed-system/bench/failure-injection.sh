#!/usr/bin/env bash
set -euo pipefail

GATEWAY_URL="${1:-http://127.0.0.1:3000}"
RESULTS_DIR="bench/results/$(date +%Y%m%d-%H%M%S)-failure"
mkdir -p "$RESULTS_DIR"

PAYLOAD='{"task_type":"compute","data":{"value":42}}'

echo "==> Starting sustained load at 500 RPS for 60s..."
echo "    A processor pod will be killed at the 15-second mark."

oha -z 60s -q 500 -m POST -H "Content-Type: application/json" -d "$PAYLOAD" --output-format json "${GATEWAY_URL}/tasks" > "${RESULTS_DIR}/load-with-failure.json" 2>&1 &
LOAD_PID=$!

sleep 15
echo "==> Killing a processor pod..."
VICTIM=$(kubectl -n distributed-system get pods -l app=processor -o jsonpath='{.items[0].metadata.name}')
kubectl -n distributed-system delete pod "$VICTIM" --grace-period=0 --force
echo "    Killed: $VICTIM"

wait $LOAD_PID
echo "==> Failure injection test complete. Results in ${RESULTS_DIR}/"
