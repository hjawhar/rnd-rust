use futures::StreamExt;
use vm_data::db::Database;
use vm_data::models::streams::{StreamInfo, StreamType, TokenInfoPayload, PoolFinancialsPayload};
use vm_nats::subjects;
use std::env;
use std::error::Error;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Global worker ID, set once at startup. Used by task ownership claims.
static WORKER_ID: OnceLock<String> = OnceLock::new();

/// Get the worker ID for this process.
pub fn get_worker_id() -> &'static str {
    WORKER_ID.get().expect("WORKER_ID not initialized")
}

pub mod cache;
pub mod handlers;
pub mod requests;
pub mod rpc;

use handlers::rpc::rpc_handler;
use requests::project::{
    add_project, collect_eth, collect_tokens, delete_project, disperse_eth, disperse_tokens,
    start_task, stop_task,
};
use requests::tokens::token_info;
use requests::wallets::{delete_wallets, generate_wallets, import_wallets, view_wallets};

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
            "ETH_HTTP",
            "BASE_HTTP",
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
                format!("worker-evm-{}", short_uuid.split('-').next().unwrap())
            }),
            queue_group: env::var("WORKER_QUEUE_GROUP")
                .unwrap_or_else(|_| "workers-evm".into()),
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
    WORKER_ID
        .set(config.worker_id.clone())
        .expect("WORKER_ID already initialized");

    let file_appender = tracing_appender::rolling::daily("./logs/", "logger.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let filter = tracing_subscriber::EnvFilter::new(
        format!("{},async_nats=warn,hyper=warn,hyper_util=warn,reqwest=warn,rustls=warn,alloy=warn,h2=warn,tower=warn,tonic=warn,diesel=warn,tokio_postgres=warn", config.log_level),
    );

    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(filter)
        .with_thread_names(true)
        .with_thread_ids(true)
        .with_target(false)
        .init();

    tracing::info!(
        "[EVM] Starting worker-evm (id={}, env={}, queue_group={}, db_pool={}, semaphore={})",
        config.worker_id,
        config.app_env,
        config.queue_group,
        config.db_pool_size,
        config.handler_semaphore,
    );

    // Database
    let db = Database::new(config.db_pool_size).await;
    db.warm_pool(config.db_pool_size / 2 + 1).await;
    db.check_connection()
        .await
        .expect("Database connection health check failed");
    tracing::info!("[EVM] Database connected");

    // NATS
    let nats_client = vm_nats::connect_nats()
        .await
        .expect("Failed to connect to NATS");
    let jetstream = vm_nats::setup_evm_jetstream(&nats_client)
        .await
        .expect("Failed to setup EVM JetStream");
    tracing::info!("[EVM] NATS connected, JetStream ready");

    let queue_group = config.queue_group.clone();

    // Semaphore to cap concurrent spawned handlers (JetStream commands)
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
                let _ = vm_redis::hset_redis("workers:evm", &worker_id, &timestamp).await;
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(15)) => {}
                    _ = shutdown.cancelled() => {
                        // Release all owned projects so the sweep can reassign immediately
                        if let Ok(projects) = vm_redis::task_ownership::get_owned_projects("evm", &worker_id).await {
                            let count = projects.len();
                            for pid in projects {
                                let _ = vm_redis::task_ownership::release_task("evm", pid, &worker_id).await;
                                let _ = vm_redis::task_heartbeat::clear_heartbeat("evm", pid).await;
                            }
                            tracing::info!("[EVM] Released ownership of {} project(s) on shutdown", count);
                        }
                        let _ = vm_redis::hdel_redis("workers:evm", &worker_id).await;
                        break;
                    }
                }
            }
        });
    }

    let mut handles = Vec::new();

    // ── Fetch ETH price at startup ────────────────────────────────────────
    let eth_price = vm_evm::simulation::fetch_eth_price::fetch_eth_price(
        &vm_evm::constants::Network::Ethereum,
    )
    .await;
    match &eth_price {
        Ok(p) => {
            let _ = cache::set_eth_price(*p).await;
            tracing::info!("[EVM] ETH price init: ${:.2}", p);
        }
        Err(e) => {
            tracing::error!("[EVM] ETH price init failed: {}", e);
        }
    }

    // ── Self-initialize from api-server ───────────────────────────────────
    handlers::init::self_initialize(&nats_client, &db).await;

    // ── RPC Request-Reply handlers (generic StreamInfo protocol) ──────────

    // PROJECT_NEW
    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::evm::PROJECT_NEW,
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
            move |nc, user_id, p| {
                let db = db.clone();
                async move {
                    add_project::add_project_req(&db, &nc, user_id, p)
                        .await
                        .ok()
                }
            }
        },
        shutdown.clone(),
    ));

    // WALLETS_IMPORT
    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::evm::WALLETS_IMPORT,
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
                async move { import_wallets::import_wallets_req(&db, &nc, user_id, p).await }
            }
        },
        shutdown.clone(),
    ));

    // WALLETS_GENERATE
    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::evm::WALLETS_GENERATE,
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
                async move { generate_wallets::generate_wallets_req(&db, &nc, user_id, p).await }
            }
        },
        shutdown.clone(),
    ));

    // WALLETS_VIEW
    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::evm::WALLETS_VIEW,
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
                async move { view_wallets::view_wallets_req(&db, user_id, p).await }
            }
        },
        shutdown.clone(),
    ));

    // WALLETS_DELETE
    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::evm::WALLETS_DELETE,
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
                async move { delete_wallets::delete_wallets_req(&db, &nc, user_id, p).await }
            }
        },
        shutdown.clone(),
    ));

    // PROJECT_START_TASK
    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::evm::PROJECT_START_TASK,
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
            move |nc, user_id, project_id| {
                let db = db.clone();
                async move { start_task::start_task_req(&db, &nc, user_id, project_id).await }
            }
        },
        shutdown.clone(),
    ));

    // PROJECT_STOP_TASK
    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::evm::PROJECT_STOP_TASK,
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
            move |nc, user_id, project_id| {
                let db = db.clone();
                async move { stop_task::stop_task_req(&db, &nc, user_id, project_id).await }
            }
        },
        shutdown.clone(),
    ));

    // PROJECT_DELETE
    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::evm::PROJECT_DELETE,
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
                async move { delete_project::delete_project_req(&db, &nc, user_id, project_id).await }
            }
        },
        shutdown.clone(),
    ));

    // PROJECT_COLLECT_ETH
    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::evm::PROJECT_COLLECT_ETH,
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
            move |nc, user_id, project_id| {
                let db = db.clone();
                async move { collect_eth::collect_eth_req(&db, &nc, user_id, project_id).await }
            }
        },
        shutdown.clone(),
    ));

    // PROJECT_COLLECT_TOKENS
    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::evm::PROJECT_COLLECT_TOKENS,
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
            move |nc, user_id, project_id| {
                let db = db.clone();
                async move {
                    collect_tokens::collect_tokens_req(&db, &nc, user_id, project_id).await
                }
            }
        },
        shutdown.clone(),
    ));

    // PROJECT_DISPERSE_ETH
    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::evm::PROJECT_DISPERSE_ETH,
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
            move |nc, user_id, project_id| {
                let db = db.clone();
                async move { disperse_eth::disperse_eth_req(&db, &nc, user_id, project_id).await }
            }
        },
        shutdown.clone(),
    ));

    // PROJECT_DISPERSE_TOKENS
    handles.push(rpc_handler(
        &nats_client,
        &queue_group,
        subjects::rpc::evm::PROJECT_DISPERSE_TOKENS,
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
            move |nc, user_id, project_id| {
                let db = db.clone();
                async move {
                    disperse_tokens::disperse_tokens_req(&db, &nc, user_id, project_id).await
                }
            }
        },
        shutdown.clone(),
    ));

    // TOKEN_INFO
    {
        let nc = nats_client.clone();
        let db = db.clone();
        let shutdown = shutdown.clone();
        let qg = queue_group.to_string();
        let mut sub = nc
            .queue_subscribe(subjects::rpc::evm::TOKEN_INFO, qg.clone())
            .await?;
        handles.push(tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = sub.next() => {
                        let Some(msg) = msg else {
                            tracing::warn!("[NATS] Subscription ended for {}, resubscribing in 1s...", subjects::rpc::evm::TOKEN_INFO);
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            match nc.queue_subscribe(subjects::rpc::evm::TOKEN_INFO, qg.clone()).await {
                                Ok(new_sub) => { sub = new_sub; continue; }
                                Err(e) => {
                                    tracing::error!("[NATS] Resubscribe failed for {}: {e}", subjects::rpc::evm::TOKEN_INFO);
                                    break;
                                }
                            }
                        };
                        let Some(reply) = msg.reply.clone() else { continue };
                        let nc_inner = nc.clone();
                        let db = db.clone();
                        tokio::spawn(async move {
                            let Ok(StreamType::RequestTokenInfo(TokenInfoPayload { token, network })) =
                                serde_json::from_slice::<StreamType>(&msg.payload)
                            else {
                                let bytes = serde_json::to_vec(&StreamType::ResponseTokenInfo(None))
                                    .unwrap_or_default();
                                let _ = nc_inner.publish(reply, bytes.into()).await;
                                return;
                            };
                            let network_name = network.unwrap_or_else(|| "base".to_string());
                            let network =
                                vm_evm::constants::Network::from_network_name(&network_name)
                                    .unwrap_or(vm_evm::constants::Network::Base);
                            let result = token_info::get_token_info(&db, &network, &token).await;
                            let response = match result {
                                Ok((_token, pools)) => StreamType::ResponseTokenInfo(Some(pools)),
                                Err(_) => StreamType::ResponseTokenInfo(None),
                            };
                            let bytes = serde_json::to_vec(&response).unwrap_or_default();
                            let _ = nc_inner.publish(reply, bytes.into()).await;
                        });
                    }
                    _ = shutdown.cancelled() => break,
                }
            }
        }));
    }

    // POOL_FINANCIALS
    {
        let nc = nats_client.clone();
        let db = db.clone();
        let shutdown = shutdown.clone();
        let qg = queue_group.to_string();
        let mut sub = nc
            .queue_subscribe(subjects::rpc::evm::POOL_FINANCIALS, qg.clone())
            .await?;
        handles.push(tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = sub.next() => {
                        let Some(msg) = msg else {
                            tracing::warn!("[NATS] Subscription ended for {}, resubscribing in 1s...", subjects::rpc::evm::POOL_FINANCIALS);
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            match nc.queue_subscribe(subjects::rpc::evm::POOL_FINANCIALS, qg.clone()).await {
                                Ok(new_sub) => { sub = new_sub; continue; }
                                Err(e) => {
                                    tracing::error!("[NATS] Resubscribe failed for {}: {e}", subjects::rpc::evm::POOL_FINANCIALS);
                                    break;
                                }
                            }
                        };
                        let Some(reply) = msg.reply.clone() else { continue };
                        let nc_inner = nc.clone();
                        let db = db.clone();
                        tokio::spawn(async move {
                            let Ok(StreamType::RequestPoolFinancials(PoolFinancialsPayload { pool, token, network })) =
                                serde_json::from_slice::<StreamType>(&msg.payload)
                            else {
                                let bytes =
                                    serde_json::to_vec(&StreamType::ResponsePoolFinancials(None))
                                        .unwrap_or_default();
                                let _ = nc_inner.publish(reply, bytes.into()).await;
                                return;
                            };
                            let network_name = network.unwrap_or_else(|| "base".to_string());
                            let token_addr = token.unwrap_or_default();
                            let network =
                                vm_evm::constants::Network::from_network_name(&network_name)
                                    .unwrap_or(vm_evm::constants::Network::Base);
                            let pools = token_info::get_token_info(&db, &network, &token_addr).await;
                            let response = match pools {
                                Ok((token, pools)) => {
                                    let mut found = pools.into_iter().find(|p| p.pool_address == pool);
                                    if let Some(ref mut found) = found {
                                        found.balance1 /= (10_i64.pow(token.decimals.into())) as f64;
                                    }

                                    StreamType::ResponsePoolFinancials(found)
                                }
                                Err(_) => StreamType::ResponsePoolFinancials(None),
                            };
                            let bytes = serde_json::to_vec(&response).unwrap_or_default();
                            let _ = nc_inner.publish(reply, bytes.into()).await;
                        });
                    }
                    _ = shutdown.cancelled() => break,
                }
            }
        }));
    }

    // DAILY_VOLUME
    {
        let db = db.clone();
        handles.push(rpc_handler(
            &nats_client,
            &queue_group,
            subjects::rpc::evm::DAILY_VOLUME,
            |st| {
                if let StreamType::RequestDailyVolume(id) = st {
                    Some(id)
                } else {
                    None
                }
            },
            StreamType::ResponseDailyVolume,
            move |_nc, user_id, project_id: i32| {
                let db = db.clone();
                async move {
                    let project = db.get_project(user_id, project_id).await.ok().flatten()?;
                    let target_usdc = vm_data::utils::helpers::big_int_to_f64(
                        project.trading_daily_volume.clone(),
                    );
                    let volume_usdc = cache::get_daily_volume(project_id)
                        .await
                        .unwrap_or(Some(0.0))
                        .unwrap_or(0.0);
                    Some(vm_data::models::streams::DailyVolumeInfo {
                        project_id,
                        volume_usdc,
                        target_usdc,
                    })
                }
            },
            shutdown.clone(),
        ));
    }

    // WALLETS_FINANCIALS
    {
        let nc = nats_client.clone();
        let db = db.clone();
        let shutdown = shutdown.clone();
        let qg = queue_group.to_string();
        let mut sub = nc
            .queue_subscribe(
                subjects::rpc::evm::WALLETS_FINANCIALS,
                qg.clone(),
            )
            .await?;
        handles.push(tokio::spawn(async move {
            use vm_data::models::solana::{
                PortfolioSummary, ProjectWalletInfo, WalletsFinancialsResponse,
            };

            let empty_response = |pid: i32| {
                let resp = WalletsFinancialsResponse {
                    project_id: pid,
                    wallets: vec![],
                    summary: PortfolioSummary::from_wallets(&[], 0.0, 0.0, 0.0, 0.0),
                };
                StreamType::ResponseWalletsFinancials(resp)
            };

            loop {
                tokio::select! {
                    msg = sub.next() => {
                        let Some(msg) = msg else {
                            tracing::warn!("[NATS] Subscription ended for {}, resubscribing in 1s...", subjects::rpc::evm::WALLETS_FINANCIALS);
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            match nc.queue_subscribe(subjects::rpc::evm::WALLETS_FINANCIALS, qg.clone()).await {
                                Ok(new_sub) => { sub = new_sub; continue; }
                                Err(e) => {
                                    tracing::error!("[NATS] Resubscribe failed for {}: {e}", subjects::rpc::evm::WALLETS_FINANCIALS);
                                    break;
                                }
                            }
                        };
                        let Some(reply) = msg.reply.clone() else { continue };
                        let nc_inner = nc.clone();
                        let db_inner = db.clone();
                        tokio::spawn(async move {
                            let Ok(stream_info) = serde_json::from_slice::<StreamInfo>(&msg.payload) else {
                                let bytes = serde_json::to_vec(&empty_response(0)).unwrap_or_default();
                                let _ = nc_inner.publish(reply, bytes.into()).await;
                                return;
                            };

                            let StreamType::RequestWalletsFinancials((project, wallets, _refresh_cache)) =
                                stream_info.stream_type
                            else {
                                let bytes = serde_json::to_vec(&empty_response(0)).unwrap_or_default();
                                let _ = nc_inner.publish(reply, bytes.into()).await;
                                return;
                            };

                            let network =
                                vm_evm::constants::Network::from_network_name(&project.network)
                                    .unwrap_or(vm_evm::constants::Network::Base);

                            let addresses: Vec<alloy::primitives::Address> = wallets
                                .iter()
                                .filter_map(|w| w.address.parse().ok())
                                .collect();

                            let token_addr: Option<alloy::primitives::Address> =
                                project.address.parse().ok();

                            let eth_balances = vm_evm::simulation::fetch_balances::get_eth_balances(
                                network.clone(),
                                addresses.clone(),
                            )
                            .await
                            .unwrap_or_else(|_| vec![alloy::primitives::U256::ZERO; addresses.len()]);

                            let token_balances = if let Some(token) = token_addr {
                                vm_evm::simulation::fetch_balances::get_tokens_balances(
                                    network.clone(),
                                    token,
                                    addresses.clone(),
                                )
                                .await
                                .unwrap_or_else(|_| vec![alloy::primitives::U256::ZERO; addresses.len()])
                            } else {
                                vec![alloy::primitives::U256::ZERO; addresses.len()]
                            };

                            let eth_price = cache::get_eth_price().await.unwrap_or(None).unwrap_or(0.0);

                            // Derive token price (ETH-denominated) from pool data
                            let decimals = project.decimals.unwrap_or(18) as u8;
                            let decimals_factor = 10f64.powi(decimals as i32);
                            let mut token_price_eth: f64 = 0.0;

                            if let Some(token) = token_addr
                                && let Ok(token_info) =
                                    vm_evm::simulation::fetch_pools::get_pools(&db_inner, network.clone(), token).await
                                {
                                    // Find the project's configured pool
                                    let best_pool = token_info
                                        .pools
                                        .iter()
                                        .find(|p| p.pool == project.pool)
                                        .or_else(|| {
                                            token_info
                                                .pools
                                                .iter()
                                                .max_by_key(|p| (p.version, p.balance))
                                        });

                                    if let Some(pool) = best_pool {
                                        if pool.version == 4 {
                                            // V4: use on-chain quoter for spot price
                                            let v4_pools = db_inner
                                                .get_latest_uniswap_v4_pools_by_address(
                                                    network.clone() as i32,
                                                    token.to_string(),
                                                )
                                                .await
                                                .unwrap_or_default();
                                            if let Some(v4_pool) = v4_pools.iter().find(|p| p.pool_key == pool.pool) {
                                                let pkp = vm_evm::simulation::market_impact::PoolKeyParams {
                                                    currency0: v4_pool.currency0.parse().unwrap_or_default(),
                                                    currency1: v4_pool.currency1.parse().unwrap_or_default(),
                                                    fee: v4_pool.fee.parse::<u32>().unwrap_or(0),
                                                    tick_spacing: v4_pool.tick_spacing.parse::<i32>().unwrap_or(60),
                                                    hooks: v4_pool.hooks.parse().unwrap_or_default(),
                                                };
                                                if let Ok(price) = vm_evm::simulation::market_impact::quote_v4_spot_price(
                                                    network.clone(),
                                                    pkp,
                                                    decimals,
                                                ).await {
                                                    token_price_eth = price;
                                                }
                                            }
                                        } else if pool.tokens > alloy::primitives::U256::ZERO {
                                            // V2/V3: derive price from pool reserves
                                            token_price_eth = (vm_evm::helpers::u256_to_f64(pool.balance) / 1e18)
                                                / (vm_evm::helpers::u256_to_f64(pool.tokens) / decimals_factor);
                                        }
                                    }
                                }

                            let token_price_usdc = token_price_eth * eth_price;

                            let mut wallet_infos = Vec::with_capacity(wallets.len());
                            for (i, w) in wallets.iter().enumerate() {
                                let native_bal = vm_evm::helpers::u256_to_f64(
                                    *eth_balances
                                        .get(i)
                                        .unwrap_or(&alloy::primitives::U256::ZERO),
                                ) / 1e18;
                                let token_bal = vm_evm::helpers::u256_to_f64(
                                    *token_balances
                                        .get(i)
                                        .unwrap_or(&alloy::primitives::U256::ZERO),
                                ) / decimals_factor;
                                let native_usdc = native_bal * eth_price;
                                let token_usdc = token_bal * token_price_usdc;
                                wallet_infos.push(ProjectWalletInfo {
                                    id: w.id,
                                    main: w.is_main,
                                    address: w.address.clone(),
                                    native_balance: native_bal,
                                    token_balance: token_bal,
                                    native_usdc,
                                    token_usdc,
                                    total_usdc: native_usdc + token_usdc,
                                });
                            }

                            let daily_vol = cache::get_daily_volume(project.id)
                                .await
                                .unwrap_or(None)
                                .unwrap_or(0.0);
                            let daily_target = vm_data::utils::helpers::big_int_to_f64(
                                project.trading_daily_volume.clone(),
                            );

                            let summary = PortfolioSummary::from_wallets(
                                &wallet_infos,
                                eth_price,
                                token_price_usdc,
                                daily_vol,
                                daily_target,
                            );

                            let response =
                                StreamType::ResponseWalletsFinancials(WalletsFinancialsResponse {
                                    project_id: project.id,
                                    wallets: wallet_infos,
                                    summary,
                                });
                            let bytes = serde_json::to_vec(&response).unwrap_or_default();
                            let _ = nc_inner.publish(reply, bytes.into()).await;
                        });
                    }
                    _ = shutdown.cancelled() => break,
                }
            }
        }));
    }

    // ETH PRICE
    {
        let nc = nats_client.clone();
        let shutdown = shutdown.clone();
        let qg = queue_group.to_string();
        let mut sub = nc
            .queue_subscribe(subjects::rpc::evm::PRICE, qg.clone())
            .await?;
        handles.push(tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = sub.next() => {
                        let Some(msg) = msg else {
                            tracing::warn!("[NATS] Subscription ended for {}, resubscribing in 1s...", subjects::rpc::evm::PRICE);
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            match nc.queue_subscribe(subjects::rpc::evm::PRICE, qg.clone()).await {
                                Ok(new_sub) => { sub = new_sub; continue; }
                                Err(e) => {
                                    tracing::error!("[NATS] Resubscribe failed for {}: {e}", subjects::rpc::evm::PRICE);
                                    break;
                                }
                            }
                        };
                        let Some(reply) = msg.reply.clone() else { continue };
                        let nc_inner = nc.clone();
                        tokio::spawn(async move {
                            let cached = cache::get_eth_price().await.unwrap_or(None);
                            let price = match cached {
                                Some(p) => p,
                                None => {
                                    let fetched =
                                        vm_evm::simulation::fetch_eth_price::fetch_eth_price(
                                            &vm_evm::constants::Network::Ethereum,
                                        )
                                        .await;
                                    if let Ok(p) = fetched {
                                        let _ = cache::set_eth_price(p).await;
                                        p
                                    } else {
                                        tracing::error!("[ETH_PRICE] Failed to fetch: {:?}", fetched);
                                        return;
                                    }
                                }
                            };
                            let response = StreamType::ResponsePrice(price);
                            let bytes = serde_json::to_vec(&response).unwrap_or_default();
                            let _ = nc_inner.publish(reply, bytes.into()).await;
                        });
                    }
                    _ = shutdown.cancelled() => break,
                }
            }
        }));
    }

    // ETH PRICE REFRESH
    {
        let nc = nats_client.clone();
        let shutdown = shutdown.clone();
        let qg = queue_group.to_string();
        let mut sub = nc
            .queue_subscribe(subjects::rpc::evm::PRICE_REFRESH, qg.clone())
            .await?;
        handles.push(tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = sub.next() => {
                        let Some(msg) = msg else {
                            tracing::warn!("[NATS] Subscription ended for {}, resubscribing in 1s...", subjects::rpc::evm::PRICE_REFRESH);
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            match nc.queue_subscribe(subjects::rpc::evm::PRICE_REFRESH, qg.clone()).await {
                                Ok(new_sub) => { sub = new_sub; continue; }
                                Err(e) => {
                                    tracing::error!("[NATS] Resubscribe failed for {}: {e}", subjects::rpc::evm::PRICE_REFRESH);
                                    break;
                                }
                            }
                        };
                        let Some(reply) = msg.reply.clone() else { continue };
                        let nc_inner = nc.clone();
                        tokio::spawn(async move {
                            let eth_price = vm_evm::simulation::fetch_eth_price::fetch_eth_price(
                                &vm_evm::constants::Network::Ethereum,
                            )
                            .await;
                            if let Ok(price) = eth_price {
                                let _ = cache::set_eth_price(price).await;

                                let response = StreamType::ResponsePrice(price);
                                let bytes = serde_json::to_vec(&response).unwrap_or_default();
                                if let Err(e) = nc_inner.publish(reply, bytes.into()).await {
                                    tracing::error!("[ETH_PRICE_REFRESH] Failed to publish reply: {}", e);
                                }

                                // Broadcast to event subscribers (api-server WS)
                                let broadcast = StreamInfo {
                                    user_id: -1,
                                    stream_type: StreamType::ResponsePrice(price),
                                };
                                let bytes = serde_json::to_vec(&broadcast).unwrap_or_default();
                                if let Err(e) = nc_inner
                                    .publish(subjects::events::evm::PRICE, bytes.into())
                                    .await
                                {
                                    tracing::error!("[ETH_PRICE_REFRESH] Failed to broadcast: {}", e);
                                }
                            } else {
                                tracing::error!(
                                    "[ETH_PRICE_REFRESH] Failed to fetch price: {:?}",
                                    eth_price
                                );
                            }
                        });
                    }
                    _ = shutdown.cancelled() => break,
                }
            }
        }));
    }

    // ── Uniswap V4 pool discovery loop ─────────────────────────────────────
    {
        let db = db.clone();
        let shutdown = shutdown.clone();
        handles.push(tokio::task::spawn(async move {
            loop {
                let res = vm_evm::simulation::fetch_balances::populate_uniswap_v4_pools(
                    &db,
                    vm_evm::constants::Network::Base,
                )
                .await;
                if let Err(e) = res {
                    tracing::warn!("[EVM] V4 pool population error: {}", e);
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                    _ = shutdown.cancelled() => break,
                }
            }
        }));
    }

    // ── JetStream consumer (WorkQueue) ────────────────────────────────────
    {
        let nc = nats_client.clone();
        let db = db.clone();
        let sem = handler_semaphore.clone();
        let shutdown = shutdown.clone();

        handles.push(tokio::spawn(async move {
            let consumer_config = async_nats::jetstream::consumer::pull::Config {
                durable_name: Some("worker-evm-commands".to_string()),
                ..Default::default()
            };

            loop {
                if shutdown.is_cancelled() {
                    break;
                }

                let stream = match jetstream.get_stream("VM_EVM_COMMANDS").await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("[EVM] Failed to get VM_EVM_COMMANDS stream: {e}");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                };

                let consumer = match stream
                    .get_or_create_consumer("worker-evm-commands", consumer_config.clone())
                    .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!("[EVM] Failed to create JetStream consumer: {e}");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }
                };

                // Inner fetch loop — breaks out on stream end to re-create consumer
                loop {
                    if shutdown.is_cancelled() {
                        break;
                    }
                    let batch = tokio::time::timeout(
                        Duration::from_secs(5),
                        consumer.fetch().max_messages(10).messages(),
                    )
                    .await;
                    match batch {
                        Ok(Ok(mut messages)) => {
                            loop {
                                tokio::select! {
                                    msg = messages.next() => {
                                        let Some(Ok(msg)) = msg else { break };
                                        let subject = msg.subject.to_string();
                                        tracing::info!("[EVM JS] Received JetStream message on subject: {}", subject);
                                        match serde_json::from_slice::<StreamInfo>(&msg.payload) {
                                            Ok(stream_info) => {
                                                let nc_clone = nc.clone();
                                                let db_clone = db.clone();
                                                let sem = sem.clone();
                                                let shutdown = shutdown.clone();
                                                tokio::spawn(async move {
                                                    let _permit = sem.acquire().await;
                                                    handlers::commands::process_command(
                                                        nc_clone,
                                                        db_clone,
                                                        subject,
                                                        stream_info,
                                                        shutdown,
                                                    )
                                                    .await;
                                                });
                                            }
                                            Err(e) => {
                                                let payload_preview = String::from_utf8_lossy(&msg.payload);
                                                let preview = if payload_preview.len() > 200 { &payload_preview[..200] } else { &payload_preview };
                                                tracing::error!(
                                                    "[EVM JS] Failed to deserialize StreamInfo on subject={}: {} | payload_preview={}",
                                                    subject, e, preview,
                                                );
                                            }
                                        }
                                        let _ = msg.ack().await;
                                    }
                                    _ = shutdown.cancelled() => break,
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            tracing::warn!("[EVM] JetStream fetch error: {}", e);
                            tokio::select! {
                                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                                _ = shutdown.cancelled() => break,
                            }
                        }
                        Err(_) => continue, // timeout, retry fetch
                    }
                }
            }
        }));
    }

    tracing::info!(
        "[EVM] worker-evm started, {} handlers active",
        handles.len()
    );

    // Wait for shutdown signal
    shutdown.cancelled().await;

    // Give in-flight work time to complete (12s timeout)
    tracing::info!("Waiting up to 12s for in-flight work to complete...");
    let _ = tokio::time::timeout(
        Duration::from_secs(12),
        futures::future::join_all(handles),
    )
    .await;

    tracing::info!("worker-evm shut down cleanly");
    Ok(())
}
