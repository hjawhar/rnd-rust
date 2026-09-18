use crate::models::ws_client_manager::WsClientManager;
use crate::{models::state::AppState, routes::start_app_router};
use vm_data::db::Database;
use vm_nats::subjects;
use std::env;
use std::error::Error;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub mod background;
pub mod config;
pub mod models;
pub mod nats_subscriptions;
pub mod notifiers;
pub mod routes;
pub mod rpc_handlers;
pub mod streams;

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let app_env = std::env::var("APP_ENV").unwrap_or_else(|_| "development".into());
    dotenv::from_filename(format!(".env.{}", app_env)).ok();
    dotenv::dotenv().ok();

    // Clean up empty TOKIO_WORKER_THREADS (dotenv sets it to "" which tokio can't parse)
    if std::env::var("TOKIO_WORKER_THREADS").is_ok_and(|v| v.is_empty()) {
        unsafe { std::env::remove_var("TOKIO_WORKER_THREADS"); }
    }
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    if let Some(n) = std::env::var("TOKIO_WORKER_THREADS").ok().and_then(|v| v.parse::<usize>().ok()) {
        builder.worker_threads(n);
    }
    let runtime = builder.build().expect("Failed to build Tokio runtime");
    runtime.block_on(async_main())
}

async fn async_main() -> Result<(), Box<dyn Error + Send + Sync>> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let config = config::Config::from_env();

    let file_appender = tracing_appender::rolling::daily("./logs/", "logger.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let filter = tracing_subscriber::EnvFilter::new(
        format!("{},async_nats=warn,hyper=warn,hyper_util=warn,reqwest=warn,rustls=warn,tonic=warn,h2=warn,tower=warn,diesel=warn,tokio_postgres=warn", config.log_level)
    );

    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(filter)
        .with_thread_names(true)
        .with_thread_ids(true)
        .with_target(false)
        .init();
    tracing::info!(
        "Starting api-server (env={}, port={}, db_pool={}, read_replica={})",
        config.app_env,
        config.port,
        config.db_pool_size,
        env::var("DATABASE_READ_URL").is_ok(),
    );

    let ws_clients = Arc::new(WsClientManager::new());
    let db: Database = Database::new(config.db_pool_size).await;
    db.warm_pool(config.db_pool_size / 2 + 1).await;

    db.check_connection()
        .await
        .expect("Database connection health check failed");

    let read_db = if let Ok(read_url) = env::var("DATABASE_READ_URL") {
        let rdb = Database::new_with_url(&read_url, config.db_pool_size).await;
        rdb.warm_pool(config.db_pool_size / 2 + 1).await;
        rdb.check_connection()
            .await
            .expect("Read replica DB connection health check failed");
        tracing::info!("Read replica DB pool created");
        Some(rdb)
    } else {
        None
    };

    let db_clone = db.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let _ = db_clone.run_pending_migrations();
    })
    .await;

    // Seed admin user from ADMIN_ADDRESS env var (bootstrap on fresh install)
    if let Ok(address) = std::env::var("ADMIN_ADDRESS")
        && !address.is_empty()
            && db.get_user_by_address(&address).await.unwrap_or(None).is_none()
        {
            let new_user = vm_data::models::user::NewUser {
                address: address.clone(),
                nonce: uuid::Uuid::new_v4().to_string(),
                whitelisted: true,
                group_id: 1,
                date_added: std::time::SystemTime::now(),
            };
            let _ = db.add_user(&new_user).await;
            tracing::info!("Seeded admin user {}", address);
        }

    // Connect to NATS
    let nats_client = vm_nats::connect_nats()
        .await
        .expect("Failed to connect to NATS");

    // Start Discord notification service
    #[cfg(feature = "discord")]
    let _ = notifiers::discord::start_discord_service(nats_client.clone()).await;

    let shared_state = Arc::new(AppState {
        db,
        read_db,
        ws_clients,
        nats_client: nats_client.clone(),
    });

    // ── Shutdown signal ─────────────────────────────────────────────────────
    let shutdown = CancellationToken::new();
    {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = signal(SignalKind::terminate()).expect("failed to register SIGTERM");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = sigterm.recv() => {}
            }
            tracing::info!("Shutdown signal received, draining...");
            shutdown.cancel();
        });
    }

    // ── NATS subscriptions (DB writers + WS broadcast) ──────────────────────
    nats_subscriptions::setup(&nats_client, shared_state.clone(), shutdown.clone());

    // ── INIT_DATA RPC handlers ──────────────────────────────────────────────
    rpc_handlers::setup(&nats_client, shared_state.clone(), shutdown.clone());

    // ── Background tasks ────────────────────────────────────────────────────
    background::spawn_price_refresh(
        shared_state.clone(),
        shutdown.clone(),
        subjects::rpc::sol::PRICE_REFRESH,
        "api:sol_price_refresh",
        "SOL_PRICE",
    );
    background::spawn_price_refresh(
        shared_state.clone(),
        shutdown.clone(),
        subjects::rpc::evm::PRICE_REFRESH,
        "api:eth_price_refresh",
        "ETH_PRICE",
    );
    background::spawn_status_broadcast(shared_state.clone(), shutdown.clone());
    background::spawn_orphan_sweep(shared_state.clone(), shutdown.clone());

    // Start HTTP server — blocks until shutdown signal drains connections (10s timeout)
    start_app_router(shared_state.clone(), shutdown.clone(), config.port, config.max_concurrent_requests).await;

    // Give in-flight spawned tasks a moment to finish
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    tracing::info!("api-server shut down cleanly");
    Ok(())
}
