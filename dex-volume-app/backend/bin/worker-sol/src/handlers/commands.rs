use crate::{
    requests::strategies::{
        bundle_buy_sell::process_bundle_buy_sell,
        buy::process_buy,
        sell::process_sell,
        volume_maker::start_volume_maker,
    },
    state::get_state,
};
use vm_data::models::{
    streams::{StreamInfo, StreamType},
    wallet::StoredWallet,
};
use vm_data::utils::helpers::mul_f64_and_u64_to_u64;
use vm_nats::subjects;
use vm_solana::{
    instructions::{
        disperse_sol::disperse_sol, disperse_tokens::disperse_tokens, send_sol::send_sol,
        send_tokens::send_tokens,
    },
    rpc::fetch_tokens_balance::fetch_tokens_balance,
    utils::helpers::str_to_pk,
};
use solana_native_token::LAMPORTS_PER_SOL;
use solana_signer::Signer;
use std::sync::Arc;
use tokio::sync::mpsc::{self, Sender};

pub async fn process_command(nc: async_nats::Client, db: vm_data::db::Database, val: StreamInfo, subject: &str) {
    let state = get_state();
    match subject {
        subjects::cmd::sol::CLMM_FETCH => {
            let _ = state.fetch_clmm_configs().await;
        }
        subjects::cmd::sol::TASK_START => {
            let task = match val.stream_type {
                StreamType::StartTask(t) => t,
                _ => return,
            };

            tracing::info!("[CMD] {} received for project {}", subject, task.project.id);

            // Guard: skip stale JetStream messages
            match db.get_project(task.project.user_id, task.project.id).await {
                Ok(Some(p)) if p.status == "running" => {}
                _ => {
                    tracing::info!("[CMD] {} skipped for project {} (DB status is not 'running')", subject, task.project.id);
                    return;
                }
            }

            let project_id = task.project.id;

            // Claim ownership via Redis — returns None if another worker owns it
            let ownership = match vm_redis::task_ownership::claim_task("sol", project_id, &state.worker_id).await {
                Ok(Some(o)) => o,
                Ok(None) => {
                    tracing::warn!("[CMD] project {} owned by another worker, skipping start", project_id);
                    return;
                }
                Err(e) => {
                    tracing::error!("[CMD] failed to claim ownership for project {}: {}", project_id, e);
                    return;
                }
            };

            let task_gen = ownership.generation;

            // 1. Abort any existing volume maker loop (instant cancellation)
            state.abort_task_loop(project_id);

            // 2. Register project's pool and wallet addresses for Geyser streaming.
            //    Without this, transaction confirmations are invisible to this worker.
            let project = &task.project;
            if let Ok(Some(pair)) = state.fetch_pair(project.pool.clone()).await {
                let generic = pair.generic();
                state.track_token(
                    project.user_id,
                    project.id,
                    project.address.clone(),
                    project.pool.clone(),
                    generic.base_vault,
                    generic.quote_vault,
                );
            }
            for w in &task.wallets {
                let token_program = state.fetch_token_program(&project.address).await
                    .unwrap_or(spl_token::ID);
                state.track_address(
                    project.user_id,
                    project.id,
                    w.address.clone(),
                    project.address.clone(),
                    &token_program,
                );
                state.add_wallet(vm_data::models::wallet::StoredWallet {
                    user_id: project.user_id,
                    wallet: vm_data::models::wallet::Wallet {
                        id: w.wallet_id,
                        project_id,
                        address: w.address.clone(),
                        pk: w.private_key.clone(),
                        is_main: false,
                        date_added: std::time::SystemTime::now(),
                    },
                });
            }
            state.refresh_geyser_subscription().await;

            // 3. Update task data in local cache
            state.add_task(project_id, task.clone());
            tracing::info!("[CMD] project {} starting volume maker loop (gen={})", project_id, task_gen);
            let tx = create_internal_tx(nc.clone());
            let worker_id = state.worker_id.clone();

            // 3. Spawn the volume maker loop and store its abort handle
            let handle = tokio::spawn(async move {
                if let Err(e) = start_volume_maker(task, tx, nc, db, task_gen, worker_id).await {
                    tracing::error!("[CMD] volume_maker exited for project {}: {}", project_id, e);
                }
            });
            state.set_task_handle(project_id, handle.abort_handle());
        }
        subjects::cmd::sol::TASK_STOP => {
            if let StreamType::StopTask(project_id) = val.stream_type {
                // Release ownership in Redis (only succeeds if we are the owner)
                match vm_redis::task_ownership::release_task("sol", project_id, &state.worker_id).await {
                    Ok(true) => {
                        state.untrack_project(project_id);
                        state.remove_task(project_id);
                        state.refresh_geyser_subscription().await;
                        tracing::info!("[CMD] TASK_STOP released ownership and untracked project {}", project_id);
                    }
                    Ok(false) => {
                        tracing::info!("[CMD] TASK_STOP project {} not owned by this worker, skipping", project_id);
                    }
                    Err(e) => {
                        tracing::error!("[CMD] TASK_STOP failed to release project {}: {}", project_id, e);
                        // Still clean up local state to avoid zombie loops and stale subscriptions
                        state.untrack_project(project_id);
                        state.remove_task(project_id);
                        state.refresh_geyser_subscription().await;
                    }
                }
                // Reset failure counter in Redis
                let _ = vm_redis::task_state::reset_task_failures("sol", project_id).await;
            }
        }
        subjects::cmd::sol::TRADE_BUY => {
            if let StreamType::Buy(task) = val.stream_type {
                let _ = process_buy(task, nc.clone()).await;
            }
        }
        subjects::cmd::sol::TRADE_SELL => {
            if let StreamType::Sell(task) = val.stream_type {
                let _ = process_sell(task, nc.clone()).await;
            }
        }
        subjects::cmd::sol::TRADE_BUNDLE_BUY_SELL => {
            if let StreamType::BundleBuySell(task) = val.stream_type {
                process_bundle_buy_sell(task, nc.clone()).await;
            }
        }
        subjects::cmd::sol::TRACK_TOKEN => {
            if let StreamType::TrackToken(payload) = val.stream_type {
                let pair = state.fetch_pair(payload.pool.clone()).await;
                if let Ok(Some(pair)) = pair {
                    state.track_token(
                        payload.user_id,
                        payload.project_id,
                        payload.mint.clone(),
                        payload.pool.clone(),
                        pair.generic().base_vault,
                        pair.generic().quote_vault,
                    );

                    state.refresh_geyser_subscription().await;
                }
            }
        }
        subjects::cmd::sol::TRACK_ADDRESS => {
            if let StreamType::TrackAddresses(payload) = val.stream_type {
                tracing::info!("[TRACK_ADDRESSES] Tracking {} addresses", payload.len());
                for address in &payload {
                    let token_program = state.fetch_token_program(&address.mint).await
                        .unwrap_or(spl_token::ID);
                    state.track_address(
                        address.user_id,
                        address.project_id,
                        address.address.clone(),
                        address.mint.clone(),
                        &token_program,
                    );
                }
                tracing::info!(
                    "[TRACK_ADDRESSES] Refreshing Geyser subscription"
                );
                state.refresh_geyser_subscription().await;
            }
        }
        subjects::cmd::sol::TRACK_UNTRACK => {
            if let StreamType::UntrackAddresses(payload) = val.stream_type {
                for address in payload {
                    state.untrack_address(
                        address.user_id,
                        address.project_id,
                        address.address,
                    );
                }
                state.refresh_geyser_subscription().await;
            }
        }
        subjects::cmd::sol::TRACK_DELETE => {
            if let StreamType::DeleteAddresses(payload) = val.stream_type {
                for address in payload {
                    state.remove_wallet(&address);
                }
            }
        }
        subjects::cmd::sol::WALLET_STORE => {
           if let StreamType::StoreWallets(payload) = val.stream_type {
                for address in payload {
                    state.add_wallet(StoredWallet {
                        user_id: address.user_id,
                        wallet: address.wallet.clone(),
                    });
                }
            }
        }
        subjects::cmd::sol::COLLECT_SOL => {
            if let StreamType::ProcessCollectSOL(task) = val.stream_type {
                let rpc_client = state.rpc.clone();
                for wallet in &task.senders_pks {
                    let _ = send_sol(
                        false,
                        rpc_client.clone(),
                        wallet.clone(),
                        task.recipient_pk.clone(),
                    )
                    .await;
                }
            }
        }
        subjects::cmd::sol::COLLECT_TOKENS => {
            if let StreamType::ProcessCollectTokens(task) = val.stream_type {
                let rpc_client = state.rpc.clone();
                let token_info = state.fetch_token_info(&task.token).await.ok();
                if let Some(token_info) = token_info {
                    let token_program = state.fetch_token_program(&task.token).await
                        .unwrap_or(spl_token::ID);
                    for wallet in &task.senders_pks {
                        let _ = send_tokens(
                            false,
                            rpc_client.clone(),
                            task.token.clone(),
                            token_info.decimals,
                            wallet.clone(),
                            task.recipient_pk.clone(),
                            token_program,
                        )
                        .await;
                    }
                }
            }
        }
        subjects::cmd::sol::DISPERSE_SOL => {
            if let StreamType::ProcessDisperseSOL(task) = val.stream_type {
                let rpc_client = state.rpc.clone();
                let sender_kp = match str_to_pk(task.sender_pk.clone()) {
                    Ok(kp) => kp,
                    Err(e) => {
                        tracing::error!("[CMD] DISPERSE_SOL failed to parse sender key: {}", e);
                        return;
                    }
                };

                let balance = rpc_client
                    .clone()
                    .get_balance(&sender_kp.pubkey())
                    .await
                    .unwrap_or(0);

                let mut value_per_wallet = 0;
                let balance_keep = mul_f64_and_u64_to_u64(0.2, LAMPORTS_PER_SOL);
                if balance > balance_keep {
                    value_per_wallet =
                        (balance - balance_keep) / task.recipients_pubkeys.len() as u64;
                }

                if value_per_wallet > 0 {
                    for wallet in &task.recipients_pubkeys {
                        let _ = disperse_sol(
                            false,
                            rpc_client.clone(),
                            task.sender_pk.clone(),
                            wallet.clone(),
                            value_per_wallet as f64 / LAMPORTS_PER_SOL as f64,
                        )
                        .await;
                    }
                }
            }
        }
        subjects::cmd::sol::DISPERSE_TOKENS => {
            if let StreamType::ProcessDisperseTokens(task) = val.stream_type {
                tracing::info!("[CMD] DISPERSE_TOKENS received — token={} recipients={}", task.token, task.recipients_pubkeys.len());
                let rpc_client = state.rpc.clone();
                let sender_kp = match str_to_pk(task.sender_pk.clone()) {
                    Ok(kp) => kp,
                    Err(e) => {
                        tracing::error!("[CMD] DISPERSE_TOKENS failed to parse sender key: {}", e);
                        return;
                    }
                };
                tracing::info!("[CMD] DISPERSE_TOKENS sender={}", sender_kp.pubkey());

                let token_info = state.fetch_token_info(&task.token).await.ok();
                if token_info.is_none() {
                    tracing::error!("[CMD] DISPERSE_TOKENS failed to fetch token info for {}", task.token);
                    return;
                }
                let token_info = token_info.unwrap();
                tracing::info!("[CMD] DISPERSE_TOKENS token_info decimals={}", token_info.decimals);

                let amount = fetch_tokens_balance(
                    rpc_client.clone(),
                    sender_kp.pubkey().to_string().clone(),
                    task.token.clone(),
                )
                .await
                .unwrap_or(0.0);
                tracing::info!("[CMD] DISPERSE_TOKENS raw balance={}", amount);

                let amount = amount as u64 / task.recipients_pubkeys.len() as u64;
                tracing::info!("[CMD] DISPERSE_TOKENS per-wallet amount={} (recipients={})", amount, task.recipients_pubkeys.len());
                if amount > 0 {
                    let token_program = state.fetch_token_program(&task.token).await
                        .unwrap_or(spl_token::ID);
                    tracing::info!("[CMD] DISPERSE_TOKENS token_program={}", token_program);
                    for (i, wallet) in task.recipients_pubkeys.iter().enumerate() {
                        tracing::info!("[CMD] DISPERSE_TOKENS [{}/{}] sending {} to {}", i + 1, task.recipients_pubkeys.len(), amount, wallet);
                        match disperse_tokens(
                            false,
                            rpc_client.clone(),
                            task.token.clone(),
                            task.sender_pk.clone(),
                            wallet.clone(),
                            amount,
                            token_info.decimals,
                            token_program,
                        )
                        .await {
                            Ok(sig) => tracing::info!("[CMD] DISPERSE_TOKENS [{}/{}] success sig={}", i + 1, task.recipients_pubkeys.len(), sig),
                            Err(e) => tracing::error!("[CMD] DISPERSE_TOKENS [{}/{}] failed: {}", i + 1, task.recipients_pubkeys.len(), e),
                        }
                    }
                    tracing::info!("[CMD] DISPERSE_TOKENS complete");
                } else {
                    tracing::warn!("[CMD] DISPERSE_TOKENS amount=0, skipping");
                }
            }
        }
        _ => {
            tracing::error!("[CMD] Unknown command subject: {}", subject);
        }
    }
}

/// Creates an internal mpsc sender that forwards StreamInfo messages to NATS
/// as JetStream commands.
pub fn create_internal_tx(nc: async_nats::Client) -> Arc<Sender<StreamInfo>> {
    let (tx, mut rx) = mpsc::channel::<StreamInfo>(100);
    tokio::spawn(async move {
        while let Some(si) = rx.recv().await {
            let subject = match &si.stream_type {
                StreamType::Buy(_) => subjects::cmd::sol::TRADE_BUY,
                StreamType::Sell(_) => subjects::cmd::sol::TRADE_SELL,
                StreamType::BundleBuySell(_) => subjects::cmd::sol::TRADE_BUNDLE_BUY_SELL,
                StreamType::StartTask(_) => subjects::cmd::sol::TASK_START,
                StreamType::StopTask(_) => subjects::cmd::sol::TASK_STOP,
                StreamType::BalanceUpdate(_) => subjects::events::sol::BALANCE_UPDATE,
                StreamType::NewTransactionRequest(_) => subjects::events::sol::TRANSACTION_NEW,
                StreamType::ResponsePrice(_) => subjects::events::sol::PRICE,
                _ => {
                    tracing::error!("[BRIDGE] No subject mapping for {:?}", std::mem::discriminant(&si.stream_type));
                    continue;
                }
            };
            let bytes = serde_json::to_vec(&si).unwrap_or_default();
            let _ = nc.publish(subject, bytes.into()).await;
        }
    });
    Arc::new(tx)
}
