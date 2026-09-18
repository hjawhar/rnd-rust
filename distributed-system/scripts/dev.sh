#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

PIDS=()

cleanup() {
    echo ""
    echo "==> Shutting down services..."
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
    echo "==> Services stopped."
    echo "    Run 'docker compose down' to stop infrastructure."
    exit 0
}

trap cleanup SIGINT SIGTERM

echo "==> Killing stale processes on service ports..."
for port in 3000 3001 9090 9091; do
    lsof -ti:"$port" 2>/dev/null | xargs kill -9 2>/dev/null || true
done
sleep 1

echo "==> Starting infrastructure (NATS, PostgreSQL, Prometheus, Grafana, Jaeger)..."
docker compose up -d

echo "==> Waiting for NATS..."
for i in $(seq 1 30); do
    if curl -sf http://127.0.0.1:8222/varz > /dev/null 2>&1; then
        echo "    NATS ready."
        break
    fi
    if [ "$i" -eq 30 ]; then
        echo "    NATS monitoring not responding, checking client port..."
        until nc -z 127.0.0.1 4222 2>/dev/null; do sleep 1; done
        echo "    NATS client port ready."
    fi
    sleep 1
done

echo "==> Waiting for PostgreSQL..."
until docker compose exec -T postgres pg_isready -U distributed > /dev/null 2>&1; do
    sleep 1
done
echo "    PostgreSQL ready."

echo "==> Building release binaries..."
cargo build --release

echo "==> Starting services..."

RUST_LOG=info ./target/release/enricher &
PIDS+=($!)
sleep 1

RUST_LOG=info ./target/release/processor &
PIDS+=($!)
sleep 1

RUST_LOG=info ./target/release/aggregator &
PIDS+=($!)
sleep 2

RUST_LOG=info ./target/release/gateway &
PIDS+=($!)
sleep 1

echo ""
echo "========================================"
echo "  All services running"
echo "========================================"
echo ""
echo "  Services:"
echo "    Gateway:      http://127.0.0.1:3000"
echo "    Aggregator:   http://127.0.0.1:3001"
echo ""
echo "  Observability:"
echo "    Grafana:      http://127.0.0.1:3003  (admin/admin)"
echo "    Prometheus:   http://127.0.0.1:9092"
echo "    Jaeger:       http://127.0.0.1:16686"
echo ""
echo "  Metrics:"
echo "    Enricher:     http://127.0.0.1:9090/metrics"
echo "    Processor:    http://127.0.0.1:9091/metrics"
echo "    Gateway:      http://127.0.0.1:3000/metrics"
echo "    Aggregator:   http://127.0.0.1:3001/metrics"
echo ""
echo "  Test:"
echo "    curl -X POST http://127.0.0.1:3000/tasks -H 'Content-Type: application/json' -d '{\"task_type\":\"compute\",\"data\":{\"value\":42}}'"
echo "    curl http://127.0.0.1:3001/stats"
echo ""
echo "  Benchmarks:"
echo "    ./bench/stress.sh http://127.0.0.1:3000 30s 500"
echo "    ./bench/baseline.sh http://127.0.0.1:3000"
echo ""
echo "  Press Ctrl+C to stop all services."
echo "========================================"
echo ""

wait
