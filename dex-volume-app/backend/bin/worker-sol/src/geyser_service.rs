//! Yellowstone Geyser streaming service — embedded in worker-sol
//!
//! Connects to Yellowstone Geyser gRPC for real-time Solana account/transaction updates.
//! Caches pool states and vault balances to in-memory DashMaps (no Redis round-trips).
//! Processes events directly via handlers (no NATS serialization overhead).

use crate::state::get_state;
use futures::{SinkExt, StreamExt};
use rand::Rng;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task::JoinHandle;
use tonic::{
    metadata::errors::InvalidMetadataValue,
    transport::{ClientTlsConfig, Endpoint},
};
use tonic_health::pb::health_client::HealthClient;
use yellowstone_grpc_client::{GeyserGrpcClient, InterceptorXToken};
use yellowstone_grpc_proto::{
    geyser::{
        geyser_client::GeyserClient, subscribe_update::UpdateOneof, SubscribeRequest,
        SubscribeRequestFilterAccounts, SubscribeRequestFilterSlots,
        SubscribeRequestFilterTransactions, SubscribeUpdateAccount,
        SubscribeUpdateTransaction,
    },
    prelude::{CommitmentLevel, SubscribeRequestPing},
};

// ============================================================================
// Pool & Vault Detection
// ============================================================================

/// Known pool program IDs (bs58-encoded)
const POOL_PROGRAMS: &[&str] = &[
    "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8", // Raydium AMM V4
    "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK", // Raydium CLMM
    "Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB", // Meteora DAMM
    "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG",  // Meteora DAMM V2
    "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo",  // Meteora DLMM
    "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA",  // Pumpfun AMM
];

/// SPL Token program ID (raw bytes for fast comparison)
const SPL_TOKEN_PROGRAM: [u8; 32] = [
    6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172, 28, 180, 133,
    237, 95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169,
];

fn is_pool_account(account_update: &SubscribeUpdateAccount) -> bool {
    if let Some(account_info) = &account_update.account {
        let owner = bs58::encode(&account_info.owner).into_string();
        POOL_PROGRAMS.contains(&owner.as_str())
    } else {
        false
    }
}

fn is_vault_account(account_update: &SubscribeUpdateAccount) -> bool {
    if let Some(account_info) = &account_update.account {
        account_info.data.len() >= 165 && account_info.owner == SPL_TOKEN_PROGRAM
    } else {
        false
    }
}

/// Parse SPL token balance from account data (bytes 64-72, u64 LE)
fn parse_spl_token_balance(data: &[u8]) -> Option<u64> {
    if data.len() < 72 {
        return None;
    }
    let amount_bytes: [u8; 8] = data[64..72].try_into().ok()?;
    Some(u64::from_le_bytes(amount_bytes))
}

// ============================================================================
// Geyser gRPC Connection
// ============================================================================

#[derive(Debug)]
enum GeyserEvent {
    Account(SubscribeUpdateAccount),
    Transaction(SubscribeUpdateTransaction),
}

struct GrpcStreamManager {
    client: GeyserGrpcClient<InterceptorXToken>,
    tx_mpsc: Arc<Sender<GeyserEvent>>,
}

impl GrpcStreamManager {
    async fn new(
        endpoint: &str,
        x_token: &str,
        tx_mpsc: Arc<Sender<GeyserEvent>>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let interceptor = InterceptorXToken {
            x_token: Some(x_token.parse().map_err(|e: InvalidMetadataValue| e)?),
            x_request_snapshot: true,
        };

        let tls_config = ClientTlsConfig::new().with_native_roots();
        let channel = Endpoint::from_shared(endpoint.to_string())?
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(10))
            .tls_config(tls_config)?
            .connect()
            .await?;

        let client = GeyserGrpcClient::new(
            HealthClient::with_interceptor(channel.clone(), interceptor.clone())
                .max_decoding_message_size(1024 * 1024 * 128),
            GeyserClient::with_interceptor(channel, interceptor)
                .max_decoding_message_size(1024 * 1024 * 128),
        );

        Ok(Self { client, tx_mpsc })
    }

    async fn connect(
        &mut self,
        request: SubscribeRequest,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (mut subscribe_tx, mut stream) = self
            .client
            .subscribe_with_request(Some(request))
            .await?;

        while let Some(message) = stream.next().await {
            match message {
                Ok(msg) => match msg.update_oneof {
                    Some(UpdateOneof::Account(account)) => {
                        let _ = self.tx_mpsc.send(GeyserEvent::Account(account)).await;
                    }
                    Some(UpdateOneof::Transaction(transaction)) => {
                        let _ = self
                            .tx_mpsc
                            .send(GeyserEvent::Transaction(transaction))
                            .await;
                    }
                    Some(UpdateOneof::Ping(_)) => {
                        subscribe_tx
                            .send(SubscribeRequest {
                                ping: Some(SubscribeRequestPing { id: 1 }),
                                ..Default::default()
                            })
                            .await?;
                    }
                    Some(UpdateOneof::Pong(_)) | Some(UpdateOneof::Slot(_)) => {}
                    _ => {}
                },
                Err(err) => {
                    tracing::error!("[GEYSER] Stream error: {:?}", err);
                    break;
                }
            }
        }

        Ok(())
    }
}

async fn start_geyser_stream(
    endpoint: String,
    token: String,
    accounts: Vec<String>,
    tx_mpsc: Arc<Sender<GeyserEvent>>,
) {
    const INITIAL_DELAY_SECS: f64 = 1.0;
    const MAX_DELAY_SECS: f64 = 60.0;
    const BACKOFF_MULTIPLIER: f64 = 2.0;

    let mut delay_secs = INITIAL_DELAY_SECS;

    loop {
        let req = SubscribeRequest {
            slots: HashMap::from_iter(vec![(
                "slots".to_string(),
                SubscribeRequestFilterSlots {
                    filter_by_commitment: Some(true),
                    interslot_updates: None,
                },
            )]),
            transactions: HashMap::from_iter(vec![(
                "transactions".to_string(),
                SubscribeRequestFilterTransactions {
                    vote: None,
                    failed: None,
                    signature: None,
                    account_include: accounts.clone(),
                    account_exclude: vec![],
                    account_required: vec![],
                },
            )]),
            accounts: HashMap::from_iter(vec![(
                "accounts".to_string(),
                SubscribeRequestFilterAccounts {
                    nonempty_txn_signature: None,
                    account: accounts.clone(),
                    owner: vec![],
                    filters: vec![],
                },
            )]),
            commitment: Some(CommitmentLevel::Processed as i32),
            ..Default::default()
        };

        match GrpcStreamManager::new(&endpoint, &token, tx_mpsc.clone()).await {
            Ok(mut manager) => {
                let _ = manager.connect(req).await;
                delay_secs = INITIAL_DELAY_SECS;
                tracing::info!(
                    "[GEYSER] Stream closed, reconnecting after {:.1}s...",
                    delay_secs
                );
            }
            Err(err) => {
                tracing::warn!(
                    "[GEYSER] Connection failed, reconnecting after {:.1}s... (Error: {})",
                    delay_secs,
                    err
                );
            }
        }

        let jitter = rand::rng().random_range(0.75..=1.25);
        let actual_delay = delay_secs * jitter;
        tokio::time::sleep(Duration::from_secs_f64(actual_delay)).await;

        delay_secs = (delay_secs * BACKOFF_MULTIPLIER).min(MAX_DELAY_SECS);
    }
}

// ============================================================================
// Service Entry Point
// ============================================================================

/// Start the embedded Geyser streaming service.
///
/// Spawns background tasks that:
/// 1. Listen for address list changes via mpsc channel (from AppState)
/// 2. Manage Geyser gRPC subscription (reconnect on address changes)
/// 3. Cache pool states + vault balances to in-memory DashMaps
/// 4. Process account/transaction updates via handlers directly
pub fn start_geyser_service(mut address_rx: Receiver<Vec<String>>) {
    let geyser_endpoint = match std::env::var("GEYSER_ENDPOINT") {
        Ok(v) => v,
        Err(_) => {
            tracing::error!("[GEYSER] GEYSER_ENDPOINT not set, Geyser service disabled");
            return;
        }
    };

    let geyser_token = match std::env::var("GEYSER_TOKEN") {
        Ok(v) => v,
        Err(_) => {
            tracing::error!("[GEYSER] GEYSER_TOKEN not set, Geyser service disabled");
            return;
        }
    };

    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<GeyserEvent>(1000);

    // Seed initial addresses from in-memory state
    let state = get_state();
    let initial_addresses = state.get_all_tracked_addresses();
    if !initial_addresses.is_empty() {
        tracing::info!(
            "[GEYSER] Seeded {} addresses from cache",
            initial_addresses.len()
        );
        // Send via the sender side — geyser_address_tx is the AppState sender,
        // but we need to feed initial addresses into address_rx.
        // Use a blocking try_send since we're in an async context but before the receiver loop starts.
        let state_clone = state.clone();
        tokio::spawn(async move {
            let _ = state_clone.geyser_address_tx.send(initial_addresses).await;
        });
    }

    // Subscription manager: restarts Geyser stream when addresses change
    tokio::spawn(async move {
        let mut current_handle: Option<JoinHandle<()>> = None;
        let mut old_addresses: Vec<String> = vec![];

        while let Some(mut addresses) = address_rx.recv().await {
            addresses.sort();
            if addresses == old_addresses {
                continue;
            }

            old_addresses = addresses.clone();
            tracing::info!(
                "[GEYSER] Address list changed ({} addresses), restarting subscription",
                addresses.len()
            );

            if let Some(ref handle) = current_handle {
                handle.abort();
            }

            if addresses.is_empty() {
                tracing::warn!("[GEYSER] No addresses to track, waiting...");
                current_handle = None;
                continue;
            }

            let endpoint = geyser_endpoint.clone();
            let token = geyser_token.clone();
            let tx = event_tx.clone();
            current_handle = Some(tokio::spawn(async move {
                start_geyser_stream(endpoint, token, addresses, Arc::new(tx)).await;
            }));

            // Brief debounce to coalesce rapid address changes
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    // Event processor: cache writes + direct handler calls
    tokio::spawn(async move {
        let state = get_state();
        let nc = state.nats.clone();
        let db = state.db.clone();

        while let Some(event) = event_rx.recv().await {
            match event {
                GeyserEvent::Account(account_update) => {
                    // Cache pool state (all pool accounts, regardless of tracking)
                    if is_pool_account(&account_update) {
                        cache_pool_state(&account_update);
                    }

                    // Cache vault balance (all SPL token accounts, regardless of tracking)
                    if is_vault_account(&account_update) {
                        cache_vault_balance(&account_update);
                    }

                    // Process tracked address updates (balance tracking, broadcasts)
                    let nc = nc.clone();
                    let db = db.clone();
                    tokio::spawn(async move {
                        crate::handlers::geyser::process_account_update(nc, db, account_update)
                            .await;
                    });
                }
                GeyserEvent::Transaction(tx_update) => {
                    let nc = nc.clone();
                    let db = db.clone();
                    tokio::spawn(async move {
                        crate::handlers::geyser::process_transaction_update(nc, db, tx_update)
                            .await;
                    });
                }
            }
        }
    });

    tracing::info!("[GEYSER] Service started");
}

// ============================================================================
// Cache Helpers (write to in-memory DashMaps)
// ============================================================================

/// Cache pool account state from Geyser update → AppState.pool_states DashMap
fn cache_pool_state(account_update: &SubscribeUpdateAccount) {
    if let Some(account_info) = &account_update.account {
        let address = bs58::encode(&account_info.pubkey).into_string();
        let state = get_state();
        state.set_pool_state(address, account_info.data.clone(), account_update.slot);
    }
}

/// Cache vault balance from Geyser update → AppState.vault_balances DashMap
fn cache_vault_balance(account_update: &SubscribeUpdateAccount) {
    if let Some(account_info) = &account_update.account
        && let Some(balance) = parse_spl_token_balance(&account_info.data) {
            let address = bs58::encode(&account_info.pubkey).into_string();
            let state = get_state();
            state.set_vault_balance(address, balance, account_update.slot);
        }
}
