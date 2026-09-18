use common::nats::extract_trace_context;
use tracing_opentelemetry::OpenTelemetrySpanExt;
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
use futures::StreamExt;
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

            if let Some(ref headers) = msg.headers {
                let parent_context = extract_trace_context(headers);
                let _ = tracing::Span::current().set_parent(parent_context);
            }

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
