use std::sync::Arc;
use std::time::Duration;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;
use vm_data::models::streams::{InitDataPayload, StreamType};
use crate::models::state::AppState;

/// Spawn an INIT_DATA RPC handler for a given chain.
/// Workers call this on startup to get the full project/wallet dataset.
fn spawn_init_data_handler(
    nc: async_nats::Client,
    state: Arc<AppState>,
    shutdown: CancellationToken,
    subject: &'static str,
    network: &'static str,
) {
    tokio::task::spawn(async move {
        let mut sub = nc
            .queue_subscribe(subject, "api-server".to_string())
            .await
            .unwrap_or_else(|_| panic!("Failed to subscribe to {}", subject));
        loop {
            tokio::select! {
                msg = sub.next() => {
                    let Some(msg) = msg else {
                        tracing::warn!("[NATS] Subscription ended for {}, resubscribing in 1s...", subject);
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        match nc.queue_subscribe(subject, "api-server".to_string()).await {
                            Ok(new_sub) => { sub = new_sub; continue; }
                            Err(e) => {
                                tracing::error!("[NATS] Resubscribe failed for {}: {e}", subject);
                                break;
                            }
                        }
                    };
                    let state = state.clone();
                    tokio::spawn(async move {
                        if let Some(reply) = msg.reply {
                            let projects = state.get_db().get_all_projects_by_network(network).await.unwrap_or_default();
                            let wallets_projects = state.get_db().get_wallets_projects_by_network(network).await.unwrap_or_default();
                            let payload = InitDataPayload { projects, wallets_projects };
                            let response = StreamType::ResponseInitData(payload);
                            let bytes = serde_json::to_vec(&response).unwrap_or_default();
                            let _ = state.nats_client.publish(reply, bytes.into()).await;
                        }
                    });
                }
                _ = shutdown.cancelled() => break,
            }
        }
    });
}

pub fn setup(nc: &async_nats::Client, state: Arc<AppState>, shutdown: CancellationToken) {
    use vm_nats::subjects;
    spawn_init_data_handler(nc.clone(), state.clone(), shutdown.clone(), subjects::rpc::sol::INIT_DATA, "solana");
    spawn_init_data_handler(nc.clone(), state, shutdown, subjects::rpc::evm::INIT_DATA, "evm");
}
