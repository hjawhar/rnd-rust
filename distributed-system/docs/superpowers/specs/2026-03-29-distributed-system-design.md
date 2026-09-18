# Distributed System Design Spec

## Overview

A learning-oriented distributed system built in Rust, designed to exercise real distributed patterns (service discovery, pub/sub, request-reply, horizontal scaling, distributed tracing) with production-grade observability. The system processes HTTP requests through a pipeline of decoupled services communicating via NATS, deployed on Kubernetes, and instrumented with Prometheus, Grafana, and Jaeger.

The goal is to build transferable skills for a production application.

## Architecture

### Services

Four services in a Cargo workspace, plus a shared library crate:

```
Client ──HTTP──> Gateway ──NATS pub──> Processor (N replicas, competing consumers)
                                            │
                                    NATS req/reply ──> Enricher
                                            │
                                    NATS pub ──> Aggregator ──> Query API (HTTP)

All services ──metrics──> Prometheus ──> Grafana
All services ──traces──>  OTel Collector ──> Jaeger
K8s handles service discovery, load balancing, and replica scaling.
```

### Service Responsibilities

| Crate | Type | Responsibilities |
|---|---|---|
| `common` | Library | Shared types (request/response models, event schemas), NATS connection factory, tracing/metrics bootstrap, error types |
| `gateway` | Binary (axum) | HTTP API entry point. Accepts client requests, validates input, publishes tasks to NATS. Exposes `/health` and Prometheus `/metrics`. No business logic. |
| `processor` | Binary (NATS consumer) | Subscribes to task queue (NATS queue group for competing consumers). Processes work. Calls `enricher` via NATS request-reply for supplemental data. Publishes results to NATS. Runs as N replicas. |
| `enricher` | Binary (NATS responder) | Listens for NATS request-reply. Enriches data (e.g., geo-lookup, metadata decoration). Stateless. Returns enriched payload. |
| `aggregator` | Binary (axum + NATS consumer + Diesel) | Subscribes to results from `processor`. Persists to PostgreSQL via Diesel. Exposes query API over HTTP. |

### Simulated Business Logic

Since this is a learning project, the "business logic" is deliberately simple — the focus is on the distributed infrastructure, not the domain:

- **Gateway**: Accepts a JSON payload with a `task_type` field and arbitrary `data`. Validates schema, assigns a unique request ID, publishes to NATS.
- **Processor**: Receives the task, performs a simulated CPU-bound operation (e.g., computes a hash, applies a transform, sleeps for a configurable duration to simulate load). Calls enricher for metadata. Publishes enriched result.
- **Enricher**: Receives a request, returns simulated metadata (e.g., timestamp decoration, random geo-coordinates, classification tags). Optionally adds configurable latency to simulate external API calls.
- **Aggregator**: Receives results via NATS, persists to PostgreSQL through Diesel (via `spawn_blocking` to avoid blocking the Tokio runtime). Exposes a query API to retrieve aggregated counts, latency stats, and recent results. Diesel migrations manage the schema.

### NATS Subject Design

| Subject | Pattern | Publisher | Subscriber |
|---|---|---|---|
| `tasks.<type>` | Pub/Sub (queue group) | Gateway | Processor (competing consumers) |
| `enrich.request` | Request-Reply | Processor | Enricher |
| `results.<type>` | Pub/Sub | Processor | Aggregator |

### Distributed Pattern Mapping

| Pattern | Where It's Exercised |
|---|---|
| Service discovery & load balancing | k8s Services + DNS. Gateway doesn't know processor IPs — NATS handles message routing. |
| Async pub/sub | Gateway → `tasks.*` → Processor(s). Aggregator subscribes to `results.*`. |
| Request-reply | Processor → `enrich.request` → Enricher → reply. Synchronous within async context. |
| Horizontal scaling | Processor uses NATS queue groups. Deploy 1, 3, or 5 replicas. NATS distributes work automatically. |
| Distributed tracing | OpenTelemetry context propagated in NATS message headers. Full trace: gateway → processor → enricher → aggregator. |

## Project Structure

```
distributed-system/
├── Cargo.toml              # workspace root
├── crates/
│   ├── common/             # shared types, NATS client setup, tracing, metrics
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   ├── gateway/            # HTTP API gateway (axum)
│   │   ├── Cargo.toml
│   │   └── src/main.rs
│   ├── processor/          # worker service (NATS consumer, horizontally scaled)
│   │   ├── Cargo.toml
│   │   └── src/main.rs
│   ├── enricher/           # enrichment service (NATS request-reply responder)
│   │   ├── Cargo.toml
│   │   └── src/main.rs
│   └── aggregator/         # aggregation + query API (axum + Diesel)
│       ├── Cargo.toml
│       ├── migrations/     # Diesel migrations (created via diesel_cli)
│       └── src/main.rs
├── deploy/
│   ├── docker/
│   │   └── Dockerfile      # shared multi-stage Dockerfile, parameterized by crate name
│   └── k8s/
│       ├── namespace.yaml
│       ├── nats/            # NATS StatefulSet or Helm values
│       ├── gateway/         # Deployment + Service + Ingress
│       ├── processor/       # Deployment (replicas: 3) + Service
│       ├── enricher/        # Deployment + Service
│       ├── aggregator/      # Deployment + Service + Ingress
│       └── observability/   # Prometheus, Grafana, Jaeger, OTel Collector
│       ├── postgres/        # PostgreSQL StatefulSet or Helm values
├── bench/
│   ├── http/               # oha scripts for HTTP load testing
│   ├── nats/               # nats bench scripts
│   └── e2e/                # end-to-end latency measurement (Rust binary or script)
├── scripts/
│   ├── cluster-up.sh       # create k3d cluster + local registry
│   ├── cluster-down.sh     # tear down
│   ├── build-images.sh     # build + push all Docker images to local registry
│   └── deploy.sh           # kubectl apply all manifests
└── docs/
    └── superpowers/specs/  # design documents
```

## Tech Stack

### Rust Crates

Defined at workspace level, inherited by member crates:

| Crate | Version | Purpose |
|---|---|---|
| `tokio` | 1.50 | Async runtime (`features = ["full"]`) |
| `axum` | 0.8 | HTTP framework (gateway, aggregator) |
| `async-nats` | 0.46 | NATS client |
| `serde` | 1.0 (`derive` feature) | Serialization |
| `serde_json` | 1.0 | JSON serialization |
| `opentelemetry` | 0.30 | Telemetry API |
| `opentelemetry-sdk` | 0.30 | Telemetry SDK |
| `opentelemetry-otlp` | 0.30 | OTLP exporter |
| `tracing` | 0.1 | Structured logging / span instrumentation |
| `tracing-opentelemetry` | latest | Bridge tracing spans to OpenTelemetry |
| `tracing-subscriber` | 0.3 | Log output formatting |
| `metrics` + `metrics-exporter-prometheus` | latest | Prometheus `/metrics` endpoint |
| `tokio-graceful-shutdown` | latest | Clean shutdown handling |
| `clap` | 4.x | CLI arg parsing (port, NATS URL, config overrides) |
| `thiserror` | 2.x | Typed errors in common crate |
| `diesel` | 2.3 (`postgres` + `r2d2` features) | ORM and query builder (aggregator) |
| `diesel_migrations` | 2.3 | Embedded migrations, run on startup |
| `deadpool-diesel` | latest | Async-aware connection pool for Diesel + Tokio |

### Infrastructure

| Tool | Version | Notes |
|---|---|---|
| Docker | latest stable | Multi-stage Rust builds |
| k3d | latest | Local k8s (wraps k3s in Docker) |
| NATS Server | 2.10+ | Helm chart, single node dev / 3-node cluster |
| Prometheus + Grafana | kube-prometheus-stack Helm chart | Metrics + dashboards |
| Jaeger | all-in-one | Helm chart, trace visualization |
| OTel Collector | latest | Receives OTLP from services, exports to Jaeger |
| oha | latest | HTTP benchmarking |
| NATS CLI | latest | NATS benchmarking (`nats bench`) |
| PostgreSQL | 16+ | Helm chart (bitnami/postgresql), aggregator persistence |

## Database (Aggregator)

The aggregator is the only service that uses a database. Diesel + PostgreSQL.

### Schema

```sql
-- Initial migration: results table
CREATE TABLE results (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    request_id UUID NOT NULL,
    task_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    enrichment JSONB,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processor_id TEXT NOT NULL
);

CREATE INDEX idx_results_task_type ON results(task_type);
CREATE INDEX idx_results_processed_at ON results(processed_at);
```

### Async Integration Pattern

Diesel is synchronous. In the Tokio runtime, all Diesel calls go through a blocking-aware pool:

```rust
// Using deadpool-diesel for async-safe connection pooling
let pool: deadpool_diesel::postgres::Pool = /* ... */;

// Every DB call uses interact() which runs on a blocking thread
let results = pool.get().await?
    .interact(|conn| {
        results::table
            .filter(results::task_type.eq(task_type))
            .load::<Result>(conn)
    })
    .await??;
```

**Rule**: Never call `pool.get().interact()` outside of this pattern. A bare `diesel::Connection` used directly on a Tokio task will block the runtime.

### Migrations

Managed via `diesel_cli`:

```sh
# Install diesel CLI (one-time)
cargo install diesel_cli --no-default-features --features postgres

# Create a migration
diesel migration generate create_results --migration-dir crates/aggregator/migrations

# Run migrations
diesel migration run --migration-dir crates/aggregator/migrations
```

Migrations also run embedded on startup via `diesel_migrations::embed_migrations!()` so the aggregator is self-bootstrapping on fresh deploys.

## Container Strategy

Single parameterized multi-stage Dockerfile for all services:

```dockerfile
# Builder
FROM rust:1.90-bookworm AS builder
ARG CRATE_NAME
WORKDIR /app
COPY . .
RUN cargo build --release -p ${CRATE_NAME}

# Runtime
FROM debian:bookworm-slim
ARG CRATE_NAME
COPY --from=builder /app/target/release/${CRATE_NAME} /usr/local/bin/service
CMD ["service"]
```

Target image size: ~20-30MB per service.

## Kubernetes Resources

Per-service:

| Resource | Purpose |
|---|---|
| `Deployment` | Pod management. Processor: `replicas: 3`. Others: `replicas: 1`. |
| `Service` (ClusterIP) | Internal DNS for HTTP services (gateway, aggregator) |
| `Service` (NodePort or Ingress) | External access to gateway + aggregator query API |
| `HorizontalPodAutoscaler` | Optional: scale processor based on CPU or custom NATS queue depth metric |

NATS: StatefulSet or Helm chart. Single node for dev, 3-node cluster for resilience testing.

## Observability

### Metrics

Each service exposes a Prometheus `/metrics` endpoint via `metrics-exporter-prometheus`:

- `http_requests_total{method, path, status}` (gateway, aggregator)
- `http_request_duration_seconds` histogram (gateway, aggregator)
- `nats_messages_published_total{subject}` / `nats_messages_received_total{subject}`
- `processing_duration_seconds` histogram (processor)
- Custom business metrics as needed

Prometheus scrapes all pods via k8s service discovery annotations.

### Tracing

OpenTelemetry trace context propagated through NATS message headers:

```
Gateway creates root span
  → injects trace context into NATS message headers
    → Processor extracts context, creates child span
      → injects into request-reply headers
        → Enricher extracts context, creates child span, responds
    → Processor publishes result with trace context
      → Aggregator extracts context, creates child span
```

All spans land in Jaeger via OTel Collector. One trace shows the full lifecycle of a request across all 4 services.

### Grafana Dashboards

Pre-built dashboard covering:
- Request rate (gateway inbound, per service)
- Latency percentiles (p50, p95, p99) per service
- NATS message rates (published / received / pending)
- Error rates
- Pod CPU / memory usage
- Processor queue depth

## Benchmarking & Stress Testing

### Tools

| Tool | What It Tests |
|---|---|
| `oha` | HTTP load against gateway |
| `nats bench` (NATS CLI) | Raw NATS pub/sub throughput |
| Custom Rust harness | End-to-end pipeline latency |

### Scenarios

1. **Baseline throughput**: Sustained load at increasing RPS (100 → 500 → 1000 → 5000). Measure gateway acceptance rate, processor throughput, aggregator receipt rate. Find the bottleneck.

2. **Horizontal scaling validation**: Run scenario 1 with processor replicas = 1, 3, 5. Confirm throughput scales linearly (or identify where it doesn't).

3. **Latency under load**: p50/p95/p99 at 80% of max throughput. Use distributed traces to identify where latency originates (gateway? NATS? processor? enricher round-trip?).

4. **Stress/soak test**: Sustained load at ~80% capacity for 10+ minutes. Watch for memory leaks, connection pool exhaustion, NATS slow consumers, pod restarts.

5. **Failure injection**: Kill a processor pod mid-test. Verify NATS rebalances to remaining consumers. Measure recovery time via Grafana.

### Success Criteria

- Can identify the bottleneck at each scale point
- Horizontal scaling shows measurable throughput improvement
- Have p50/p95/p99 latency numbers at each scale
- System recovers from pod failure without data loss
- All results captured as JSON for comparison across runs

## Non-Goals

- Production-ready authentication/authorization (out of scope for learning)
- Multi-cluster or multi-region deployment
- CI/CD pipeline (manual deploy via scripts is fine)
- TLS between services (k8s-internal traffic is sufficient for learning)
