# distributed-system

A distributed system in Rust exercising real distributed patterns: service discovery, pub/sub, request-reply, horizontal scaling, and distributed tracing. Four services communicate via NATS, deployed locally via docker-compose or on Kubernetes, with Prometheus/Grafana/Jaeger observability.

## Architecture

```
Client --HTTP--> Gateway (port 3000)
                   |
              NATS pub [tasks.<type>]
                   |
            Processor (port 9091 metrics)
             /              \
    NATS req/reply      NATS pub [results.<type>]
    [enrich.request]         |
         |              Aggregator (port 3001)
    Enricher                 |
    (port 9090 metrics)  PostgreSQL
```

- **Gateway** -- HTTP API that accepts tasks and publishes them to NATS.
- **Processor** -- Queue consumer that processes tasks, calls the enricher via request-reply, and publishes results.
- **Enricher** -- NATS request-reply service that returns simulated enrichment data.
- **Aggregator** -- Consumes results from NATS and persists them to PostgreSQL. Exposes a query API.

All services emit Prometheus metrics. With `OTEL_EXPORTER_OTLP_ENDPOINT` set, traces propagate through NATS headers to Jaeger.

## Prerequisites

- [Rust](https://rustup.rs/) 1.90.0+
- [Docker](https://docs.docker.com/get-docker/) and Docker Compose
- [oha](https://github.com/hatoo/oha) (for benchmarks, optional)
- [nats CLI](https://github.com/nats-io/natscli) (for NATS benchmarks, optional)
- [k3d](https://k3d.io/) + [kubectl](https://kubernetes.io/docs/tasks/tools/) + [Helm](https://helm.sh/) (for Kubernetes deployment, optional)

## Quick Start

```sh
# Start everything (infrastructure + services)
./scripts/dev.sh

# With distributed tracing (traces visible in Jaeger)
./scripts/dev-tracing.sh

# Submit a test task
curl -X POST http://127.0.0.1:3000/tasks \
  -H 'Content-Type: application/json' \
  -d '{"task_type":"compute","data":{"value":42}}'

# Check results
curl http://127.0.0.1:3001/stats
curl http://127.0.0.1:3001/results?limit=5

# Stop services (Ctrl+C in the dev.sh terminal)
# Stop infrastructure
docker compose down
```

## Configuration

All configuration is done through environment variables. Copy `.env.example` to `.env` to customize:

```sh
cp .env.example .env
```

Every variable has a sensible default, so `.env` is optional for local development.

### Infrastructure (docker-compose)

| Variable | Default | Description |
|---|---|---|
| `POSTGRES_USER` | `distributed` | PostgreSQL username |
| `POSTGRES_PASSWORD` | `distributed` | PostgreSQL password |
| `POSTGRES_DB` | `aggregator` | PostgreSQL database name |
| `GRAFANA_ADMIN_PASSWORD` | `admin` | Grafana admin password |

### Services

| Variable | Default | Used by |
|---|---|---|
| `NATS_URL` | `nats://localhost:4222` | All services |
| `GATEWAY_ADDR` | `0.0.0.0:3000` | Gateway |
| `AGGREGATOR_ADDR` | `0.0.0.0:3001` | Aggregator |
| `DATABASE_URL` | `postgres://distributed:distributed@localhost:5432/aggregator` | Aggregator |
| `METRICS_ADDR` | `0.0.0.0:9091` (processor) / `0.0.0.0:9090` (enricher) | Processor, Enricher |
| `PROCESSOR_ID` | random UUID | Processor |
| `PROCESSING_DELAY_MS` | `50` | Processor |
| `ENRICH_TIMEOUT_MS` | `2000` | Processor |
| `ENRICH_DELAY_MS` | `10` | Enricher |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | unset | All services (enables tracing) |

### Scripts

| Variable | Default | Used by |
|---|---|---|
| `REGISTRY` | `localhost:5111` | `build-images.sh`, `deploy.sh` |

## URLs

| Service | URL | Notes |
|---|---|---|
| Gateway | http://127.0.0.1:3000 | HTTP API, POST /tasks, GET /health, /metrics |
| Aggregator | http://127.0.0.1:3001 | Query API, GET /results, /stats, /health, /metrics |
| Grafana | http://127.0.0.1:3003 | Dashboards (admin/admin) |
| Prometheus | http://127.0.0.1:9092 | Metrics queries |
| Jaeger | http://127.0.0.1:16686 | Distributed traces (requires dev-tracing.sh) |
| Enricher metrics | http://127.0.0.1:9090/metrics | Prometheus metrics |
| Processor metrics | http://127.0.0.1:9091/metrics | Prometheus metrics |
| NATS monitoring | http://127.0.0.1:8222 | NATS server info |

## Development

### Build

```sh
cargo build                          # Debug, all crates
cargo build --release                # Release, all crates
cargo build -p <crate>               # Single crate
```

### Manual Service Startup

```sh
docker compose up -d                 # Infrastructure only
cargo run --release -p enricher
cargo run --release -p processor
cargo run --release -p aggregator
cargo run --release -p gateway
```

### Benchmarks

Services must be running.

```sh
./bench/stress.sh http://127.0.0.1:3000 30s 500
./bench/baseline.sh http://127.0.0.1:3000
./bench/nats-bench.sh
```

### Kubernetes

```sh
./scripts/cluster-up.sh              # Create k3d cluster
./scripts/build-images.sh            # Build + push Docker images
./scripts/deploy.sh                  # Deploy to k8s
./scripts/cluster-down.sh            # Tear down

# Custom registry
REGISTRY=my-registry.io ./scripts/build-images.sh
```

### Database Migrations

```sh
cargo install diesel_cli --no-default-features --features postgres
cd crates/aggregator
DATABASE_URL="postgres://distributed:distributed@localhost:5432/aggregator" diesel migration run
```

## Project Structure

```
crates/
  common/       Shared library: message types, NATS helpers, telemetry
  gateway/      HTTP API gateway (axum)
  processor/    NATS queue consumer
  enricher/     NATS request-reply handler
  aggregator/   NATS consumer + query API, PostgreSQL via Diesel
deploy/
  docker/       Multi-stage Dockerfile
  k8s/          Kubernetes manifests
  grafana/      Grafana provisioning and dashboards
bench/          Benchmark and stress test scripts
scripts/        Dev scripts, k3d cluster lifecycle, image build, deploy
```

## Tech Stack

| Component | Technology |
|---|---|
| Language | Rust (edition 2024) |
| Async runtime | Tokio |
| HTTP framework | Axum 0.8 |
| Messaging | NATS via async-nats 0.46 |
| Database | PostgreSQL 16 via Diesel 2.3 + deadpool-diesel |
| Tracing | OpenTelemetry 0.31 + tracing + tracing-opentelemetry |
| Metrics | metrics 0.24 + metrics-exporter-prometheus |
| Containers | Docker (multi-stage) |
| Orchestration | Kubernetes via k3d (optional) |
| Observability | Prometheus, Grafana, Jaeger, OTel Collector |
