use crate::requests::{
    project::{
        add_project::add_project_req, collect_sol::collect_sol_req,
        collect_tokens::collect_tokens_req, delete_project::delete_project_req,
        disperse_sol::disperse_sol_req, disperse_tokens::disperse_tokens_req,
        start_task::start_task_req, stop_task::stop_task_req,
    },
    wallets::{
        delete_wallets::delete_wallets_req, generate_wallets::generate_wallets_req,
        import_wallets::import_wallets_req, view_wallets::view_wallets_req,
    },
};
use futures::StreamExt;
use vm_data::db::Database;
use vm_data::models::streams::{StreamInfo, StreamType};
use vm_solana::utils::helpers::fetch_sol_price;
use vm_nats::subjects;
use solana_commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use std::{env, error::Error, sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct Config {
    app_env: String,
    log_level: String,
    worker_id: String,
    queue_group: String,
    db_pool_size: usize,
    handler_semaphore: usize,
}

impl Config {
    fn from_env() -> Self {
        let app_env = env::var("APP_ENV").unwrap_or_else(|_| "development".into());
        let is_prod = app_env == "production";

        let required = [
            "DATABASE_URL",
            "REDIS_URL",
            "AES256_GCM_KEY",
            "ED25519_KEY",
            "GPA_ENDPOINT",
            "GEYSER_ENDPOINT",
            "GEYSER_TOKEN",
        ];
        let missing: Vec<&str> = required
            .iter()
            .filter(|k| env::var(k).is_err())
            .copied()
            .collect();
        if !missing.is_empty() {
            eprintln!(
                "FATAL: missing required environment variables: {}",
                missing.join(", ")
            );
            std::process::exit(1);
        }

        Self {
            log_level: env::var("LOG_LEVEL").unwrap_or_else(|_| {
                if is_prod { "info" } else { "debug" }.into()
            }),
            worker_id: env::var("WORKER_ID").unwrap_or_else(|_| {
                let short_uuid = Uuid::new_v4().to_string();
                format!("worker-{}", short_uuid.split('-').next().unwrap())
            }),
            queue_group: env::var("WORKER_QUEUE_GROUP")
                .unwrap_or_else(|_| "workers".into()),
            db_pool_size: env::var("DB_POOL_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(if is_prod { 5 } else { 3 }),
            handler_semaphore: env::var("MAX_CONCURRENT_HANDLERS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(if is_prod { 200 } else { 50 }),
            app_env,
        }
    }
}

pub mod cache;
pub mod geyser_service;
pub mod handlers;
pub mod requests;
pub mod state;

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let app_env = std::env::var("APP_ENV").unwrap_or_else(|_| "development".into());
    dotenv::from_filename(format!(".env.{}", app_env)).ok();
    dotenv::dotenv().ok();

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

    let config = Config::from_env();

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
        "[{}] Starting worker (env={}, queue_group={}, db_pool={}, semaphore={})",
        config.worker_id,
        config.app_env,
        config.queue_group,
        config.db_pool_size,
        config.handler_semaphore,
    );

    let sol_price = fetch_sol_price().await;
    if let Err(e) = &sol_price {
        tracing::error!("[{}] SOL Price init failed: {}", config.worker_id, e);
    }

    // Connect to NATS
    let nats_client = vm_nats::connect_nats()
        .await
        .expect("Failed to connect to NATS");
    let jetstream = vm_nats::setup_jetstream(&nats_client)
        .await
        .expect("Failed to setup JetStream");

    tracing::info!("[{}] Connected to NATS", config.worker_id);

    // Connect to database
    let db = Database::new(config.db_pool_size).await;
    db.warm_pool(config.db_pool_size / 2 + 1).await;
    tracing::info!("[{}] Connected to database", config.worker_id);

    // Create RPC client
    let gpa_endpoint = env::var("GPA_ENDPOINT").expect("GPA_ENDPOINT is required");
    let rpc: Arc<RpcClient> = Arc::new(RpcClient::new_with_commitment(
        gpa_endpoint,
        CommitmentConfig {
            commitment: CommitmentLevel::Processed,
        },
    ));

    // Create AppState with in-memory caches
    let (geyser_tx, geyser_rx) = tokio::sync::mpsc::channel::<Vec<String>>(16);
    let app_state = Arc::new(state::AppState::new(
        rpc,
        nats_client.clone(),
        jetstream.clone(),
        db.clone(),
        config.worker_id.clone(),
        geyser_tx,
    ));
    state::init_global_state(app_state);
    tracing::info!("[{}] AppState initialized with in-memory caches", config.worker_id);

    // Self-initialize: fetch all projects/wallets from api-server and populate caches
    handlers::init::self_initialize(&nats_client, &db, &config.worker_id).await;

    // Start embedded Geyser streaming service (replaces geyser-relay)
    geyser_service::start_geyser_service(geyser_rx);
    tracing::info!("[{}] Geyser service started", config.worker_id);

    // Semaphore to cap concurrent spawned handlers (JetStream + Geyser)
    let handler_semaphore = Arc::new(Semaphore::new(config.handler_semaphore));

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

    // ── Heartbeat loop — write to Redis every 15s ────────────────────────
    {
        let worker_id = config.worker_id.clone();
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            loop {
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    .to_string();
                let _ = vm_redis::hset_redis("workers:sol", &worker_id, &timestamp).await;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(15)) => {}
                    _ = shutdown.cancelled() => {
                        // Release all owned projects so the sweep can reassign immediately
                        if let Ok(projects) = vm_redis::task_ownership::get_owned_projects("sol", &worker_id).await {
                            let count = projects.len();
                            for pid in projects {
                                let _ = vm_redis::task_ownership::release_task("sol", pid, &worker_id).await;
                                let _ = vm_redis::task_heartbeat::clear_heartbeat("sol", pid).await;
                            }
                            tracing::info!("[SOL] Released ownership of {} project(s) on shutdown", count);
                        }
                        let _ = vm_redis::hdel_redis("workers:sol", &worker_id).await;
                        break;
                    }
                }
            }
        });
    }

    let queue_group = config.queue_group.clone();
    let mut handles = vec![];

    // ── RPC Request-Reply handlers (inline, custom protocol) ────────────────
    use handlers::rpc::*;
    handles.push(spawn_token_info(&nats_client, &queue_group, shutdown.clone()));
    handles.push(spawn_pair_info(&nats_client, &queue_group, shutdown.clone()));
    handles.push(spawn_pool_financials(&nats_client, &queue_group, shutdown.clone()));
    handles.push(spawn_wallets_financials(&nats_client, &queue_group, shutdown.clone()));
    handles.push(spawn_sol_price(&nats_client, &queue_group, shutdown.clone()));
    handles.push(spawn_sol_price_refresh(&nats_client, &queue_group, shutdown.clone()));
    handles.push(spawn_daily_volume(&nats_client, &queue_group, shutdown.clone()));
    handles.push(spawn_verify_signature(&nats_client, &queue_group, shutdown.clone()));

    // ── RPC Request-Reply handlers (generic StreamInfo protocol) ────────────
    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::sol::PROJECT_NEW,
        |st| {
            if let StreamType::RequestNewProject(p) = st {
                Some(p)
            } else {
                None
            }
        },
        StreamType::ResponseNewProject,
        {
            let db = db.clone();
            move |nc, user_id, p: vm_data::models::project::NewProjectPayload| {
                let db = db.clone();
                async move {
                    let pool = p.pool.unwrap_or_default();
                    (add_project_req(&db, &nc, user_id, p.address, pool, p.trading_strategy).await)
                        .ok()
                }
            }
        },
        shutdown.clone(),
    ));

    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::sol::WALLETS_IMPORT,
        |st| {
            if let StreamType::RequestImportWallets(p) = st {
                Some(p)
            } else {
                None
            }
        },
        StreamType::ResponseImportWallets,
        {
            let db = db.clone();
            move |nc, user_id, p| {
                let db = db.clone();
                async move { import_wallets_req(&db, &nc, user_id, p).await }
            }
        },
        shutdown.clone(),
    ));

    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::sol::WALLETS_GENERATE,
        |st| {
            if let StreamType::RequestGenerateWallets(p) = st {
                Some(p)
            } else {
                None
            }
        },
        StreamType::ResponseGenerateWallets,
        {
            let db = db.clone();
            move |nc, user_id, p| {
                let db = db.clone();
                async move { generate_wallets_req(&db, &nc, user_id, p).await }
            }
        },
        shutdown.clone(),
    ));

    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::sol::WALLETS_VIEW,
        |st| {
            if let StreamType::RequestViewWallets(p) = st {
                Some(p)
            } else {
                None
            }
        },
        StreamType::ResponseViewWallets,
        {
            let db = db.clone();
            move |_nc, user_id, p| {
                let db = db.clone();
                async move { view_wallets_req(&db, user_id, p).await }
            }
        },
        shutdown.clone(),
    ));

    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::sol::WALLETS_DELETE,
        |st| {
            if let StreamType::RequestDeleteWallets(p) = st {
                Some(p)
            } else {
                None
            }
        },
        StreamType::ResponseDeleteWallets,
        {
            let db = db.clone();
            move |nc, user_id, p| {
                let db = db.clone();
                async move { delete_wallets_req(&db, &nc, user_id, p).await }
            }
        },
        shutdown.clone(),
    ));

    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::sol::PROJECT_START_TASK,
        |st| {
            if let StreamType::RequestStartTask(id) = st {
                Some(id)
            } else {
                None
            }
        },
        StreamType::ResponseStartTask,
        {
            let db = db.clone();
            let js = jetstream.clone();
            move |nc, user_id, project_id| {
                let db = db.clone();
                let js = js.clone();
                async move { start_task_req(&db, &nc, &js, user_id, project_id).await }
            }
        },
        shutdown.clone(),
    ));

    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::sol::PROJECT_STOP_TASK,
        |st| {
            if let StreamType::RequestStopTask(id) = st {
                Some(id)
            } else {
                None
            }
        },
        StreamType::ResponseStopTask,
        {
            let db = db.clone();
            let js = jetstream.clone();
            move |nc, user_id, project_id| {
                let db = db.clone();
                let js = js.clone();
                async move { stop_task_req(&db, &nc, &js, user_id, project_id).await }
            }
        },
        shutdown.clone(),
    ));

    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::sol::PROJECT_DELETE,
        |st| {
            if let StreamType::RequestDeleteProject(id) = st {
                Some(id)
            } else {
                None
            }
        },
        StreamType::ResponseDeleteProject,
        {
            let db = db.clone();
            move |nc, user_id, project_id| {
                let db = db.clone();
                async move { delete_project_req(&db, &nc, user_id, project_id).await }
            }
        },
        shutdown.clone(),
    ));

    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::sol::PROJECT_COLLECT_SOL,
        |st| {
            if let StreamType::RequestCollectNative(id) = st {
                Some(id)
            } else {
                None
            }
        },
        StreamType::ResponseCollectNative,
        {
            let db = db.clone();
            let js = jetstream.clone();
            move |_nc, user_id, project_id| {
                let db = db.clone();
                let js = js.clone();
                async move { collect_sol_req(&db, &js, user_id, project_id).await }
            }
        },
        shutdown.clone(),
    ));

    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::sol::PROJECT_COLLECT_TOKENS,
        |st| {
            if let StreamType::RequestCollectTokens(id) = st {
                Some(id)
            } else {
                None
            }
        },
        StreamType::ResponseCollectTokens,
        {
            let db = db.clone();
            let js = jetstream.clone();
            move |_nc, user_id, project_id| {
                let db = db.clone();
                let js = js.clone();
                async move { collect_tokens_req(&db, &js, user_id, project_id).await }
            }
        },
        shutdown.clone(),
    ));

    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::sol::PROJECT_DISPERSE_SOL,
        |st| {
            if let StreamType::RequestDisperseNative(id) = st {
                Some(id)
            } else {
                None
            }
        },
        StreamType::ResponseDisperseNative,
        {
            let db = db.clone();
            let js = jetstream.clone();
            move |_nc, user_id, project_id| {
                let db = db.clone();
                let js = js.clone();
                async move { disperse_sol_req(&db, &js, user_id, project_id).await }
            }
        },
        shutdown.clone(),
    ));

    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::sol::PROJECT_DISPERSE_TOKENS,
        |st| {
            if let StreamType::RequestDisperseTokens(id) = st {
                Some(id)
            } else {
                None
            }
        },
        StreamType::ResponseDisperseTokens,
        {
            let db = db.clone();
            let js = jetstream.clone();
            move |_nc, user_id, project_id| {
                let db = db.clone();
                let js = js.clone();
                async move { disperse_tokens_req(&db, &js, user_id, project_id).await }
            }
        },
        shutdown.clone(),
    ));

    // ── JetStream command consumer ──────────────────────────────────────────
    {
        let nc = nats_client.clone();
        let js = jetstream.clone();
        let db = db.clone();
        let sem = handler_semaphore.clone();
        let shutdown = shutdown.clone();
        handles.push(tokio::task::spawn(async move {
            let consumer_config = async_nats::jetstream::consumer::pull::Config {
                durable_name: Some("worker-sol-commands".to_string()),
                ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                max_deliver: 3,
                ack_wait: Duration::from_secs(30),
                ..Default::default()
            };

            loop {
                if shutdown.is_cancelled() {
                    break;
                }

                let stream = match js.get_stream("VM_SOL_COMMANDS").await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("[NATS] Failed to get VM_SOL_COMMANDS stream: {e}");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                };

                let consumer = match stream
                    .get_or_create_consumer("worker-sol-commands", consumer_config.clone())
                    .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("[NATS] Failed to create JetStream consumer: {e}");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                };

                let mut messages = match consumer.messages().await {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::error!("[NATS] Failed to start JetStream message stream: {e}");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                };

                loop {
                    tokio::select! {
                        msg = messages.next() => {
                            let Some(Ok(msg)) = msg else {
                                tracing::warn!("[NATS] JetStream SOL message stream ended, re-creating consumer in 2s...");
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                break; // break inner loop → outer loop re-creates consumer
                            };
                            let nc = nc.clone();
                            let db = db.clone();
                            let sem = sem.clone();
                            let subject = msg.subject.clone().to_string();
                            let payload = msg.payload.clone();

                            tokio::spawn(async move {
                                let _permit = sem.acquire().await;
                                let si: StreamInfo = match serde_json::from_slice(&payload) {
                                    Ok(v) => v,
                                    Err(_) => return,
                                };

                                handlers::commands::process_command(nc, db, si, &subject).await;
                            });

                            let _ = msg.ack().await;
                        }
                        _ = shutdown.cancelled() => break,
                    }
                }
            }
        }));
    }

    // Wait for shutdown or all handles to finish
    shutdown.cancelled().await;

    // Give in-flight work time to complete (4s timeout)
    tracing::info!("Waiting up to 4s for in-flight work to complete...");
    let _ = tokio::time::timeout(
        Duration::from_secs(4),
        futures::future::join_all(handles),
    )
    .await;

    tracing::info!("worker-sol shut down cleanly");
    Ok(())
}
