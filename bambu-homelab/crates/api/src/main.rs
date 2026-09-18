mod auth;
mod db;
mod models;
mod routes;
mod state;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use diesel::prelude::*;
use diesel_async::pooled_connection::bb8;
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::AsyncPgConnection;
use diesel_async::RunQueryDsl;
use futures_util::StreamExt;
use prost::Message;
use tokio::sync::{broadcast, RwLock};
use tower_http::cors::CorsLayer;
use tracing::info;

use bambu_shared::config::{self, ApiConfig};
use bambu_shared::telemetry::TelemetrySnapshot;

use crate::db::models::NewUser;
use crate::db::schema::users;
use crate::models::WsServerMessage;
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,bambu_api=debug".parse().unwrap()),
        )
        .with_target(true)
        .init();

    info!("bambu-api starting");

    let cfg: ApiConfig =
        config::load("BAMBU_API").context("failed to load API configuration")?;

    // Connect to NATS
    let ca_cert = std::path::Path::new(&cfg.nats.ca_cert);
    let nats_client = async_nats::ConnectOptions::new()
        .nkey(cfg.nats.nkey_seed.clone())
        .add_root_certificates(ca_cert.into())
        .require_tls(true)
        .connect(&cfg.nats.url)
        .await
        .context("failed to connect to NATS")?;

    info!("connected to NATS");

    // Initialize database connection pool
    let db_config = AsyncDieselConnectionManager::<AsyncPgConnection>::new(&cfg.database_url);
    let db_pool = bb8::Pool::builder()
        .max_size(10)
        .build(db_config)
        .await
        .context("failed to create database pool")?;

    info!("connected to database");

    // Run migrations
    run_migrations(&db_pool).await?;

    // Bootstrap admin user if none exists
    bootstrap_admin(&db_pool).await?;

    // Broadcast channel for WebSocket clients
    let (ws_tx, _) = broadcast::channel::<WsServerMessage>(256);

    let state = AppState {
        db: db_pool,
        jwt_secret: cfg.jwt_secret.clone(),
        telemetry_cache: Arc::new(RwLock::new(HashMap::new())),
        online_status: Arc::new(RwLock::new(HashMap::new())),
        ws_broadcast: ws_tx,
        nats: nats_client.clone(),
    };

    // Spawn NATS telemetry listener
    let telemetry_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = nats_telemetry_listener(nats_client, telemetry_state).await {
            tracing::error!(error = %e, "NATS telemetry listener failed");
        }
    });

    // Spawn NATS heartbeat listener
    let heartbeat_state = state.clone();
    let heartbeat_nats = state.nats.clone();
    tokio::spawn(async move {
        if let Err(e) = nats_heartbeat_listener(heartbeat_nats, heartbeat_state).await {
            tracing::error!(error = %e, "NATS heartbeat listener failed");
        }
    });

    // Build router
    let app = routes::build_router()
        .layer(CorsLayer::permissive())
        .with_state(state);

    info!(addr = %cfg.listen_addr, "bambu-api listening");
    let listener = tokio::net::TcpListener::bind(&cfg.listen_addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            info!("shutting down");
        })
        .await?;

    Ok(())
}

/// Run SQL migrations directly (avoids needing diesel_cli).
async fn run_migrations(pool: &db::Pool) -> anyhow::Result<()> {
    let mut conn = pool
        .get()
        .await
        .context("failed to get DB connection for migrations")?;

    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS __diesel_schema_migrations (
            version VARCHAR(50) PRIMARY KEY,
            run_on TIMESTAMP NOT NULL DEFAULT NOW()
        )",
    )
    .execute(&mut conn)
    .await
    .context("failed to create migrations table")?;

    // Migration 1: users table
    if !migration_applied(&mut conn, "2026-04-19-000001").await? {
        diesel::sql_query("CREATE EXTENSION IF NOT EXISTS \"uuid-ossp\"")
            .execute(&mut conn)
            .await?;
        diesel::sql_query(
            "CREATE TABLE users (
                id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
                username VARCHAR(255) NOT NULL UNIQUE,
                password_hash VARCHAR(255) NOT NULL,
                role VARCHAR(50) NOT NULL DEFAULT 'admin',
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&mut conn)
        .await?;
        diesel::sql_query(
            "INSERT INTO __diesel_schema_migrations (version) VALUES ('2026-04-19-000001')",
        )
        .execute(&mut conn)
        .await?;
        info!("migration applied: create_users");
    }

    // Migration 2: printers table
    if !migration_applied(&mut conn, "2026-04-19-000002").await? {
        diesel::sql_query(
            "CREATE TABLE printers (
                id VARCHAR(255) PRIMARY KEY,
                name VARCHAR(255) NOT NULL,
                ip VARCHAR(45) NOT NULL,
                serial VARCHAR(255) NOT NULL UNIQUE,
                access_code VARCHAR(255) NOT NULL,
                model VARCHAR(50) NOT NULL DEFAULT 'X1C',
                owner_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .execute(&mut conn)
        .await?;
        diesel::sql_query("CREATE INDEX idx_printers_owner ON printers(owner_id)")
            .execute(&mut conn)
            .await?;
        diesel::sql_query(
            "INSERT INTO __diesel_schema_migrations (version) VALUES ('2026-04-19-000002')",
        )
        .execute(&mut conn)
        .await?;
        info!("migration applied: create_printers");
    }

    // Migration 3: print_jobs table
    if !migration_applied(&mut conn, "2026-04-19-000003").await? {
        diesel::sql_query(
            "CREATE TABLE print_jobs (
                id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
                printer_id VARCHAR(255) NOT NULL REFERENCES printers(id) ON DELETE CASCADE,
                file_name VARCHAR(500) NOT NULL DEFAULT '',
                started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                finished_at TIMESTAMPTZ,
                status VARCHAR(50) NOT NULL DEFAULT 'printing',
                total_layers INTEGER NOT NULL DEFAULT 0,
                duration_seconds INTEGER,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )"
        )
        .execute(&mut conn)
        .await?;
        diesel::sql_query(
            "CREATE INDEX idx_print_jobs_printer ON print_jobs(printer_id)"
        )
        .execute(&mut conn)
        .await?;
        diesel::sql_query(
            "INSERT INTO __diesel_schema_migrations (version) VALUES ('2026-04-19-000003')"
        )
        .execute(&mut conn)
        .await?;
        info!("migration applied: create_print_jobs");
    }

    // Migration 4: print_queue
    if !migration_applied(&mut conn, "2026-04-19-000004").await? {
        diesel::sql_query(
            "CREATE TABLE print_queue (
                id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
                printer_id VARCHAR(255) NOT NULL REFERENCES printers(id) ON DELETE CASCADE,
                file_name VARCHAR(500) NOT NULL,
                plate_number INTEGER NOT NULL DEFAULT 1,
                status VARCHAR(50) NOT NULL DEFAULT 'queued',
                position INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )"
        )
        .execute(&mut conn)
        .await?;
        diesel::sql_query(
            "CREATE INDEX idx_print_queue_printer ON print_queue(printer_id, position)"
        )
        .execute(&mut conn)
        .await?;
        diesel::sql_query(
            "INSERT INTO __diesel_schema_migrations (version) VALUES ('2026-04-19-000004')"
        )
        .execute(&mut conn)
        .await?;
        info!("migration applied: create_print_queue");
    }

    // Migration 5: filament_usage
    if !migration_applied(&mut conn, "2026-04-19-000005").await? {
        diesel::sql_query(
            "CREATE TABLE filament_usage (
                id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
                print_job_id UUID REFERENCES print_jobs(id) ON DELETE CASCADE,
                printer_id VARCHAR(255) NOT NULL REFERENCES printers(id) ON DELETE CASCADE,
                filament_type VARCHAR(50) NOT NULL DEFAULT '',
                color VARCHAR(20) NOT NULL DEFAULT '',
                weight_grams REAL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )"
        )
        .execute(&mut conn)
        .await?;
        diesel::sql_query(
            "INSERT INTO __diesel_schema_migrations (version) VALUES ('2026-04-19-000005')"
        )
        .execute(&mut conn)
        .await?;
        info!("migration applied: create_filament_usage");
    }

    // Migration 6: printer_assignments
    if !migration_applied(&mut conn, "2026-04-20-000006").await? {
        diesel::sql_query(
            "CREATE TABLE printer_assignments (
                id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
                user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                printer_id VARCHAR(255) NOT NULL REFERENCES printers(id) ON DELETE CASCADE,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                UNIQUE (user_id, printer_id)
            )",
        )
        .execute(&mut conn)
        .await?;
        diesel::sql_query("CREATE INDEX idx_printer_assignments_user ON printer_assignments(user_id)")
            .execute(&mut conn)
            .await?;
        diesel::sql_query("CREATE INDEX idx_printer_assignments_printer ON printer_assignments(printer_id)")
            .execute(&mut conn)
            .await?;
        diesel::sql_query(
            "INSERT INTO __diesel_schema_migrations (version) VALUES ('2026-04-20-000006')",
        )
        .execute(&mut conn)
        .await?;
        info!("migration applied: create_printer_assignments");
    }

    Ok(())
}

#[derive(QueryableByName)]
struct MigrationCheck {
    #[diesel(sql_type = diesel::sql_types::Bool)]
    exists: bool,
}

async fn migration_applied(
    conn: &mut AsyncPgConnection,
    version: &str,
) -> anyhow::Result<bool> {
    let query = format!(
        "SELECT EXISTS(SELECT 1 FROM __diesel_schema_migrations WHERE version = '{version}') AS exists"
    );
    let result: MigrationCheck = diesel::sql_query(&query)
        .get_result(conn)
        .await
        .unwrap_or(MigrationCheck { exists: false });
    Ok(result.exists)
}

/// Create an admin user if none exists. Prints the password to stdout.
async fn bootstrap_admin(pool: &db::Pool) -> anyhow::Result<()> {
    let mut conn = pool
        .get()
        .await
        .context("failed to get DB connection for admin bootstrap")?;

    let admin_count: i64 = users::table
        .filter(users::role.eq("admin"))
        .count()
        .get_result(&mut conn)
        .await
        .unwrap_or(0);

    if admin_count > 0 {
        return Ok(());
    }

    // Generate a random password
    let password: String = uuid::Uuid::new_v4()
        .to_string()
        .chars()
        .take(12)
        .collect();

    use argon2::password_hash::rand_core::OsRng;
    use argon2::password_hash::SaltString;
    use argon2::PasswordHasher;

    let salt = SaltString::generate(&mut OsRng);
    let password_hash = argon2::Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("failed to hash password: {e}"))?
        .to_string();

    let new_user = NewUser {
        username: "admin",
        password_hash: &password_hash,
        role: "admin",
    };

    diesel::insert_into(users::table)
        .values(&new_user)
        .execute(&mut conn)
        .await
        .context("failed to create admin user")?;

    info!("========================================");
    info!("  First run - admin user created");
    info!("  Username: admin");
    info!("  Password: {password}");
    info!("  Change this password on first login!");
    info!("========================================");

    Ok(())
}

/// Subscribe to all printer telemetry on NATS and broadcast to WebSocket clients.
async fn nats_telemetry_listener(
    nats: async_nats::Client,
    state: AppState,
) -> anyhow::Result<()> {
    let mut sub = nats
        .subscribe("printers.*.telemetry")
        .await
        .context("failed to subscribe to printer telemetry")?;

    info!("listening for printer telemetry on NATS");

    let mut prev_states: HashMap<String, String> = HashMap::new();

    while let Some(msg) = sub.next().await {
        let parts: Vec<&str> = msg.subject.as_str().split('.').collect();
        let printer_id = match parts.get(1) {
            Some(id) => id.to_string(),
            None => continue,
        };

        let snapshot = match TelemetrySnapshot::decode(msg.payload.as_ref()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "failed to decode telemetry");
                continue;
            }
        };

        // Detect print state transitions and log to DB
        let gcode_state = snapshot.gcode_state.clone();
        let prev = prev_states.get(&printer_id).cloned().unwrap_or_default();
        prev_states.insert(printer_id.clone(), gcode_state.clone());

        if prev != gcode_state && !prev.is_empty() {
            let pool = state.db.clone();
            let pid = printer_id.clone();
            let file_name = snapshot.subtask_name.clone();
            let total_layers = snapshot.total_layer_num as i32;
            let gs = gcode_state.clone();
            let ps = prev.clone();

            tokio::spawn(async move {
                if let Err(e) = log_state_transition(&pool, &pid, &ps, &gs, &file_name, total_layers).await {
                    tracing::warn!(error = %e, "failed to log state transition");
                }
            });
        }

        let json_value = serde_json::to_value(&snapshot).unwrap_or_default();

        state
            .telemetry_cache
            .write()
            .await
            .insert(printer_id.clone(), json_value.clone());
        state
            .online_status
            .write()
            .await
            .insert(printer_id.clone(), std::time::Instant::now());

        let _ = state.ws_broadcast.send(WsServerMessage::Telemetry {
            printer_id,
            data: json_value,
        });
    }

    Ok(())
}

async fn log_state_transition(
    pool: &db::Pool,
    printer_id: &str,
    prev_state: &str,
    new_state: &str,
    file_name: &str,
    total_layers: i32,
) -> anyhow::Result<()> {
    use crate::db::models::{NewPrintJob, PrintJob};
    use crate::db::schema::print_jobs;

    let mut conn = pool.get().await?;

    match (prev_state, new_state) {
        // Print started
        (_, "RUNNING") if prev_state != "RUNNING" && prev_state != "PAUSE" => {
            let job = NewPrintJob {
                printer_id,
                file_name,
                status: "printing",
                total_layers,
            };
            diesel::insert_into(print_jobs::table)
                .values(&job)
                .execute(&mut conn)
                .await?;
            info!(printer_id, file_name, "print job started");
        }
        // Print completed
        ("RUNNING" | "PAUSE", "FINISH") => {
            let now = chrono::Utc::now().naive_utc();
            let active_job: Option<PrintJob> = print_jobs::table
                .filter(print_jobs::printer_id.eq(printer_id))
                .filter(print_jobs::status.eq("printing"))
                .order(print_jobs::started_at.desc())
                .first(&mut conn)
                .await
                .ok();

            if let Some(job) = active_job {
                let duration = (now - job.started_at).num_seconds() as i32;
                diesel::update(print_jobs::table.find(job.id))
                    .set((
                        print_jobs::status.eq("completed"),
                        print_jobs::finished_at.eq(Some(now)),
                        print_jobs::duration_seconds.eq(Some(duration)),
                    ))
                    .execute(&mut conn)
                    .await?;
            }
            info!(printer_id, "print job completed");
        }
        // Print failed
        ("RUNNING" | "PAUSE", "FAILED") => {
            let now = chrono::Utc::now().naive_utc();
            let active_job: Option<PrintJob> = print_jobs::table
                .filter(print_jobs::printer_id.eq(printer_id))
                .filter(print_jobs::status.eq("printing"))
                .order(print_jobs::started_at.desc())
                .first(&mut conn)
                .await
                .ok();

            if let Some(job) = active_job {
                let duration = (now - job.started_at).num_seconds() as i32;
                diesel::update(print_jobs::table.find(job.id))
                    .set((
                        print_jobs::status.eq("failed"),
                        print_jobs::finished_at.eq(Some(now)),
                        print_jobs::duration_seconds.eq(Some(duration)),
                    ))
                    .execute(&mut conn)
                    .await?;
            }
            info!(printer_id, "print job failed");
        }
        _ => {}
    }
    Ok(())
}

/// Subscribe to printer heartbeats on NATS and update online status.
async fn nats_heartbeat_listener(
    nats: async_nats::Client,
    state: AppState,
) -> anyhow::Result<()> {
    let mut sub = nats
        .subscribe("printers.*.heartbeat")
        .await
        .context("failed to subscribe to printer heartbeats")?;

    info!("listening for printer heartbeats on NATS");

    while let Some(msg) = sub.next().await {
        let parts: Vec<&str> = msg.subject.as_str().split('.').collect();
        let printer_id = match parts.get(1) {
            Some(id) => id.to_string(),
            None => continue,
        };
        state.online_status.write().await.insert(printer_id, std::time::Instant::now());
    }

    Ok(())
}