use vm_data::db::Database;
use vm_data::models::streams::{StreamInfo, StreamType};
use vm_data::models::task::EvmProcessSell;
use vm_evm::constants::Network;
use vm_evm::helpers::{f64_to_token_units, f64_to_wei};
use vm_evm::models::token::{CustomPool, CustomToken};
use vm_evm::simulation::uniswap::{exec_transaction, TxWatcherInfo};
use vm_nats::subjects;
use std::error::Error;
use std::sync::Arc;

use alloy::primitives::U256;

pub async fn process_sell(
    nc: async_nats::Client,
    db: Database,
    task: EvmProcessSell,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let network = Network::from_network_name(&task.network)
        .ok_or("Unsupported network")?;

    let token: CustomToken = match serde_json::from_str(&task.token_info) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("[EVM] Sell: failed to parse token_info for project {}: {}", task.project_id, e);
            return Err(e.into());
        }
    };
    let pool: CustomPool = match serde_json::from_str(&task.pool_info) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("[EVM] Sell: failed to parse pool_info for project {}: {}", task.project_id, e);
            return Err(e.into());
        }
    };

    let amount_in = f64_to_token_units(task.tokens, token.decimals);
    let amount_out_minimum = if task.value > 0.0 {
        f64_to_wei(task.value)
    } else {
        U256::from(0)
    };

    let tx_watcher_info = TxWatcherInfo {
        project_id: task.project_id,
        wallet_id: task.wallet_id,
        user_id: task.user_id,
        eth_price: task.eth_price,
        trade_volume_usdc: task.trade_volume_usdc,
    };

    // Create mpsc channel to receive tx events and forward to NATS
    let (tx_sender, mut tx_receiver) = tokio::sync::mpsc::channel::<StreamInfo>(32);
    let nc_clone = nc.clone();
    let project_id = task.project_id;
    let user_id = task.user_id;
    let network_name = task.network.clone();
    tokio::spawn(async move {
        while let Some(stream_info) = tx_receiver.recv().await {
            if let StreamType::TradeConfirmed { project_id, volume_usdc } = &stream_info.stream_type {
                let _ = crate::cache::add_daily_volume(*project_id, *volume_usdc).await;

                // Debounced push — deduplicates bundle buy+sell legs
                crate::requests::wallets::push_financials::schedule_push_wallets_financials(
                    nc_clone.clone(),
                    db.clone(),
                    *project_id,
                    user_id,
                    network_name.clone(),
                );
                continue;
            }
            if let Ok(bytes) = serde_json::to_vec(&stream_info) {
                let _ = nc_clone
                    .publish(subjects::events::evm::TRANSACTION_NEW, bytes.into())
                    .await;
            }
        }
    });

    exec_transaction(
        Arc::new(tx_sender),
        network,
        token,
        pool,
        false,
        task.private_key,
        amount_in,
        amount_out_minimum,
        tx_watcher_info,
    )
    .await?;

    let _ = crate::cache::reset_task_failures(project_id).await;

    Ok(())
}
