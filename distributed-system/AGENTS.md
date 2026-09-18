# Repository Guidelines

## Project Overview

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

All services emit Prometheus metrics on their respective ports.
With `OTEL_EXPORTER_OTLP_ENDPOINT` set, traces propagate through NATS headers to Jaeger.

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

### Configuration

Copy `.env.example` to `.env` and customize as needed. All values have sensible defaults, so `.env` is optional for local development.

```sh
cp .env.example .env
# Edit .env to override database credentials, ports, etc.
```

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

## Key Directories

| Directory | Purpose |
|---|---|
| `crates/common/` | Shared library: message types, NATS helpers, telemetry bootstrap |
| `crates/gateway/` | HTTP API gateway (axum). Accepts tasks, publishes to NATS |
| `crates/processor/` | NATS queue consumer. Processes tasks, calls enricher, publishes results |
| `crates/enricher/` | NATS request-reply handler. Returns simulated enrichment data |
| `crates/aggregator/` | NATS consumer + axum query API. Persists to PostgreSQL via Diesel |
| `deploy/docker/` | Multi-stage Dockerfile (parameterized by CRATE_NAME) |
| `deploy/k8s/` | Kubernetes manifests for all services and infrastructure |
| `deploy/grafana/` | Grafana provisioning: datasources, dashboard provider, dashboard JSON |
| `deploy/` | Prometheus config, OTel Collector config |
| `bench/` | Benchmark and stress test scripts |
| `scripts/` | Dev scripts: dev runner, k3d cluster lifecycle, image build, deploy |

## Development Commands

```sh
# === Local Development ===
./scripts/dev.sh                     # Start infra + all services
./scripts/dev-tracing.sh             # Same, with distributed tracing
docker compose down                  # Stop infrastructure

# === Build ===
cargo build                          # Debug, all crates
cargo build --release                # Release, all crates
cargo build -p <crate>               # Single crate

# === Manual service startup ===
docker compose up -d                 # Infrastructure only
cargo run --release -p enricher      # Start individually
cargo run --release -p processor
cargo run --release -p aggregator
cargo run --release -p gateway

# === Benchmarks (services must be running) ===
./bench/stress.sh http://127.0.0.1:3000 30s 500
./bench/baseline.sh http://127.0.0.1:3000
./bench/nats-bench.sh
./bench/failure-injection.sh         # k8s only

# === Kubernetes ===
./scripts/cluster-up.sh              # Create k3d cluster
./scripts/build-images.sh            # Build + push Docker images
./scripts/deploy.sh                  # Deploy to k8s
REGISTRY=my-registry.io ./scripts/build-images.sh  # Custom registry
./scripts/cluster-down.sh            # Tear down

# === Diesel (aggregator) ===
cargo install diesel_cli --no-default-features --features postgres
cd crates/aggregator
DATABASE_URL="postgres://distributed:distributed@localhost:5432/aggregator" diesel migration run
```

## Code Conventions

- **Workspace deps**: All versions in root `Cargo.toml` `[workspace.dependencies]`. Crates use `dep.workspace = true`.
- **Shared types**: `common::messages` for all message types. `common::messages::subjects` for NATS subjects.
- **NATS helpers**: `common::nats::connect_nats()`, `inject_trace_context()`, `extract_trace_context()`.
- **Telemetry**: `common::telemetry::init_telemetry("name")` at startup. Returns `MetricsHandle` for `/metrics`.
- **Diesel async pattern**: `pool.get().await?.interact(|conn| { ... }).await??` — never use Diesel on a Tokio task directly.
- **Config**: `clap` with `#[arg(env = "...")]`. Env vars for containers, CLI flags for local.
- **Error handling**: `thiserror` in common, `Box<dyn Error>` in services.
- **Formatting**: `cargo fmt`. **Linting**: `cargo clippy`.
- **IPv4**: Use `127.0.0.1` not `localhost` (macOS IPv6 mismatch with oha/services binding 0.0.0.0).

## Important Files

| File | Purpose |
|---|---|
| `Cargo.toml` | Workspace root with shared dependency versions |
| `docker-compose.yaml` | Local dev: NATS, Postgres, Prometheus, Grafana, Jaeger, OTel Collector |
| `crates/common/src/messages.rs` | All shared message types and NATS subjects |
| `crates/common/src/nats.rs` | NATS connection + trace context propagation |
| `crates/common/src/telemetry.rs` | Tracing + metrics + optional OTLP setup |
| `crates/aggregator/src/schema.rs` | Diesel table schema |
| `crates/aggregator/migrations/` | PostgreSQL migrations |
| `deploy/prometheus.yaml` | Prometheus scrape config (targets host services) |
| `deploy/otel-collector-config.yaml` | OTel Collector pipeline config |
| `deploy/grafana/dashboards/distributed-system.json` | Pre-built Grafana dashboard |
| `scripts/dev.sh` | Start everything, Ctrl+C to stop |
| `.env.example` | Environment variable reference with defaults |

## Toolchain

| Item | Value |
|---|---|
| Language | Rust (edition 2024) |
| Rust version | 1.90.0+ |
| Async runtime | Tokio |
| HTTP framework | Axum 0.8 |
| Messaging | NATS via async-nats 0.46 |
| Database | PostgreSQL 16 via Diesel 2.3 + deadpool-diesel |
| Tracing | OpenTelemetry 0.31 + tracing + tracing-opentelemetry |
| Metrics | metrics 0.24 + metrics-exporter-prometheus |
| Container | Docker (multi-stage) |
| Orchestration | Kubernetes via k3d (optional) |
| Observability | Prometheus, Grafana, Jaeger, OTel Collector |
| Benchmarking | oha (HTTP load), nats CLI (NATS throughput) |

## Testing

No automated test suite. Testing is done via:
1. **Local integration**: `./scripts/dev.sh`, submit tasks via curl, check `/stats` and `/results`
2. **Stress testing**: `./bench/stress.sh`, `./bench/baseline.sh`
3. **Observability**: Grafana dashboard for metrics, Jaeger for traces, Prometheus for ad-hoc queries
4. **k8s integration**: Deploy to k3d, run benchmarks, test failure injection
