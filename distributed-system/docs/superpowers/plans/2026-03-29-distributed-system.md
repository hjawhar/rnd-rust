# Distributed System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a distributed system with 4 Rust services communicating via NATS, deployed on Kubernetes, with Prometheus/Grafana/Jaeger observability and benchmarking.

**Architecture:** API Gateway accepts HTTP requests and publishes to NATS. Processor workers (competing consumers) process tasks and call Enricher via NATS request-reply. Results are published to Aggregator which persists to PostgreSQL via Diesel and exposes a query API. All services emit metrics to Prometheus and traces to Jaeger via OpenTelemetry.

**Tech Stack:** Rust (edition 2024), Tokio, Axum 0.8, async-nats 0.46, Diesel 2.3 + PostgreSQL, OpenTelemetry 0.30, Prometheus, Grafana, Jaeger, Docker, k3d/Kubernetes.

**Spec:** `docs/superpowers/specs/2026-03-29-distributed-system-design.md`

---

## File Structure

```
distributed-system/
├── Cargo.toml                          # workspace root
├── crates/
│   ├── common/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                  # re-exports
│   │       ├── error.rs                # shared error types
│   │       ├── messages.rs             # TaskRequest, TaskResult, EnrichmentRequest/Response
│   │       ├── nats.rs                 # connect_nats(), trace context inject/extract helpers
│   │       └── telemetry.rs            # init_tracing(), init_metrics(), MetricsState
│   ├── gateway/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── main.rs                # config, axum server, routes, handlers, NATS publish
│   ├── processor/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── main.rs                # config, NATS consumer loop, processing, enrich call, result publish
│   ├── enricher/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── main.rs                # config, NATS request-reply handler
│   └── aggregator/
│       ├── Cargo.toml
│       ├── diesel.toml
│       ├── migrations/
│       │   └── 00000000000000_create_results/
│       │       ├── up.sql
│       │       └── down.sql
│       └── src/
│           ├── main.rs                # config, setup, NATS consumer + axum server
│           ├── schema.rs              # diesel table! macro (auto-generated)
│           ├── models.rs              # NewResult, QueryableResult
│           ├── db.rs                  # insert_result(), query_results(), query_stats()
│           └── routes.rs              # GET /results, GET /stats
├── deploy/
│   ├── docker/
│   │   └── Dockerfile                 # shared multi-stage, parameterized by CRATE_NAME
│   └── k8s/
│       ├── namespace.yaml
│       ├── nats/
│       │   └── deployment.yaml
│       ├── postgres/
│       │   └── deployment.yaml
│       ├── gateway/
│       │   └── deployment.yaml
│       ├── processor/
│       │   └── deployment.yaml
│       ├── enricher/
│       │   └── deployment.yaml
│       ├── aggregator/
│       │   └── deployment.yaml
│       └── observability/
│           ├── prometheus-values.yaml
│           ├── jaeger.yaml
│           └── otel-collector.yaml
├── bench/
│   ├── baseline.sh
│   ├── scaling.sh
│   └── stress.sh
├── scripts/
│   ├── cluster-up.sh
│   ├── cluster-down.sh
│   ├── build-images.sh
│   └── deploy.sh
├── docker-compose.yaml                # local dev: NATS + PostgreSQL
└── AGENTS.md
```

---

## Phase 1: Project Foundation

### Task 1: Workspace Setup

**Files:**
- Modify: `Cargo.toml` (convert to workspace root)
- Delete: `src/main.rs` (replaced by per-crate binaries)
- Create: `crates/common/Cargo.toml`
- Create: `crates/common/src/lib.rs`
- Create: `crates/gateway/Cargo.toml`
- Create: `crates/gateway/src/main.rs`
- Create: `crates/processor/Cargo.toml`
- Create: `crates/processor/src/main.rs`
- Create: `crates/enricher/Cargo.toml`
- Create: `crates/enricher/src/main.rs`
- Create: `crates/aggregator/Cargo.toml`
- Create: `crates/aggregator/src/main.rs`

- [ ] **Step 1: Convert root to workspace Cargo.toml**

Replace `Cargo.toml` entirely:

```toml
[workspace]
resolver = "3"
members = [
    "crates/common",
    "crates/gateway",
    "crates/processor",
    "crates/enricher",
    "crates/aggregator",
]

[workspace.package]
version = "0.1.0"
edition = "2024"

[workspace.dependencies]
# Async runtime
tokio = { version = "1.50", features = ["full"] }

# HTTP framework
axum = "0.8"
tower-http = { version = "0.6", features = ["cors", "trace"] }

# NATS messaging
async-nats = "0.46"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Observability
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
opentelemetry = "0.30"
opentelemetry-sdk = { version = "0.30", features = ["rt-tokio"] }
opentelemetry-otlp = "0.30"
tracing-opentelemetry = "0.29"
metrics = "0.24"
metrics-exporter-prometheus = "0.16"

# Database (aggregator only)
diesel = { version = "2.3", features = ["postgres", "uuid", "chrono", "serde_json"] }
diesel_migrations = "2.3"
deadpool-diesel = { version = "0.6", features = ["postgres"] }

# Utilities
clap = { version = "4", features = ["derive", "env"] }
thiserror = "2"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
```

- [ ] **Step 2: Delete old src/main.rs**

```bash
rm src/main.rs && rmdir src
```

- [ ] **Step 3: Create common crate**

`crates/common/Cargo.toml`:
```toml
[package]
name = "common"
version.workspace = true
edition.workspace = true

[dependencies]
async-nats.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
uuid.workspace = true
chrono.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
opentelemetry.workspace = true
opentelemetry-sdk.workspace = true
opentelemetry-otlp.workspace = true
tracing-opentelemetry.workspace = true
metrics.workspace = true
metrics-exporter-prometheus.workspace = true
tokio.workspace = true
```

`crates/common/src/lib.rs`:
```rust
pub mod error;
pub mod messages;
pub mod nats;
pub mod telemetry;
```

Create placeholder modules so it compiles:

`crates/common/src/error.rs`:
```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("NATS error: {0}")]
    Nats(#[from] async_nats::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("request failed: {0}")]
    Request(String),
}
```

`crates/common/src/messages.rs`:
```rust
// Populated in Task 2
```

`crates/common/src/nats.rs`:
```rust
// Populated in Task 3
```

`crates/common/src/telemetry.rs`:
```rust
// Populated in Task 4
```

- [ ] **Step 4: Create service crate skeletons**

`crates/gateway/Cargo.toml`:
```toml
[package]
name = "gateway"
version.workspace = true
edition.workspace = true

[dependencies]
common = { path = "../common" }
axum.workspace = true
tower-http.workspace = true
tokio.workspace = true
async-nats.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
clap.workspace = true
uuid.workspace = true
metrics.workspace = true
```

`crates/gateway/src/main.rs`:
```rust
fn main() {
    println!("gateway");
}
```

`crates/processor/Cargo.toml`:
```toml
[package]
name = "processor"
version.workspace = true
edition.workspace = true

[dependencies]
common = { path = "../common" }
tokio.workspace = true
axum.workspace = true
async-nats.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
clap.workspace = true
uuid.workspace = true
chrono.workspace = true
metrics.workspace = true
```

`crates/processor/src/main.rs`:
```rust
fn main() {
    println!("processor");
}
```

`crates/enricher/Cargo.toml`:
```toml
[package]
name = "enricher"
version.workspace = true
edition.workspace = true

[dependencies]
common = { path = "../common" }
tokio.workspace = true
axum.workspace = true
async-nats.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
clap.workspace = true
uuid.workspace = true
chrono.workspace = true
metrics.workspace = true
```

`crates/enricher/src/main.rs`:
```rust
fn main() {
    println!("enricher");
}
```

`crates/aggregator/Cargo.toml`:
```toml
[package]
name = "aggregator"
version.workspace = true
edition.workspace = true

[dependencies]
common = { path = "../common" }
axum.workspace = true
tower-http.workspace = true
tokio.workspace = true
async-nats.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
clap.workspace = true
uuid.workspace = true
chrono.workspace = true
diesel.workspace = true
diesel_migrations.workspace = true
deadpool-diesel.workspace = true
metrics.workspace = true
```

`crates/aggregator/src/main.rs`:
```rust
fn main() {
    println!("aggregator");
}
```

- [ ] **Step 5: Verify workspace compiles**

```bash
cargo build
```

Expected: all 5 crates compile successfully.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: initialize cargo workspace with 5 crates"
```

---

### Task 2: Common Crate — Shared Message Types

**Files:**
- Modify: `crates/common/src/messages.rs`

- [ ] **Step 1: Define shared message types**

`crates/common/src/messages.rs`:
```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Client submits this to the gateway via HTTP POST /tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRequest {
    pub task_type: String,
    pub data: serde_json::Value,
}

/// Gateway publishes this to NATS subject `tasks.<task_type>`.
/// Wraps the client request with a unique ID and timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMessage {
    pub request_id: Uuid,
    pub task_type: String,
    pub data: serde_json::Value,
    pub submitted_at: DateTime<Utc>,
}

/// Processor sends this to Enricher via NATS request-reply on `enrich.request`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentRequest {
    pub request_id: Uuid,
    pub task_type: String,
    pub data: serde_json::Value,
}

/// Enricher responds with this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentResponse {
    pub request_id: Uuid,
    pub tags: Vec<String>,
    pub geo: GeoPoint,
    pub enriched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoPoint {
    pub lat: f64,
    pub lon: f64,
}

/// Processor publishes this to NATS subject `results.<task_type>`.
/// Contains original data + enrichment + processing metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub request_id: Uuid,
    pub task_type: String,
    pub payload: serde_json::Value,
    pub enrichment: EnrichmentResponse,
    pub processor_id: String,
    pub processed_at: DateTime<Utc>,
}

/// Gateway returns this to the client after accepting a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAccepted {
    pub request_id: Uuid,
    pub status: &'static str,
}

/// Aggregator returns this from GET /stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregationStats {
    pub total_results: i64,
    pub by_task_type: Vec<TaskTypeCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTypeCount {
    pub task_type: String,
    pub count: i64,
}

/// NATS subject helpers.
pub mod subjects {
    /// Returns `tasks.<task_type>` for publishing.
    pub fn task_subject(task_type: &str) -> String {
        format!("tasks.{task_type}")
    }

    /// Wildcard subscription for all task types.
    pub const TASKS_ALL: &str = "tasks.*";

    /// Queue group name for competing processor consumers.
    pub const PROCESSOR_QUEUE_GROUP: &str = "processors";

    /// Subject for enrichment request-reply.
    pub const ENRICH_REQUEST: &str = "enrich.request";

    /// Returns `results.<task_type>` for publishing.
    pub fn result_subject(task_type: &str) -> String {
        format!("results.{task_type}")
    }

    /// Wildcard subscription for all result types.
    pub const RESULTS_ALL: &str = "results.*";
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo build -p common
```

Expected: compiles with no errors.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(common): add shared message types and NATS subjects"
```

---

### Task 3: Common Crate — NATS Connection Factory

**Files:**
- Modify: `crates/common/src/nats.rs`

- [ ] **Step 1: Implement NATS connection helper**

`crates/common/src/nats.rs`:
```rust
use async_nats::Client;
use tracing::info;

/// Connect to NATS with retry-friendly defaults.
/// `service_name` is used for logging context.
pub async fn connect_nats(url: &str, service_name: &str) -> Result<Client, async_nats::Error> {
    let client = async_nats::ConnectOptions::new()
        .name(service_name)
        .connect(url)
        .await?;

    info!(service = service_name, url = url, "connected to NATS");
    Ok(client)
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo build -p common
```

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(common): add NATS connection factory"
```

---

### Task 4: Common Crate — Telemetry Bootstrap

**Files:**
- Modify: `crates/common/src/telemetry.rs`

- [ ] **Step 1: Implement tracing + metrics initialization**

`crates/common/src/telemetry.rs`:
```rust
use metrics_exporter_prometheus::PrometheusBuilder;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

/// Handle to the Prometheus metrics recorder. Call `render()` to produce
/// the text exposition format for the `/metrics` endpoint.
pub struct MetricsHandle {
    handle: metrics_exporter_prometheus::PrometheusHandle,
}

impl MetricsHandle {
    pub fn render(&self) -> String {
        self.handle.render()
    }
}

/// Initialize tracing subscriber with JSON or pretty formatting based on
/// the `RUST_LOG` env var (defaults to `info`).
///
/// Returns a `MetricsHandle` for exposing Prometheus metrics.
///
/// Call this once at service startup before any tracing macros fire.
pub fn init_telemetry(service_name: &str) -> MetricsHandle {
    // Prometheus metrics recorder
    let prometheus_handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus recorder");

    // Tracing subscriber: structured JSON in production, pretty in dev
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .json();

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();

    tracing::info!(service = service_name, "telemetry initialized");

    MetricsHandle {
        handle: prometheus_handle,
    }
}
```

Note: OpenTelemetry distributed tracing (OTLP export, trace context propagation through NATS headers) is added in Phase 5 (Task 15). This task sets up the foundation (structured logging + Prometheus metrics).

- [ ] **Step 2: Verify it compiles**

```bash
cargo build -p common
```

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(common): add telemetry bootstrap (tracing + prometheus metrics)"
```

---

### Task 5: Local Dev Infrastructure (Docker Compose)

**Files:**
- Create: `docker-compose.yaml`

- [ ] **Step 1: Create Docker Compose for local NATS + PostgreSQL**

`docker-compose.yaml`:
```yaml
services:
  nats:
    image: nats:2.10-alpine
    ports:
      - "4222:4222"   # client connections
      - "8222:8222"   # HTTP monitoring
    command: ["--js"]  # enable JetStream (useful for future features)

  postgres:
    image: postgres:16-alpine
    ports:
      - "5432:5432"
    environment:
      POSTGRES_USER: distributed
      POSTGRES_PASSWORD: distributed
      POSTGRES_DB: aggregator
    volumes:
      - pgdata:/var/lib/postgresql/data

volumes:
  pgdata:
```

- [ ] **Step 2: Start and verify services**

```bash
docker compose up -d
docker compose ps
```

Expected: both `nats` and `postgres` containers running.

Verify NATS:
```bash
curl -s http://localhost:8222/varz | head -5
```

Verify Postgres:
```bash
docker compose exec postgres psql -U distributed -d aggregator -c "SELECT 1;"
```

- [ ] **Step 3: Commit**

```bash
git add docker-compose.yaml
git commit -m "infra: add docker-compose for local NATS + PostgreSQL"
```

---

## Phase 2: Gateway Service

### Task 6: Gateway — HTTP Server with Health and Metrics

**Files:**
- Modify: `crates/gateway/src/main.rs`

- [ ] **Step 1: Implement gateway skeleton with /health and /metrics**

`crates/gateway/src/main.rs`:
```rust
use std::{net::SocketAddr, sync::Arc};

use axum::{Router, extract::State, http::StatusCode, routing::get};
use clap::Parser;
use common::telemetry::{MetricsHandle, init_telemetry};
use tracing::info;

#[derive(Parser)]
struct Config {
    /// Address to bind the HTTP server
    #[arg(long, env = "GATEWAY_ADDR", default_value = "0.0.0.0:3000")]
    addr: SocketAddr,

    /// NATS server URL
    #[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
    nats_url: String,
}

struct AppState {
    nats: async_nats::Client,
    metrics: MetricsHandle,
}

#[tokio::main]
async fn main() {
    let config = Config::parse();
    let metrics = init_telemetry("gateway");

    let nats = common::nats::connect_nats(&config.nats_url, "gateway")
        .await
        .expect("failed to connect to NATS");

    let state = Arc::new(AppState { nats, metrics });

    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics_handler))
        .with_state(state);

    info!(addr = %config.addr, "gateway listening");
    let listener = tokio::net::TcpListener::bind(config.addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> String {
    state.metrics.render()
}
```

- [ ] **Step 2: Verify it compiles and starts**

```bash
cargo build -p gateway
```

With NATS running (`docker compose up -d`):
```bash
cargo run -p gateway &
curl -s http://localhost:3000/health
# Expected: 200 OK (empty body)
curl -s http://localhost:3000/metrics
# Expected: Prometheus text output
kill %1
```

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(gateway): HTTP server with /health and /metrics endpoints"
```

---

### Task 7: Gateway — Task Submission Endpoint

**Files:**
- Modify: `crates/gateway/src/main.rs`

- [ ] **Step 1: Add POST /tasks endpoint**

Add these imports to the top of `crates/gateway/src/main.rs`:
```rust
use axum::{Json, routing::post};
use common::messages::{TaskAccepted, TaskMessage, TaskRequest, subjects};
use chrono::Utc;
use uuid::Uuid;
```

Add the route to the Router:
```rust
    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics_handler))
        .route("/tasks", post(submit_task))
        .with_state(state);
```

Add the handler:
```rust
async fn submit_task(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TaskRequest>,
) -> Result<(StatusCode, Json<TaskAccepted>), (StatusCode, String)> {
    let start = std::time::Instant::now();
    let request_id = Uuid::new_v4();

    let result = async {
        let message = TaskMessage {
            request_id,
            task_type: request.task_type.clone(),
            data: request.data,
            submitted_at: Utc::now(),
        };

        let payload = serde_json::to_vec(&message)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let subject = subjects::task_subject(&request.task_type);

        state
            .nats
            .publish(subject.clone(), payload.into())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        tracing::info!(
            request_id = %request_id,
            task_type = %request.task_type,
            subject = %subject,
            "task published to NATS"
        );

        Ok((
            StatusCode::ACCEPTED,
            Json(TaskAccepted {
                request_id,
                status: "accepted",
            }),
        ))
    }
    .await;

    // Record metrics on both success and error paths
    let status = match &result {
        Ok((code, _)) => code.as_str(),
        Err((code, _)) => code.as_str(),
    };
    metrics::counter!("http_requests_total", "method" => "POST", "path" => "/tasks", "status" => status.to_string()).increment(1);
    metrics::histogram!("http_request_duration_seconds", "method" => "POST", "path" => "/tasks").record(start.elapsed().as_secs_f64());

    result
}
```

- [ ] **Step 2: Test the endpoint manually**

With NATS running:
```bash
cargo run -p gateway &
curl -s -X POST http://localhost:3000/tasks \
  -H "Content-Type: application/json" \
  -d '{"task_type":"compute","data":{"value":42}}'
```

Expected: `{"request_id":"<uuid>","status":"accepted"}` with HTTP 202.

Check gateway logs for `"task published to NATS"` message.

```bash
kill %1
```

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(gateway): add POST /tasks endpoint with NATS publishing"
```

---

## Phase 3: Processor Service

### Task 8: Processor — NATS Consumer with Simulated Processing

**Files:**
- Modify: `crates/processor/src/main.rs`

- [ ] **Step 1: Implement processor with NATS consumer loop**

`crates/processor/src/main.rs`:
```rust
use std::{net::SocketAddr, sync::Arc, time::Duration};

use async_nats::Message;
use clap::Parser;
use axum::{Router, routing::get};
use common::{
    messages::{
        EnrichmentRequest, EnrichmentResponse, TaskMessage, TaskResult, subjects,
    },
    telemetry::init_telemetry,
};
use chrono::Utc;
use tracing::{error, info, warn};
use uuid::Uuid;

#[derive(Parser)]
struct Config {
    /// NATS server URL
    #[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
    nats_url: String,

    /// Address for metrics HTTP server
    #[arg(long, env = "METRICS_ADDR", default_value = "0.0.0.0:9090")]
    metrics_addr: SocketAddr,

    /// Unique processor instance ID (defaults to random UUID)
    #[arg(long, env = "PROCESSOR_ID")]
    processor_id: Option<String>,

    /// Simulated processing delay in milliseconds
    #[arg(long, env = "PROCESSING_DELAY_MS", default_value = "50")]
    processing_delay_ms: u64,

    /// Timeout for enrichment request-reply in milliseconds
    #[arg(long, env = "ENRICH_TIMEOUT_MS", default_value = "2000")]
    enrich_timeout_ms: u64,
}

#[tokio::main]
async fn main() {
    let config = Config::parse();
    let metrics = Arc::new(init_telemetry("processor"));

    let processor_id = config
        .processor_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let nats = common::nats::connect_nats(&config.nats_url, &format!("processor-{processor_id}"))
        .await
        .expect("failed to connect to NATS");

    info!(processor_id = %processor_id, "processor starting");

    // Spawn lightweight metrics HTTP server
    {
        let metrics = metrics.clone();
        let addr = config.metrics_addr;
        tokio::spawn(async move {
            let app = Router::new()
                .route("/metrics", get(move || {
                    let m = metrics.clone();
                    async move { m.render() }
                }));
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            info!(addr = %addr, "metrics server listening");
            axum::serve(listener, app).await.unwrap();
        });
    }

    // Subscribe to tasks.* with queue group for competing consumers
    let mut subscriber = nats
        .queue_subscribe(subjects::TASKS_ALL, subjects::PROCESSOR_QUEUE_GROUP)
        .await
        .expect("failed to subscribe to task subjects");

    info!("subscribed to {} (queue group: {})", subjects::TASKS_ALL, subjects::PROCESSOR_QUEUE_GROUP);

    while let Some(msg) = subscriber.next().await {
        let nats = nats.clone();
        let processor_id = processor_id.clone();
        let processing_delay = Duration::from_millis(config.processing_delay_ms);
        let enrich_timeout = Duration::from_millis(config.enrich_timeout_ms);

        tokio::spawn(async move {
            if let Err(e) = process_message(msg, &nats, &processor_id, processing_delay, enrich_timeout).await {
                error!(error = %e, "failed to process message");
            }
        });
    }

    warn!("NATS subscription ended, shutting down");
}

async fn process_message(
    msg: Message,
    nats: &async_nats::Client,
    processor_id: &str,
    processing_delay: Duration,
    enrich_timeout: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let start = std::time::Instant::now();

    let task: TaskMessage = serde_json::from_slice(&msg.payload)?;

    info!(
        request_id = %task.request_id,
        task_type = %task.task_type,
        "processing task"
    );

    // Simulate CPU-bound work
    tokio::time::sleep(processing_delay).await;

    // Call enricher via NATS request-reply
    let enrich_req = EnrichmentRequest {
        request_id: task.request_id,
        task_type: task.task_type.clone(),
        data: task.data.clone(),
    };

    let enrich_payload = serde_json::to_vec(&enrich_req)?;

    let enrich_response = tokio::time::timeout(
        enrich_timeout,
        nats.request(subjects::ENRICH_REQUEST, enrich_payload.into()),
    )
    .await??;

    let enrichment: EnrichmentResponse = serde_json::from_slice(&enrich_response.payload)?;

    // Build result and publish
    let result = TaskResult {
        request_id: task.request_id,
        task_type: task.task_type.clone(),
        payload: task.data,
        enrichment,
        processor_id: processor_id.to_string(),
        processed_at: Utc::now(),
    };

    let result_payload = serde_json::to_vec(&result)?;
    let result_subject = subjects::result_subject(&task.task_type);

    nats.publish(result_subject.clone(), result_payload.into())
        .await?;

    let elapsed = start.elapsed();
    metrics::counter!("nats_messages_received_total", "subject" => "tasks.*").increment(1);
    metrics::histogram!("processing_duration_seconds").record(elapsed.as_secs_f64());
    metrics::counter!("nats_messages_published_total", "subject" => "results.*").increment(1);

    info!(
        request_id = %task.request_id,
        task_type = %task.task_type,
        elapsed_ms = elapsed.as_millis(),
        "task processed and result published"
    );

    Ok(())
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo build -p processor
```

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(processor): NATS consumer with simulated processing and enrich request-reply"
```

---

## Phase 4: Enricher Service

### Task 9: Enricher — NATS Request-Reply Handler

**Files:**
- Modify: `crates/enricher/src/main.rs`

- [ ] **Step 1: Implement enricher**

`crates/enricher/src/main.rs`:
```rust
use std::{net::SocketAddr, sync::Arc, time::Duration};

use clap::Parser;
use axum::{Router, routing::get};
use common::{
    messages::{EnrichmentRequest, EnrichmentResponse, GeoPoint, subjects},
    telemetry::init_telemetry,
};
use chrono::Utc;
use tracing::{error, info, warn};

#[derive(Parser)]
struct Config {
    /// NATS server URL
    #[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
    nats_url: String,

    /// Address for metrics HTTP server
    #[arg(long, env = "METRICS_ADDR", default_value = "0.0.0.0:9090")]
    metrics_addr: SocketAddr,

    /// Simulated enrichment latency in milliseconds (0 = no delay)
    #[arg(long, env = "ENRICH_DELAY_MS", default_value = "10")]
    enrich_delay_ms: u64,
}

#[tokio::main]
async fn main() {
    let config = Config::parse();
    let metrics = Arc::new(init_telemetry("enricher"));

    // Spawn lightweight metrics HTTP server
    {
        let metrics = metrics.clone();
        let addr = config.metrics_addr;
        tokio::spawn(async move {
            let app = Router::new()
                .route("/metrics", get(move || {
                    let m = metrics.clone();
                    async move { m.render() }
                }));
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            info!(addr = %addr, "metrics server listening");
            axum::serve(listener, app).await.unwrap();
        });
    }

    let nats = common::nats::connect_nats(&config.nats_url, "enricher")
        .await
        .expect("failed to connect to NATS");

    info!("enricher starting, subscribing to {}", subjects::ENRICH_REQUEST);

    let mut subscriber = nats
        .subscribe(subjects::ENRICH_REQUEST)
        .await
        .expect("failed to subscribe to enrich subject");

    while let Some(msg) = subscriber.next().await {
        let reply = match msg.reply {
            Some(ref r) => r.clone(),
            None => {
                warn!("received enrich request without reply subject, skipping");
                continue;
            }
        };

        let nats = nats.clone();
        let delay = Duration::from_millis(config.enrich_delay_ms);

        tokio::spawn(async move {
            if let Err(e) = handle_enrich_request(&msg.payload, &nats, &reply, delay).await {
                error!(error = %e, "failed to handle enrich request");
            }
        });
    }

    warn!("NATS subscription ended, shutting down");
}

async fn handle_enrich_request(
    payload: &[u8],
    nats: &async_nats::Client,
    reply: &async_nats::Subject,
    delay: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let request: EnrichmentRequest = serde_json::from_slice(payload)?;

    info!(request_id = %request.request_id, "enriching request");

    // Simulate external API latency
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }

    // Generate simulated enrichment data.
    // Uses request_id bytes for deterministic-ish but varied output.
    let id_bytes = request.request_id.as_bytes();
    let lat = (id_bytes[0] as f64 / 255.0) * 180.0 - 90.0;
    let lon = (id_bytes[1] as f64 / 255.0) * 360.0 - 180.0;

    let tags = vec![
        format!("type:{}", request.task_type),
        "source:enricher".to_string(),
        format!("priority:{}", if id_bytes[2] > 128 { "high" } else { "low" }),
    ];

    let response = EnrichmentResponse {
        request_id: request.request_id,
        tags,
        geo: GeoPoint { lat, lon },
        enriched_at: Utc::now(),
    };

    let response_payload = serde_json::to_vec(&response)?;
    nats.publish(reply.clone(), response_payload.into()).await?;

    metrics::counter!("nats_messages_received_total", "subject" => "enrich.request").increment(1);

    info!(request_id = %request.request_id, "enrichment response sent");

    Ok(())
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo build -p enricher
```

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(enricher): NATS request-reply handler with simulated enrichment"
```

---

## Phase 5: Aggregator Service

### Task 10: Aggregator — Diesel Setup and Migrations

**Files:**
- Create: `crates/aggregator/diesel.toml`
- Create: `crates/aggregator/migrations/00000000000000_create_results/up.sql`
- Create: `crates/aggregator/migrations/00000000000000_create_results/down.sql`
- Create: `crates/aggregator/src/schema.rs`
- Create: `crates/aggregator/src/models.rs`

- [ ] **Step 1: Create diesel.toml**

`crates/aggregator/diesel.toml`:
```toml
[print_schema]
file = "src/schema.rs"

[migrations_directory]
dir = "migrations"
```

- [ ] **Step 2: Create migration SQL files**

`crates/aggregator/migrations/00000000000000_create_results/up.sql`:
```sql
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

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

`crates/aggregator/migrations/00000000000000_create_results/down.sql`:
```sql
DROP TABLE IF EXISTS results;
```

- [ ] **Step 3: Write the Diesel schema manually**

Since we know the table structure, write `crates/aggregator/src/schema.rs` directly (this is what `diesel print-schema` would generate):

```rust
diesel::table! {
    results (id) {
        id -> Uuid,
        request_id -> Uuid,
        task_type -> Text,
        payload -> Jsonb,
        enrichment -> Nullable<Jsonb>,
        processed_at -> Timestamptz,
        processor_id -> Text,
    }
}
```

- [ ] **Step 4: Write Diesel models**

`crates/aggregator/src/models.rs`:
```rust
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::Serialize;
use uuid::Uuid;

use crate::schema::results;

/// For inserting a new result row.
#[derive(Insertable)]
#[diesel(table_name = results)]
pub struct NewResult {
    pub request_id: Uuid,
    pub task_type: String,
    pub payload: serde_json::Value,
    pub enrichment: Option<serde_json::Value>,
    pub processed_at: DateTime<Utc>,
    pub processor_id: String,
}

/// For querying result rows.
#[derive(Queryable, Selectable, Serialize)]
#[diesel(table_name = results)]
pub struct ResultRow {
    pub id: Uuid,
    pub request_id: Uuid,
    pub task_type: String,
    pub payload: serde_json::Value,
    pub enrichment: Option<serde_json::Value>,
    pub processed_at: DateTime<Utc>,
    pub processor_id: String,
}
```

- [ ] **Step 5: Verify it compiles**

```bash
cargo build -p aggregator
```

- [ ] **Step 6: Run the migration against local Postgres**

With Postgres running (`docker compose up -d`):
```bash
cd crates/aggregator
DATABASE_URL="postgres://distributed:distributed@localhost:5432/aggregator" \
  diesel migration run
cd ../..
```

Expected: `Running migration 00000000000000_create_results`

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(aggregator): diesel setup, migration, schema, and models"
```

---

### Task 11: Aggregator — Database Operations

**Files:**
- Create: `crates/aggregator/src/db.rs`

- [ ] **Step 1: Implement database operations**

`crates/aggregator/src/db.rs`:
```rust
use common::messages::TaskResult;
use deadpool_diesel::postgres::Pool;
use diesel::prelude::*;
use diesel::dsl::count_star;

use crate::models::{NewResult, ResultRow};
use crate::schema::results;

/// Insert a processed TaskResult into the database.
pub async fn insert_result(
    pool: &Pool,
    task_result: &TaskResult,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let new_result = NewResult {
        request_id: task_result.request_id,
        task_type: task_result.task_type.clone(),
        payload: task_result.payload.clone(),
        enrichment: Some(serde_json::to_value(&task_result.enrichment)?),
        processed_at: task_result.processed_at,
        processor_id: task_result.processor_id.clone(),
    };

    let conn = pool.get().await?;
    conn.interact(move |conn| {
        diesel::insert_into(results::table)
            .values(&new_result)
            .execute(conn)
    })
    .await??;

    Ok(())
}

/// Query recent results, optionally filtered by task_type.
/// Returns up to `limit` results ordered by processed_at descending.
pub async fn query_results(
    pool: &Pool,
    task_type: Option<&str>,
    limit: i64,
) -> Result<Vec<ResultRow>, Box<dyn std::error::Error + Send + Sync>> {
    let task_type = task_type.map(String::from);

    let conn = pool.get().await?;
    let rows = conn
        .interact(move |conn| {
            let mut query = results::table
                .order(results::processed_at.desc())
                .limit(limit)
                .into_boxed();

            if let Some(ref tt) = task_type {
                query = query.filter(results::task_type.eq(tt));
            }

            query.select(ResultRow::as_select()).load::<ResultRow>(conn)
        })
        .await??;

    Ok(rows)
}

/// Get count of results grouped by task_type, plus total count.
pub async fn query_stats(
    pool: &Pool,
) -> Result<(i64, Vec<(String, i64)>), Box<dyn std::error::Error + Send + Sync>> {
    let conn = pool.get().await?;

    let stats = conn
        .interact(|conn| {
            let total: i64 = results::table
                .select(count_star())
                .first(conn)?;

            let by_type: Vec<(String, i64)> = results::table
                .group_by(results::task_type)
                .select((results::task_type, count_star()))
                .order(count_star().desc())
                .load::<(String, i64)>(conn)?;

            Ok::<_, diesel::result::Error>((total, by_type))
        })
        .await??;

    Ok(stats)
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo build -p aggregator
```

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(aggregator): database operations (insert, query, stats)"
```

---

### Task 12: Aggregator — Query API Routes

**Files:**
- Create: `crates/aggregator/src/routes.rs`

- [ ] **Step 1: Implement query API**

`crates/aggregator/src/routes.rs`:
```rust
use std::sync::Arc;

use axum::{Json, extract::{Query, State}, http::StatusCode};
use common::messages::{AggregationStats, TaskTypeCount};
use serde::Deserialize;

use crate::AppState;
use crate::db;
use crate::models::ResultRow;

#[derive(Deserialize)]
pub struct ResultsQuery {
    pub task_type: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    50
}

/// GET /results?task_type=compute&limit=20
pub async fn get_results(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ResultsQuery>,
) -> Result<Json<Vec<ResultRow>>, (StatusCode, String)> {
    let results = db::query_results(&state.db_pool, params.task_type.as_deref(), params.limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(results))
}

/// GET /stats
pub async fn get_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AggregationStats>, (StatusCode, String)> {
    let (total, by_type) = db::query_stats(&state.db_pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(AggregationStats {
        total_results: total,
        by_task_type: by_type
            .into_iter()
            .map(|(task_type, count)| TaskTypeCount { task_type, count })
            .collect(),
    }))
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo build -p aggregator
```

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(aggregator): query API routes (GET /results, GET /stats)"
```

---

### Task 13: Aggregator — Main Entry Point (NATS Consumer + HTTP Server)

**Files:**
- Modify: `crates/aggregator/src/main.rs`

- [ ] **Step 1: Implement aggregator main with NATS consumer + axum server**

`crates/aggregator/src/main.rs`:
```rust
mod db;
mod models;
mod routes;
mod schema;

use std::{net::SocketAddr, sync::Arc};

use axum::{Router, extract::State, routing::get};
use clap::Parser;
use common::{
    messages::{TaskResult, subjects},
    telemetry::{MetricsHandle, init_telemetry},
};
use deadpool_diesel::postgres::{Manager, Pool};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use tracing::{error, info, warn};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

#[derive(Parser)]
struct Config {
    /// Address to bind the HTTP server
    #[arg(long, env = "AGGREGATOR_ADDR", default_value = "0.0.0.0:3001")]
    addr: SocketAddr,

    /// NATS server URL
    #[arg(long, env = "NATS_URL", default_value = "nats://localhost:4222")]
    nats_url: String,

    /// PostgreSQL connection URL
    #[arg(long, env = "DATABASE_URL", default_value = "postgres://distributed:distributed@localhost:5432/aggregator")]
    database_url: String,
}

pub struct AppState {
    pub db_pool: Pool,
    pub metrics: MetricsHandle,
}

#[tokio::main]
async fn main() {
    let config = Config::parse();
    let metrics = init_telemetry("aggregator");

    // Database pool
    let manager = Manager::new(&config.database_url, deadpool_diesel::Runtime::Tokio1);
    let db_pool = Pool::builder(manager)
        .build()
        .expect("failed to create database pool");

    // Run migrations on startup
    {
        let conn = db_pool.get().await.expect("failed to get DB connection for migrations");
        conn.interact(|conn| {
            conn.run_pending_migrations(MIGRATIONS)
                .expect("failed to run migrations");
        })
        .await
        .expect("migration interaction failed");
        info!("database migrations complete");
    }

    // NATS connection
    let nats = common::nats::connect_nats(&config.nats_url, "aggregator")
        .await
        .expect("failed to connect to NATS");

    let state = Arc::new(AppState { db_pool, metrics });

    // Spawn NATS consumer
    let consumer_state = state.clone();
    let consumer_nats = nats.clone();
    tokio::spawn(async move {
        run_nats_consumer(consumer_nats, consumer_state).await;
    });

    // HTTP server
    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics_handler))
        .route("/results", get(routes::get_results))
        .route("/stats", get(routes::get_stats))
        .with_state(state);

    info!(addr = %config.addr, "aggregator listening");
    let listener = tokio::net::TcpListener::bind(config.addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health() -> &'static str {
    "ok"
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> String {
    state.metrics.render()
}

async fn run_nats_consumer(nats: async_nats::Client, state: Arc<AppState>) {
    let mut subscriber = nats
        .subscribe(subjects::RESULTS_ALL)
        .await
        .expect("failed to subscribe to results");

    info!("subscribed to {}", subjects::RESULTS_ALL);

    while let Some(msg) = subscriber.next().await {
        let state = state.clone();

        tokio::spawn(async move {
            let result: TaskResult = match serde_json::from_slice(&msg.payload) {
                Ok(r) => r,
                Err(e) => {
                    error!(error = %e, "failed to deserialize task result");
                    return;
                }
            };

            info!(
                request_id = %result.request_id,
                task_type = %result.task_type,
                "received result, persisting to database"
            );

            if let Err(e) = db::insert_result(&state.db_pool, &result).await {
                error!(error = %e, request_id = %result.request_id, "failed to insert result");
                return;
            }

            metrics::counter!("nats_messages_received_total", "subject" => "results.*").increment(1);

            info!(request_id = %result.request_id, "result persisted");
        });
    }

    warn!("NATS subscription ended");
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo build -p aggregator
```

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(aggregator): main entry point with NATS consumer and HTTP query API"
```

---

### Task 14: Full Pipeline Integration Test (Local)

**Files:** None created — this is a manual verification task.

- [ ] **Step 1: Start infrastructure**

```bash
docker compose up -d
```

- [ ] **Step 2: Start all 4 services in separate terminals**

Terminal 1 (enricher — start first, processor depends on it):
```bash
cargo run -p enricher
```

Terminal 2 (processor):
```bash
cargo run -p processor
```

Terminal 3 (aggregator):
```bash
cargo run -p aggregator
```

Terminal 4 (gateway):
```bash
cargo run -p gateway
```

- [ ] **Step 3: Submit a task and verify the full pipeline**

```bash
# Submit a task
curl -s -X POST http://localhost:3000/tasks \
  -H "Content-Type: application/json" \
  -d '{"task_type":"compute","data":{"value":42}}' | jq .

# Wait a moment for processing
sleep 2

# Check aggregator stats
curl -s http://localhost:3001/stats | jq .

# Check aggregator results
curl -s http://localhost:3001/results | jq .
```

Expected:
- Gateway returns `{"request_id":"...","status":"accepted"}`
- Stats shows `total_results: 1` with `compute: 1`
- Results shows the full result with enrichment data

- [ ] **Step 4: Submit multiple tasks to test throughput**

```bash
for i in $(seq 1 10); do
  curl -s -X POST http://localhost:3000/tasks \
    -H "Content-Type: application/json" \
    -d "{\"task_type\":\"compute\",\"data\":{\"value\":$i}}" &
done
wait

sleep 3
curl -s http://localhost:3001/stats | jq .
```

Expected: `total_results: 11` (1 from step 3 + 10 from this step).

- [ ] **Step 5: Stop all services and commit any fixes**

If any fixes were required during integration testing, commit them:

```bash
git add -A
git commit -m "fix: integration test fixes from full pipeline verification"
```

---

## Phase 6: Distributed Tracing (OpenTelemetry)

### Task 15: Add OpenTelemetry Trace Propagation

**Files:**
- Modify: `crates/common/src/telemetry.rs`
- Modify: `crates/common/src/nats.rs`
- Modify: `crates/gateway/src/main.rs`
- Modify: `crates/processor/src/main.rs`
- Modify: `crates/enricher/src/main.rs`
- Modify: `crates/aggregator/src/main.rs`

This task adds OTLP trace export and trace context propagation through NATS message headers. It touches every service because each one needs to create/propagate spans.

- [ ] **Step 1: Update telemetry.rs to configure OpenTelemetry OTLP exporter**

Add to `crates/common/src/telemetry.rs`:

```rust
use metrics_exporter_prometheus::PrometheusBuilder;
use opentelemetry::global;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_sdk::{Resource, runtime, trace::SdkTracerProvider};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

pub struct MetricsHandle {
    handle: metrics_exporter_prometheus::PrometheusHandle,
}

impl MetricsHandle {
    pub fn render(&self) -> String {
        self.handle.render()
    }
}

/// Initialize tracing with optional OTLP export and Prometheus metrics.
///
/// Set `OTEL_EXPORTER_OTLP_ENDPOINT` to enable trace export (e.g., `http://localhost:4317`).
/// If not set, traces are only logged locally.
pub fn init_telemetry(service_name: &str) -> MetricsHandle {
    let prometheus_handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus recorder");

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .json();

    let otlp_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    if let Some(endpoint) = otlp_endpoint {
        let exporter = SpanExporter::builder()
            .with_tonic()
            .with_endpoint(&endpoint)
            .build()
            .expect("failed to create OTLP exporter");

        let resource = Resource::builder()
            .with_service_name(service_name.to_string())
            .build();

        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(resource)
            .build();

        let tracer = provider.tracer(service_name.to_string());

        let otel_layer = OpenTelemetryLayer::new(tracer);


        // CRITICAL: register the W3C TraceContext propagator globally.
        // Without this, inject/extract_trace_context are no-ops.
        global::set_text_map_propagator(TraceContextPropagator::new());

        registry.with(otel_layer).init();

        tracing::info!(service = service_name, endpoint = endpoint, "OTLP tracing enabled");
    } else {
        registry.init();
        tracing::info!(service = service_name, "telemetry initialized (no OTLP export)");
    }

    MetricsHandle {
        handle: prometheus_handle,
    }
}
```

- [ ] **Step 2: Add trace context inject/extract helpers to nats.rs**

Update `crates/common/src/nats.rs`:

```rust
use async_nats::Client;
use async_nats::HeaderMap;
use opentelemetry::propagation::{Extractor, Injector, TextMapPropagator};
use opentelemetry::global;
use tracing::info;
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub async fn connect_nats(url: &str, service_name: &str) -> Result<Client, async_nats::Error> {
    let client = async_nats::ConnectOptions::new()
        .name(service_name)
        .connect(url)
        .await?;

    info!(service = service_name, url = url, "connected to NATS");
    Ok(client)
}

/// Inject the current tracing span's context into NATS headers.
/// Call this before publishing a message to propagate the trace.
pub fn inject_trace_context(headers: &mut HeaderMap) {
    let context = tracing::Span::current().context();
    let propagator = opentelemetry::global::get_text_map_propagator(|p| {
        p.inject_context(&context, &mut NatsHeaderInjector(headers));
    });
}

/// Extract trace context from NATS headers and set it on the current span.
/// Call this at the start of a message handler.
pub fn extract_trace_context(headers: &HeaderMap) -> opentelemetry::Context {
    let propagator = global::get_text_map_propagator(|p| {
        p.extract(&NatsHeaderExtractor(headers))
    });
    propagator
}

struct NatsHeaderInjector<'a>(&'a mut HeaderMap);

impl Injector for NatsHeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key, value.as_str());
    }
}

struct NatsHeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for NatsHeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.iter().next()).map(|v| v.as_str())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.iter().map(|(k, _)| k.as_str()).collect()
    }
}
```

- [ ] **Step 3: Update gateway to inject trace context**

In `crates/gateway/src/main.rs`, update the `submit_task` handler to create a span and inject context into NATS headers:

Add import:
```rust
use common::nats::inject_trace_context;
use async_nats::HeaderMap;
```

Replace the NATS publish section in `submit_task`:
```rust
    let mut headers = HeaderMap::new();
    inject_trace_context(&mut headers);

    state
        .nats
        .publish_with_headers(subject.clone(), headers, payload.into())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
```

- [ ] **Step 4: Update processor to extract/inject trace context**

In `crates/processor/src/main.rs`, update `process_message` to extract context from incoming message and inject into outgoing messages. Add imports:

```rust
use common::nats::{inject_trace_context, extract_trace_context};
use async_nats::HeaderMap;
use tracing_opentelemetry::OpenTelemetrySpanExt;
```

At the start of `process_message`, after deserializing the task:
```rust
    // Extract trace context from incoming NATS headers
    if let Some(ref headers) = msg.headers {
        let parent_context = extract_trace_context(headers);
        tracing::Span::current().set_parent(parent_context);
    }
```

When publishing the enrich request:
```rust
    let mut enrich_headers = HeaderMap::new();
    inject_trace_context(&mut enrich_headers);

    let enrich_response = tokio::time::timeout(
        enrich_timeout,
        nats.request_with_headers(subjects::ENRICH_REQUEST, enrich_headers, enrich_payload.into()),
    )
    .await??;
```

When publishing the result:
```rust
    let mut result_headers = HeaderMap::new();
    inject_trace_context(&mut result_headers);

    nats.publish_with_headers(result_subject.clone(), result_headers, result_payload.into())
        .await?;
```

- [ ] **Step 5: Update enricher to extract trace context**

In `crates/enricher/src/main.rs`, add imports:

```rust
use async_nats::Message;
use common::nats::extract_trace_context;
use tracing_opentelemetry::OpenTelemetrySpanExt;
```

Change the `handle_enrich_request` signature to accept the full `Message` instead of just `&[u8]`:

```rust
async fn handle_enrich_request(
    msg: &Message,
    nats: &async_nats::Client,
    delay: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Extract trace context from incoming NATS headers
    if let Some(ref headers) = msg.headers {
        let parent_context = extract_trace_context(headers);
        tracing::Span::current().set_parent(parent_context);
    }

    let reply = msg.reply.as_ref().ok_or("missing reply subject")?;
    let request: EnrichmentRequest = serde_json::from_slice(&msg.payload)?;
    // ... rest of handler unchanged (enrichment logic + nats.publish to reply)
```

Update the spawn call in the main loop to pass the full message:

```rust
    while let Some(msg) = subscriber.next().await {
        let nats = nats.clone();
        let delay = Duration::from_millis(config.enrich_delay_ms);

        tokio::spawn(async move {
            if let Err(e) = handle_enrich_request(&msg, &nats, delay).await {
                error!(error = %e, "failed to handle enrich request");
            }
        });
    }
```

- [ ] **Step 6: Update aggregator to extract trace context**

In the NATS consumer loop in `crates/aggregator/src/main.rs`:

```rust
use common::nats::extract_trace_context;
use tracing_opentelemetry::OpenTelemetrySpanExt;
```

After deserializing the result:
```rust
    if let Some(ref headers) = msg.headers {
        let parent_context = extract_trace_context(headers);
        tracing::Span::current().set_parent(parent_context);
    }
```

- [ ] **Step 7: Verify everything compiles**

```bash
cargo build
```

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: add OpenTelemetry distributed tracing with NATS header propagation"
```

---

## Phase 7: Docker & Kubernetes

### Task 16: Dockerfile

**Files:**
- Create: `deploy/docker/Dockerfile`

- [ ] **Step 1: Create parameterized multi-stage Dockerfile**

`deploy/docker/Dockerfile`:
```dockerfile
# === Builder ===
FROM rust:1.90-bookworm AS builder

ARG CRATE_NAME

WORKDIR /app

# Copy workspace manifests first for layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/common/Cargo.toml crates/common/Cargo.toml
COPY crates/gateway/Cargo.toml crates/gateway/Cargo.toml
COPY crates/processor/Cargo.toml crates/processor/Cargo.toml
COPY crates/enricher/Cargo.toml crates/enricher/Cargo.toml
COPY crates/aggregator/Cargo.toml crates/aggregator/Cargo.toml

# Create dummy source files to build dependencies first (layer cache optimization)
RUN mkdir -p crates/common/src && echo "pub fn dummy() {}" > crates/common/src/lib.rs \
    && mkdir -p crates/gateway/src && echo "fn main() {}" > crates/gateway/src/main.rs \
    && mkdir -p crates/processor/src && echo "fn main() {}" > crates/processor/src/main.rs \
    && mkdir -p crates/enricher/src && echo "fn main() {}" > crates/enricher/src/main.rs \
    && mkdir -p crates/aggregator/src && echo "fn main() {}" > crates/aggregator/src/main.rs

# Build dependencies only (cached unless Cargo.toml changes)
RUN cargo build --release -p ${CRATE_NAME} || true

# Now copy real source code
COPY crates/ crates/

# Touch source files to invalidate the build cache for actual source
RUN find crates -name "*.rs" -exec touch {} +

# Build the actual binary
RUN cargo build --release -p ${CRATE_NAME}

# === Runtime ===
FROM debian:bookworm-slim

ARG CRATE_NAME

# Install runtime dependencies (libpq for diesel/postgres if aggregator)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libpq5 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/${CRATE_NAME} /usr/local/bin/service

# Copy migrations for aggregator (no-op for other services)
COPY crates/aggregator/migrations /app/migrations

EXPOSE 3000 3001

CMD ["service"]
```

- [ ] **Step 2: Test building one image**

```bash
docker build -f deploy/docker/Dockerfile --build-arg CRATE_NAME=gateway -t distributed-system/gateway:dev .
```

Expected: successful build.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "infra: add multi-stage Dockerfile for all services"
```

---

### Task 17: Kubernetes Manifests

**Files:**
- Create: `deploy/k8s/namespace.yaml`
- Create: `deploy/k8s/nats/deployment.yaml`
- Create: `deploy/k8s/postgres/deployment.yaml`
- Create: `deploy/k8s/gateway/deployment.yaml`
- Create: `deploy/k8s/processor/deployment.yaml`
- Create: `deploy/k8s/enricher/deployment.yaml`
- Create: `deploy/k8s/aggregator/deployment.yaml`

- [ ] **Step 1: Namespace**

`deploy/k8s/namespace.yaml`:
```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: distributed-system
```

- [ ] **Step 2: NATS deployment**

`deploy/k8s/nats/deployment.yaml`:
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: nats
  namespace: distributed-system
spec:
  replicas: 1
  selector:
    matchLabels:
      app: nats
  template:
    metadata:
      labels:
        app: nats
    spec:
      containers:
        - name: nats
          image: nats:2.10-alpine
          args: ["--js"]
          ports:
            - containerPort: 4222
              name: client
            - containerPort: 8222
              name: monitoring
---
apiVersion: v1
kind: Service
metadata:
  name: nats
  namespace: distributed-system
spec:
  selector:
    app: nats
  ports:
    - port: 4222
      targetPort: 4222
      name: client
    - port: 8222
      targetPort: 8222
      name: monitoring
```

- [ ] **Step 3: PostgreSQL deployment**

`deploy/k8s/postgres/deployment.yaml`:
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: postgres
  namespace: distributed-system
spec:
  replicas: 1
  selector:
    matchLabels:
      app: postgres
  template:
    metadata:
      labels:
        app: postgres
    spec:
      containers:
        - name: postgres
          image: postgres:16-alpine
          ports:
            - containerPort: 5432
          env:
            - name: POSTGRES_USER
              value: distributed
            - name: POSTGRES_PASSWORD
              value: distributed
            - name: POSTGRES_DB
              value: aggregator
---
apiVersion: v1
kind: Service
metadata:
  name: postgres
  namespace: distributed-system
spec:
  selector:
    app: postgres
  ports:
    - port: 5432
      targetPort: 5432
```

- [ ] **Step 4: Gateway deployment**

`deploy/k8s/gateway/deployment.yaml`:
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: gateway
  namespace: distributed-system
spec:
  replicas: 1
  selector:
    matchLabels:
      app: gateway
  template:
    metadata:
      labels:
        app: gateway
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/port: "3000"
        prometheus.io/path: "/metrics"
    spec:
      containers:
        - name: gateway
          image: distributed-system/gateway:dev
          ports:
            - containerPort: 3000
          env:
            - name: NATS_URL
              value: "nats://nats.distributed-system.svc.cluster.local:4222"
            - name: GATEWAY_ADDR
              value: "0.0.0.0:3000"
            - name: OTEL_EXPORTER_OTLP_ENDPOINT
              value: "http://otel-collector.distributed-system.svc.cluster.local:4317"
          readinessProbe:
            httpGet:
              path: /health
              port: 3000
            initialDelaySeconds: 5
            periodSeconds: 10
---
apiVersion: v1
kind: Service
metadata:
  name: gateway
  namespace: distributed-system
spec:
  type: NodePort
  selector:
    app: gateway
  ports:
    - port: 3000
      targetPort: 3000
      nodePort: 30000
```

- [ ] **Step 5: Processor deployment (3 replicas)**

`deploy/k8s/processor/deployment.yaml`:
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: processor
  namespace: distributed-system
spec:
  replicas: 3
  selector:
    matchLabels:
      app: processor
  template:
    metadata:
      labels:
        app: processor
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/port: "9090"
        prometheus.io/path: "/metrics"
    spec:
      containers:
        - name: processor
          image: distributed-system/processor:dev
          env:
            - name: NATS_URL
              value: "nats://nats.distributed-system.svc.cluster.local:4222"
            - name: PROCESSING_DELAY_MS
              value: "50"
            - name: ENRICH_TIMEOUT_MS
              value: "2000"
            - name: OTEL_EXPORTER_OTLP_ENDPOINT
              value: "http://otel-collector.distributed-system.svc.cluster.local:4317"
```

- [ ] **Step 6: Enricher deployment**

`deploy/k8s/enricher/deployment.yaml`:
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: enricher
  namespace: distributed-system
spec:
  replicas: 1
  selector:
    matchLabels:
      app: enricher
  template:
    metadata:
      labels:
        app: enricher
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/port: "9090"
        prometheus.io/path: "/metrics"
    spec:
      containers:
        - name: enricher
          image: distributed-system/enricher:dev
          env:
            - name: NATS_URL
              value: "nats://nats.distributed-system.svc.cluster.local:4222"
            - name: ENRICH_DELAY_MS
              value: "10"
            - name: OTEL_EXPORTER_OTLP_ENDPOINT
              value: "http://otel-collector.distributed-system.svc.cluster.local:4317"
```

- [ ] **Step 7: Aggregator deployment**

`deploy/k8s/aggregator/deployment.yaml`:
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: aggregator
  namespace: distributed-system
spec:
  replicas: 1
  selector:
    matchLabels:
      app: aggregator
  template:
    metadata:
      labels:
        app: aggregator
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/port: "3001"
        prometheus.io/path: "/metrics"
    spec:
      containers:
        - name: aggregator
          image: distributed-system/aggregator:dev
          ports:
            - containerPort: 3001
          env:
            - name: NATS_URL
              value: "nats://nats.distributed-system.svc.cluster.local:4222"
            - name: DATABASE_URL
              value: "postgres://distributed:distributed@postgres.distributed-system.svc.cluster.local:5432/aggregator"
            - name: AGGREGATOR_ADDR
              value: "0.0.0.0:3001"
            - name: OTEL_EXPORTER_OTLP_ENDPOINT
              value: "http://otel-collector.distributed-system.svc.cluster.local:4317"
          readinessProbe:
            httpGet:
              path: /health
              port: 3001
            initialDelaySeconds: 10
            periodSeconds: 10
---
apiVersion: v1
kind: Service
metadata:
  name: aggregator
  namespace: distributed-system
spec:
  type: NodePort
  selector:
    app: aggregator
  ports:
    - port: 3001
      targetPort: 3001
      nodePort: 30001
```

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "infra: add Kubernetes manifests for all services"
```

---

### Task 18: Dev Scripts (Cluster Lifecycle + Image Build + Deploy)

**Files:**
- Create: `scripts/cluster-up.sh`
- Create: `scripts/cluster-down.sh`
- Create: `scripts/build-images.sh`
- Create: `scripts/deploy.sh`

- [ ] **Step 1: Cluster up script**

`scripts/cluster-up.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="distributed-system"
REGISTRY_NAME="dsystem-registry"
REGISTRY_PORT="5111"

echo "==> Creating local registry..."
if ! docker inspect "$REGISTRY_NAME" &>/dev/null; then
    docker run -d --restart=always -p "${REGISTRY_PORT}:5000" --name "$REGISTRY_NAME" registry:2
fi

echo "==> Creating k3d cluster..."
k3d cluster create "$CLUSTER_NAME" \
    --registry-use "k3d-${REGISTRY_NAME}:${REGISTRY_PORT}" \
    --port "30000-30010:30000-30010@server:0" \
    --agents 2

echo "==> Cluster ready. Context set to k3d-${CLUSTER_NAME}"
kubectl cluster-info
```

- [ ] **Step 2: Cluster down script**

`scripts/cluster-down.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="distributed-system"
REGISTRY_NAME="dsystem-registry"

echo "==> Deleting k3d cluster..."
k3d cluster delete "$CLUSTER_NAME" 2>/dev/null || true

echo "==> Removing local registry..."
docker rm -f "$REGISTRY_NAME" 2>/dev/null || true

echo "==> Cluster and registry removed."
```

- [ ] **Step 3: Build images script**

`scripts/build-images.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail

REGISTRY="localhost:5111"
TAG="${1:-dev}"
SERVICES="gateway processor enricher aggregator"

for svc in $SERVICES; do
    echo "==> Building ${svc}..."
    docker build \
        -f deploy/docker/Dockerfile \
        --build-arg CRATE_NAME="$svc" \
        -t "${REGISTRY}/distributed-system/${svc}:${TAG}" \
        .

    echo "==> Pushing ${svc}..."
    docker push "${REGISTRY}/distributed-system/${svc}:${TAG}"
done

echo "==> All images built and pushed."
```

- [ ] **Step 4: Deploy script**

`scripts/deploy.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail

REGISTRY="localhost:5111"
TAG="${1:-dev}"

echo "==> Applying namespace..."
kubectl apply -f deploy/k8s/namespace.yaml

echo "==> Deploying infrastructure (NATS + PostgreSQL)..."
kubectl apply -f deploy/k8s/nats/
kubectl apply -f deploy/k8s/postgres/

echo "==> Waiting for NATS and PostgreSQL to be ready..."
kubectl -n distributed-system wait --for=condition=available deployment/nats --timeout=60s
kubectl -n distributed-system wait --for=condition=available deployment/postgres --timeout=60s

echo "==> Updating image references..."
for svc in gateway processor enricher aggregator; do
    sed "s|image: distributed-system/${svc}:dev|image: k3d-dsystem-registry:5111/distributed-system/${svc}:${TAG}|" \
        "deploy/k8s/${svc}/deployment.yaml" | kubectl apply -f -
done

echo "==> Waiting for services to be ready..."
for svc in gateway processor enricher aggregator; do
    kubectl -n distributed-system wait --for=condition=available "deployment/${svc}" --timeout=120s
done

echo "==> All services deployed."
echo "    Gateway:    http://localhost:30000"
echo "    Aggregator: http://localhost:30001"
```

- [ ] **Step 5: Make scripts executable**

```bash
chmod +x scripts/*.sh
```

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "infra: add dev scripts for k3d cluster lifecycle, image build, and deploy"
```

---

## Phase 8: Observability Stack

### Task 19: Observability Kubernetes Manifests

**Files:**
- Create: `deploy/k8s/observability/otel-collector.yaml`
- Create: `deploy/k8s/observability/jaeger.yaml`
- Create: `deploy/k8s/observability/prometheus-values.yaml`

- [ ] **Step 1: OpenTelemetry Collector**

`deploy/k8s/observability/otel-collector.yaml`:
```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: otel-collector-config
  namespace: distributed-system
data:
  config.yaml: |
    receivers:
      otlp:
        protocols:
          grpc:
            endpoint: 0.0.0.0:4317
          http:
            endpoint: 0.0.0.0:4318

    processors:
      batch:
        timeout: 1s
        send_batch_size: 1024

    exporters:
      otlp/jaeger:
        endpoint: jaeger-collector:4317
        tls:
          insecure: true

    service:
      pipelines:
        traces:
          receivers: [otlp]
          processors: [batch]
          exporters: [otlp/jaeger]
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: otel-collector
  namespace: distributed-system
spec:
  replicas: 1
  selector:
    matchLabels:
      app: otel-collector
  template:
    metadata:
      labels:
        app: otel-collector
    spec:
      containers:
        - name: otel-collector
          image: otel/opentelemetry-collector-contrib:latest
          args: ["--config=/etc/otel/config.yaml"]
          ports:
            - containerPort: 4317
              name: otlp-grpc
            - containerPort: 4318
              name: otlp-http
          volumeMounts:
            - name: config
              mountPath: /etc/otel
      volumes:
        - name: config
          configMap:
            name: otel-collector-config
---
apiVersion: v1
kind: Service
metadata:
  name: otel-collector
  namespace: distributed-system
spec:
  selector:
    app: otel-collector
  ports:
    - port: 4317
      targetPort: 4317
      name: otlp-grpc
    - port: 4318
      targetPort: 4318
      name: otlp-http
```

- [ ] **Step 2: Jaeger all-in-one**

`deploy/k8s/observability/jaeger.yaml`:
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: jaeger
  namespace: distributed-system
spec:
  replicas: 1
  selector:
    matchLabels:
      app: jaeger
  template:
    metadata:
      labels:
        app: jaeger
    spec:
      containers:
        - name: jaeger
          image: jaegertracing/all-in-one:latest
          ports:
            - containerPort: 16686
              name: ui
            - containerPort: 4317
              name: otlp-grpc
          env:
            - name: COLLECTOR_OTLP_ENABLED
              value: "true"
---
apiVersion: v1
kind: Service
metadata:
  name: jaeger-collector
  namespace: distributed-system
spec:
  selector:
    app: jaeger
  ports:
    - port: 4317
      targetPort: 4317
      name: otlp-grpc
---
apiVersion: v1
kind: Service
metadata:
  name: jaeger-ui
  namespace: distributed-system
spec:
  type: NodePort
  selector:
    app: jaeger
  ports:
    - port: 16686
      targetPort: 16686
      nodePort: 30002
```

- [ ] **Step 3: Prometheus + Grafana install instructions**

`deploy/k8s/observability/prometheus-values.yaml`:
```yaml
# Helm values for kube-prometheus-stack
# Install: helm install prometheus prometheus-community/kube-prometheus-stack \
#          -n distributed-system -f deploy/k8s/observability/prometheus-values.yaml

prometheus:
  prometheusSpec:
    podMonitorSelectorNilUsesHelmValues: false
    serviceMonitorSelectorNilUsesHelmValues: false
    # Scrape pods with prometheus.io/scrape annotation
    additionalScrapeConfigs:
      - job_name: 'distributed-system-pods'
        kubernetes_sd_configs:
          - role: pod
            namespaces:
              names:
                - distributed-system
        relabel_configs:
          - source_labels: [__meta_kubernetes_pod_annotation_prometheus_io_scrape]
            action: keep
            regex: true
          - source_labels: [__meta_kubernetes_pod_annotation_prometheus_io_path]
            action: replace
            target_label: __metrics_path__
            regex: (.+)
          - source_labels: [__address__, __meta_kubernetes_pod_annotation_prometheus_io_port]
            action: replace
            regex: ([^:]+)(?::\d+)?;(\d+)
            replacement: $1:$2
            target_label: __address__
          - source_labels: [__meta_kubernetes_pod_label_app]
            action: replace
            target_label: app

grafana:
  service:
    type: NodePort
    nodePort: 30003
  adminPassword: admin
```

- [ ] **Step 4: Update deploy.sh to include observability**

Add to `scripts/deploy.sh` before the "Updating image references" step:

```bash
echo "==> Deploying observability stack..."
kubectl apply -f deploy/k8s/observability/otel-collector.yaml
kubectl apply -f deploy/k8s/observability/jaeger.yaml

# Prometheus + Grafana via Helm (if not already installed)
if ! helm list -n distributed-system | grep -q prometheus; then
    helm repo add prometheus-community https://prometheus-community.github.io/helm-charts 2>/dev/null || true
    helm repo update
    helm install prometheus prometheus-community/kube-prometheus-stack \
        -n distributed-system \
        -f deploy/k8s/observability/prometheus-values.yaml \
        --wait --timeout 5m
fi
```

- [ ] **Step 5: Create Grafana dashboard ConfigMap**

`deploy/k8s/observability/grafana-dashboard.yaml`:
```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: distributed-system-dashboard
  namespace: distributed-system
  labels:
    grafana_dashboard: "1"
data:
  distributed-system.json: |
    {
      "dashboard": {
        "title": "Distributed System Overview",
        "panels": [
          {
            "title": "Gateway Request Rate",
            "type": "timeseries",
            "targets": [{"expr": "rate(http_requests_total{app=\"gateway\"}[1m])"}],
            "gridPos": {"h": 8, "w": 12, "x": 0, "y": 0}
          },
          {
            "title": "Request Latency (p50/p95/p99)",
            "type": "timeseries",
            "targets": [
              {"expr": "histogram_quantile(0.50, rate(http_request_duration_seconds_bucket{app=\"gateway\"}[1m]))", "legendFormat": "p50"},
              {"expr": "histogram_quantile(0.95, rate(http_request_duration_seconds_bucket{app=\"gateway\"}[1m]))", "legendFormat": "p95"},
              {"expr": "histogram_quantile(0.99, rate(http_request_duration_seconds_bucket{app=\"gateway\"}[1m]))", "legendFormat": "p99"}
            ],
            "gridPos": {"h": 8, "w": 12, "x": 12, "y": 0}
          },
          {
            "title": "NATS Messages Published/Received",
            "type": "timeseries",
            "targets": [
              {"expr": "rate(nats_messages_published_total[1m])", "legendFormat": "published {{subject}}"},
              {"expr": "rate(nats_messages_received_total[1m])", "legendFormat": "received {{subject}}"}
            ],
            "gridPos": {"h": 8, "w": 12, "x": 0, "y": 8}
          },
          {
            "title": "Processing Duration (p50/p95/p99)",
            "type": "timeseries",
            "targets": [
              {"expr": "histogram_quantile(0.50, rate(processing_duration_seconds_bucket[1m]))", "legendFormat": "p50"},
              {"expr": "histogram_quantile(0.95, rate(processing_duration_seconds_bucket[1m]))", "legendFormat": "p95"},
              {"expr": "histogram_quantile(0.99, rate(processing_duration_seconds_bucket[1m]))", "legendFormat": "p99"}
            ],
            "gridPos": {"h": 8, "w": 12, "x": 12, "y": 8}
          },
          {
            "title": "Error Rate",
            "type": "timeseries",
            "targets": [{"expr": "rate(http_requests_total{status!~\"2..\"}[1m])"}],
            "gridPos": {"h": 8, "w": 12, "x": 0, "y": 16}
          },
          {
            "title": "Pod CPU Usage",
            "type": "timeseries",
            "targets": [{"expr": "rate(container_cpu_usage_seconds_total{namespace=\"distributed-system\"}[1m])", "legendFormat": "{{pod}}"}],
            "gridPos": {"h": 8, "w": 12, "x": 12, "y": 16}
          }
        ],
        "schemaVersion": 39,
        "refresh": "5s"
      }
    }
```

Add to the deploy script's observability section:
```bash
kubectl apply -f deploy/k8s/observability/grafana-dashboard.yaml
```


- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "infra: add observability stack (OTel Collector, Jaeger, Prometheus, Grafana)"
```

---

## Phase 9: Benchmarking & Stress Testing

### Task 20: Benchmark Scripts

**Files:**
- Create: `bench/baseline.sh`
- Create: `bench/scaling.sh`
- Create: `bench/stress.sh`

- [ ] **Step 1: Baseline throughput benchmark**

`bench/baseline.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail

# Baseline throughput benchmark
# Requires: oha (cargo install oha)
# Usage: ./bench/baseline.sh [GATEWAY_URL]

GATEWAY_URL="${1:-http://localhost:30000}"
RESULTS_DIR="bench/results/$(date +%Y%m%d-%H%M%S)-baseline"
mkdir -p "$RESULTS_DIR"

PAYLOAD='{"task_type":"compute","data":{"value":42}}'

for RPS in 100 500 1000 5000; do
    echo "==> Testing at ${RPS} RPS for 30 seconds..."

    oha -z 30s \
        --rate "$RPS" \
        -m POST \
        -H "Content-Type: application/json" \
        -d "$PAYLOAD" \
        --json \
        "${GATEWAY_URL}/tasks" \
        > "${RESULTS_DIR}/rps-${RPS}.json" 2>&1

    echo "    Results saved to ${RESULTS_DIR}/rps-${RPS}.json"

    # Brief cool-down between tests
    sleep 5
done

echo "==> Baseline benchmark complete. Results in ${RESULTS_DIR}/"
echo ""
echo "Summary:"
for f in "${RESULTS_DIR}"/*.json; do
    rps=$(basename "$f" .json | sed 's/rps-//')
    if command -v jq &>/dev/null; then
        success=$(jq '.statusCodeDistribution."202" // 0' "$f")
        p50=$(jq '.responseTimePercentiles.p50 // "N/A"' "$f")
        p99=$(jq '.responseTimePercentiles.p99 // "N/A"' "$f")
        echo "  ${rps} RPS: ${success} accepted, p50=${p50}s, p99=${p99}s"
    else
        echo "  ${rps} RPS: see ${f}"
    fi
done
```

- [ ] **Step 2: Scaling validation benchmark**

`bench/scaling.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail

# Horizontal scaling benchmark
# Tests throughput with different processor replica counts
# Usage: ./bench/scaling.sh [GATEWAY_URL]

GATEWAY_URL="${1:-http://localhost:30000}"
RESULTS_DIR="bench/results/$(date +%Y%m%d-%H%M%S)-scaling"
mkdir -p "$RESULTS_DIR"

PAYLOAD='{"task_type":"compute","data":{"value":42}}'
RPS=1000
DURATION="30s"

for REPLICAS in 1 3 5; do
    echo "==> Scaling processor to ${REPLICAS} replicas..."
    kubectl -n distributed-system scale deployment/processor --replicas="$REPLICAS"
    kubectl -n distributed-system rollout status deployment/processor --timeout=60s
    sleep 10  # Let consumers settle

    echo "==> Testing at ${RPS} RPS with ${REPLICAS} processor replicas for ${DURATION}..."

    oha -z "$DURATION" \
        --rate "$RPS" \
        -m POST \
        -H "Content-Type: application/json" \
        -d "$PAYLOAD" \
        --json \
        "${GATEWAY_URL}/tasks" \
        > "${RESULTS_DIR}/replicas-${REPLICAS}.json" 2>&1

    echo "    Results saved."
    sleep 5
done

# Restore to 3 replicas
kubectl -n distributed-system scale deployment/processor --replicas=3

echo "==> Scaling benchmark complete. Results in ${RESULTS_DIR}/"
echo ""
echo "Summary:"
for f in "${RESULTS_DIR}"/*.json; do
    replicas=$(basename "$f" .json | sed 's/replicas-//')
    if command -v jq &>/dev/null; then
        success=$(jq '.statusCodeDistribution."202" // 0' "$f")
        p50=$(jq '.responseTimePercentiles.p50 // "N/A"' "$f")
        p99=$(jq '.responseTimePercentiles.p99 // "N/A"' "$f")
        echo "  ${replicas} replicas: ${success} accepted, p50=${p50}s, p99=${p99}s"
    else
        echo "  ${replicas} replicas: see ${f}"
    fi
done
```

- [ ] **Step 3: Stress/soak test**

`bench/stress.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail

# Stress/soak test
# Sustained load at moderate RPS for extended duration
# Usage: ./bench/stress.sh [GATEWAY_URL] [DURATION] [RPS]

GATEWAY_URL="${1:-http://localhost:30000}"
DURATION="${2:-10m}"
RPS="${3:-500}"
RESULTS_DIR="bench/results/$(date +%Y%m%d-%H%M%S)-stress"
mkdir -p "$RESULTS_DIR"

PAYLOAD='{"task_type":"compute","data":{"value":42}}'

echo "==> Stress test: ${RPS} RPS for ${DURATION}"
echo "    Gateway: ${GATEWAY_URL}"
echo "    Monitor Grafana at http://localhost:30003 during the test."
echo ""

oha -z "$DURATION" \
    --rate "$RPS" \
    -m POST \
    -H "Content-Type: application/json" \
    -d "$PAYLOAD" \
    --json \
    "${GATEWAY_URL}/tasks" \
    > "${RESULTS_DIR}/stress.json" 2>&1

echo "==> Stress test complete. Results in ${RESULTS_DIR}/stress.json"

if command -v jq &>/dev/null; then
    echo ""
    echo "Summary:"
    jq '{
        total_requests: .summary.total,
        successful: .statusCodeDistribution."202",
        errors: (.summary.total - (.statusCodeDistribution."202" // 0)),
        duration: .summary.elapsed,
        rps_actual: .summary.requestsPerSec,
        p50: .responseTimePercentiles.p50,
        p95: .responseTimePercentiles.p95,
        p99: .responseTimePercentiles.p99
    }' "${RESULTS_DIR}/stress.json"
fi
```

- [ ] **Step 4: Make scripts executable**

```bash
chmod +x bench/*.sh
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: add benchmark scripts (baseline, scaling, stress)"
```


---

### Task 21: NATS Benchmark and Failure Injection Scripts

**Files:**
- Create: `bench/nats-bench.sh`
- Create: `bench/failure-injection.sh`

- [ ] **Step 1: NATS throughput benchmark**

`bench/nats-bench.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail

# Raw NATS pub/sub throughput benchmark
# Requires: nats CLI (brew install nats-io/nats-tools/nats)
# Usage: ./bench/nats-bench.sh [NATS_URL]

NATS_URL="${1:-nats://localhost:4222}"
RESULTS_DIR="bench/results/$(date +%Y%m%d-%H%M%S)-nats"
mkdir -p "$RESULTS_DIR"

echo "==> NATS pub/sub throughput benchmark"
echo "    NATS URL: ${NATS_URL}"
echo ""

# Pub-only benchmark
echo "==> Publish throughput (10k messages, 256 bytes)..."
nats bench tasks.bench --pub 1 --msgs 10000 --size 256 --server "$NATS_URL" 2>&1 | tee "${RESULTS_DIR}/pub-only.txt"

echo ""

# Pub-sub benchmark (1 pub, 1 sub)
echo "==> Pub-sub throughput (1 pub, 1 sub, 10k messages)..."
nats bench tasks.bench --pub 1 --sub 1 --msgs 10000 --size 256 --server "$NATS_URL" 2>&1 | tee "${RESULTS_DIR}/pub-sub-1.txt"

echo ""

# Pub-sub benchmark (1 pub, 3 sub, queue group — simulates competing consumers)
echo "==> Pub-sub throughput (1 pub, 3 queue subs, 10k messages)..."
nats bench tasks.bench --pub 1 --sub 3 --msgs 10000 --size 256 --qgroup processors --server "$NATS_URL" 2>&1 | tee "${RESULTS_DIR}/pub-qsub-3.txt"

echo ""
echo "==> NATS benchmark complete. Results in ${RESULTS_DIR}/"
```

- [ ] **Step 2: Failure injection script**

`bench/failure-injection.sh`:
```bash
#!/usr/bin/env bash
set -euo pipefail

# Failure injection test
# Kills a processor pod during sustained load and measures recovery
# Usage: ./bench/failure-injection.sh [GATEWAY_URL]

GATEWAY_URL="${1:-http://localhost:30000}"
RESULTS_DIR="bench/results/$(date +%Y%m%d-%H%M%S)-failure"
mkdir -p "$RESULTS_DIR"

PAYLOAD='{"task_type":"compute","data":{"value":42}}'
RPS=500
DURATION="60s"

echo "==> Starting sustained load at ${RPS} RPS..."
echo "    A processor pod will be killed at the 15-second mark."
echo "    Monitor Grafana at http://localhost:30003 during the test."
echo ""

# Start load in background
oha -z "$DURATION" \
    --rate "$RPS" \
    -m POST \
    -H "Content-Type: application/json" \
    -d "$PAYLOAD" \
    --json \
    "${GATEWAY_URL}/tasks" \
    > "${RESULTS_DIR}/load-with-failure.json" 2>&1 &
LOAD_PID=$!

# Wait 15 seconds, then kill a processor pod
sleep 15
echo "==> Killing a processor pod..."
VICTIM=$(kubectl -n distributed-system get pods -l app=processor -o jsonpath='{.items[0].metadata.name}')
kubectl -n distributed-system delete pod "$VICTIM" --grace-period=0 --force
echo "    Killed: $VICTIM"
echo "    Kubernetes will reschedule. NATS should rebalance to surviving consumers."

# Wait for load to finish
wait $LOAD_PID

echo ""
echo "==> Failure injection test complete."
echo "    Results: ${RESULTS_DIR}/load-with-failure.json"
echo ""
echo "    Check Grafana for:"
echo "    - Error rate spike at the 15-second mark"
echo "    - Recovery time (when error rate returns to zero)"
echo "    - Whether any requests were lost (compare total accepted vs aggregator count)"

# Compare gateway accepted count vs aggregator persisted count
sleep 5
STATS=$(curl -s http://localhost:30001/stats)
echo ""
echo "Aggregator stats after test:"
echo "$STATS" | jq .
```

- [ ] **Step 3: Make scripts executable and commit**

```bash
chmod +x bench/nats-bench.sh bench/failure-injection.sh
git add -A
git commit -m "feat: add NATS benchmark and failure injection scripts"
```

---

## Phase 10: Documentation

### Task 22: Update AGENTS.md

**Files:**
- Modify: `AGENTS.md`

- [ ] **Step 1: Rewrite AGENTS.md to reflect the implemented project**

Update `AGENTS.md` with the actual project structure, commands, patterns, and conventions now that the codebase exists. Key sections:

- Project overview (4 services, NATS, k8s, observability)
- Architecture diagram (from spec)
- Key directories (crates, deploy, bench, scripts)
- Development commands: `docker compose up -d`, `cargo build`, `cargo run -p <service>`, benchmark scripts, k8s scripts
- Code conventions: workspace dependency inheritance, `deadpool-diesel` `interact()` pattern, NATS subject naming via `common::messages::subjects`, trace context propagation via `common::nats::{inject,extract}_trace_context`
- Testing: manual integration testing workflow, benchmark scripts
- Important files: each crate's main.rs, common/src/messages.rs (shared types), docker-compose.yaml, deploy/k8s/

- [ ] **Step 2: Commit**

```bash
git add AGENTS.md
git commit -m "docs: update AGENTS.md for implemented distributed system"
```

---

## Verification Checklist

After all tasks are complete, verify the full system:

- [ ] `cargo build` compiles all crates with zero warnings
- [ ] `docker compose up -d` starts NATS + Postgres
- [ ] All 4 services start locally and connect to NATS
- [ ] `POST /tasks` to gateway returns 202 with request_id
- [ ] Aggregator receives results and persists to Postgres
- [ ] `GET /results` and `GET /stats` return correct data
- [ ] Docker images build successfully
- [ ] k3d cluster starts, all pods become ready
- [ ] Gateway and Aggregator are reachable via NodePort
- [ ] Traces appear in Jaeger UI showing full request lifecycle
- [ ] Prometheus scrapes metrics from all pods
- [ ] Benchmark scripts run without errors
