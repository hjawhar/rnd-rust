use vm_data::db::Database;
use vm_data::models::streams::{StreamInfo, StreamType};
use vm_nats::subjects;
use tokio_util::sync::CancellationToken;

use crate::requests::strategies::{
    bundle_buy_sell::process_bundle_buy_sell, buy::process_buy, sell::process_sell,
};

pub async fn process_command(
    nc: async_nats::Client,
    db: Database,
    subject: String,
    val: StreamInfo,
    shutdown: CancellationToken,
) {
    match subject.as_str() {
        subjects::cmd::evm::TASK_START => {
            if let StreamType::EvmStartTask(task) = val.stream_type {
                // Guard: skip stale JetStream messages from before a restart
                // (self_initialize resets all statuses to "stopped")
                match db.get_project(task.project.user_id, task.project.id).await {
                    Ok(Some(p)) if p.status == "running" => {}
                    _ => {
                        tracing::info!("[EVM] TASK_START skipped for project {} (DB status is not 'running')", task.project.id);
                        return;
                    }
                }

                let _ = crate::cache::remove_task(task.project.id).await;
                let task_gen = match crate::cache::claim_task_generation(task.project.id).await {
                    Ok(generation) => generation,
                    Err(e) => {
                        tracing::warn!("[EVM] TASK_START failed to claim ownership for project {}: {}", task.project.id, e);
                        return;
                    }
                };
                tracing::info!("[EVM] TASK_START project {} generation={}", task.project.id, task_gen);

                let _ = crate::requests::strategies::volume_maker::start_volume_maker(
                    nc.clone(),
                    db,
                    task,
                    shutdown,
                    task_gen,
                )
                .await;
            }
        }
        subjects::cmd::evm::TASK_STOP => {
            if let StreamType::StopTask(project_id) = val.stream_type {
                let _ = crate::cache::remove_task(project_id).await;
                let _ = crate::cache::reset_task_failures(project_id).await;
                let _ = crate::cache::release_task_ownership(project_id).await;
                tracing::info!("[EVM] Stopped task for project {}", project_id);
            }
        }
        subjects::cmd::evm::TRADE_BUY => {
            if let StreamType::EvmBuy(task) = val.stream_type
                && let Err(e) = process_buy(nc.clone(), db.clone(), task).await {
                    tracing::error!("[EVM CMD] TRADE_BUY failed: {}", e);
                }
        }
        subjects::cmd::evm::TRADE_SELL => {
            if let StreamType::EvmSell(task) = val.stream_type
                && let Err(e) = process_sell(nc.clone(), db.clone(), task).await {
                    tracing::error!("[EVM CMD] TRADE_SELL failed: {}", e);
                }
        }
        subjects::cmd::evm::TRADE_BUNDLE_BUY_SELL => {
            if let StreamType::EvmBundleBuySell(task) = val.stream_type
                && let Err(e) = process_bundle_buy_sell(nc.clone(), db.clone(), task).await {
                    tracing::error!("[EVM CMD] TRADE_BUNDLE_BUY_SELL failed: {}", e);
                }
        }
        subjects::cmd::evm::TRACK_ADDRESS => {
            if let StreamType::TrackAddresses(payloads) = val.stream_type {
                for p in payloads {
                    let _ = crate::cache::track_address(
                        p.user_id,
                        p.project_id,
                        p.address,
                        p.mint,
                    )
                    .await;
                }
            }
        }
        subjects::cmd::evm::TRACK_UNTRACK => {
            if let StreamType::UntrackAddresses(payloads) = val.stream_type {
                for p in payloads {
                    let _ = crate::cache::remove_tracked_address(&p.address).await;
                }
            }
        }
        subjects::cmd::evm::WALLET_STORE => {
            if let StreamType::StoreWallets(payloads) = val.stream_type {
                for p in payloads {
                    let _ = crate::cache::add_wallet(vm_data::models::wallet::StoredWallet {
                        user_id: p.user_id,
                        wallet: p.wallet,
                    })
                    .await;
                }
            }
        }
        subjects::cmd::evm::COLLECT_ETH => {
            if let StreamType::ProcessCollectETH(task) = val.stream_type {
                let _ = crate::rpc::collect_eth::collect_eth(nc.clone(), task).await;
            }
        }
        subjects::cmd::evm::COLLECT_TOKENS => {
            if let StreamType::ProcessCollectEvmTokens(task) = val.stream_type {
                let _ = crate::rpc::collect_tokens::collect_tokens(nc.clone(), task).await;
            }
        }
        subjects::cmd::evm::DISPERSE_ETH => {
            if let StreamType::ProcessDisperseETH(task) = val.stream_type {
                let _ = crate::rpc::disperse_eth::disperse_eth(nc.clone(), task).await;
            }
        }
        subjects::cmd::evm::DISPERSE_TOKENS => {
            if let StreamType::ProcessDisperseEvmTokens(task) = val.stream_type {
                let _ = crate::rpc::disperse_tokens::disperse_tokens(nc.clone(), task).await;
            }
        }
        _ => {
            tracing::warn!("[EVM] Unknown command subject: {}", subject);
        }
    }
}
