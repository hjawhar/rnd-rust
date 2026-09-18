use {
    crate::models::mpsc::{GeyserProcessedTx, GeyserTracker, MpscTask},
    anyhow::Result,
    futures::{future::join_all, sink::SinkExt, stream::StreamExt},
    log::error,
    std::{collections::HashMap, sync::Arc, time::Duration},
    tokio::{
        sync::{mpsc::Receiver, Mutex},
        task::JoinHandle,
    },
    tonic::{metadata::errors::InvalidMetadataValue, transport::Endpoint},
    tonic_health::pb::health_client::HealthClient,
    yellowstone_grpc_client::{GeyserGrpcClient, InterceptorXToken},
    yellowstone_grpc_proto::{
        geyser::{
            geyser_client::GeyserClient, subscribe_update::UpdateOneof, SubscribeRequest,
            SubscribeRequestFilterSlots, SubscribeRequestFilterTransactions,
        },
        prelude::{CommitmentLevel, SubscribeRequestPing},
    },
};

use tokio::sync::mpsc::Sender;
use tonic::transport::ClientTlsConfig;

struct GrpcStreamManager {
    client: GeyserGrpcClient<InterceptorXToken>,
    is_connected: bool,
    reconnect_attempts: u32,
    max_reconnect_attempts: u32,
    reconnect_interval: Duration,
    tx_mpsc: Sender<MpscTask>,
}

impl GrpcStreamManager {
    async fn new(
        endpoint: &str,
        x_token: &str,
        tx_mpsc: Sender<MpscTask>,
    ) -> Result<Arc<Mutex<GrpcStreamManager>>> {
        let interceptor = InterceptorXToken {
            x_token: Some(
                x_token
                    .parse()
                    .map_err(|e: InvalidMetadataValue| anyhow::Error::from(e))?,
            ),
            x_request_snapshot: true,
        };

        let tls_config = ClientTlsConfig::new().with_native_roots();
        let channel = Endpoint::from_shared(endpoint.to_string())?
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(10))
            .tls_config(tls_config)?
            .connect()
            .await
            .map_err(|e| anyhow::Error::from(e))?;

        let client = GeyserGrpcClient::new(
            HealthClient::with_interceptor(channel.clone(), interceptor.clone()),
            GeyserClient::with_interceptor(channel, interceptor),
        );

        Ok(Arc::new(Mutex::new(GrpcStreamManager {
            client,
            is_connected: false,
            reconnect_attempts: 0,
            max_reconnect_attempts: 10,
            reconnect_interval: Duration::from_secs(5),
            tx_mpsc,
        })))
    }

    /// Establishes connection and handles the subscription stream
    ///
    /// # Arguments
    /// * `request` - The subscription request containing transaction filters and other parameters
    async fn connect(&mut self, request: SubscribeRequest) -> Result<()> {
        let request = request.clone();
        let (mut subscribe_tx, mut stream) = self
            .client
            .subscribe_with_request(Some(request.clone()))
            .await?;

        self.is_connected = true;
        self.reconnect_attempts = 0;

        while let Some(message) = stream.next().await {
            match message {
                Ok(msg) => {
                    match msg.update_oneof {
                        Some(UpdateOneof::Slot(slot)) => {
                            let _ = self.tx_mpsc.send(MpscTask::Slot(slot.slot)).await;
                        }
                        Some(UpdateOneof::Transaction(transaction_update)) => {
                            if let Some(transaction) = &transaction_update.transaction {
                                let mut success = false;
                                if let Some(meta) = &transaction.meta {
                                    success = meta.err.is_none();
                                }
                                let signature: String =
                                    bs58::encode(&transaction.signature).into_string();
                                let process_tx = GeyserProcessedTx {
                                    slot: transaction_update.slot,
                                    signature,
                                    success,
                                };

                                let _ = self
                                    .tx_mpsc
                                    .send(MpscTask::TxConfirmation(process_tx))
                                    .await;
                            }
                        }
                        Some(UpdateOneof::Ping(_)) => {
                            subscribe_tx
                                .send(SubscribeRequest {
                                    ping: Some(SubscribeRequestPing { id: 1 }),
                                    ..Default::default()
                                })
                                .await?;
                        }
                        Some(UpdateOneof::Pong(_)) => {} // Ignore pong responses
                        _ => {
                            tracing::info!("Other update received: {:?}", msg);
                        }
                    }
                }
                Err(err) => {
                    error!("Error: {:?}", err);
                    self.is_connected = false;
                    Box::pin(self.reconnect(request.clone())).await?;
                    break;
                }
            }
        }

        Ok(())
    }

    /// Attempts to reconnect when the connection is lost
    ///
    /// # Arguments
    /// * `request` - The original subscription request to reestablish the connection
    async fn reconnect(&mut self, request: SubscribeRequest) -> Result<()> {
        if self.reconnect_attempts >= self.max_reconnect_attempts {
            tracing::info!("Max reconnection attempts reached");
            return Ok(());
        }

        self.reconnect_attempts += 1;
        tracing::info!("Reconnecting... Attempt {}", self.reconnect_attempts);

        let backoff = self.reconnect_interval * std::cmp::min(self.reconnect_attempts, 5);
        tokio::time::sleep(backoff).await;

        Box::pin(self.connect(request)).await
    }
}

pub async fn start_yellowstone_service(
    endpoint: String,
    token: String,
    tx_mpsc: Sender<MpscTask>,
    tracker_mspc: Receiver<GeyserTracker>,
) -> Result<()> {
    let mut requests = vec![SubscribeRequest {
        slots: HashMap::from_iter(vec![(
            "slots".to_string(),
            SubscribeRequestFilterSlots {
                filter_by_commitment: Some(true),
                interslot_updates: None,
            },
        )]),
        commitment: Some(CommitmentLevel::Processed as i32),
        ..Default::default()
    }];

    let mut thread_handles: Vec<JoinHandle<()>> = vec![];
    let mut addresses_tracker_handles: HashMap<String, JoinHandle<()>> = HashMap::new();
    for request in requests {
        let endpoint = endpoint.clone();
        let token = token.clone();
        let tx_mpsc = tx_mpsc.clone();
        thread_handles.push(tokio::spawn(async move {
            let manager = match GrpcStreamManager::new(&endpoint, &token, tx_mpsc).await {
                Ok(manager) => manager,
                Err(err) => {
                    tracing::info!("{:#?}", err);
                    return;
                }
            };

            let mut manager_lock = manager.lock().await;
            let _ = manager_lock.connect(request).await;
        }));
    }

    thread_handles.push(tokio::task::spawn(async move {
        let mut t = tracker_mspc;
        while let Some(task) = t.recv().await {
            match task {
                GeyserTracker::Start(address) => {
                    if let None = addresses_tracker_handles.get(&address.clone()) {
                        let req = SubscribeRequest {
                            transactions: HashMap::from_iter(vec![(
                                "transactions".to_string(),
                                SubscribeRequestFilterTransactions {
                                    vote: None,
                                    failed: None,
                                    signature: None,
                                    account_include: vec![address.clone()],
                                    account_exclude: vec![],
                                    account_required: vec![],
                                },
                            )]),
                            commitment: Some(CommitmentLevel::Processed as i32),
                            ..Default::default()
                        };

                        let endpoint = endpoint.clone();
                        let token = token.clone();
                        let tx_mpsc = tx_mpsc.clone();
                        let task_spawned = tokio::spawn(async move {
                            let manager =
                                match GrpcStreamManager::new(&endpoint, &token, tx_mpsc).await {
                                    Ok(manager) => manager,
                                    Err(err) => {
                                        tracing::info!("{:#?}", err);
                                        return;
                                    }
                                };

                            let mut manager_lock = manager.lock().await;
                            let _ = manager_lock.connect(req).await;
                        });

                        tracing::info!("Tracking wallet {}", address.clone());
                        addresses_tracker_handles.insert(address.clone(), task_spawned);
                    } else {
                        tracing::info!(
                            "Wallet already exists in tracker wallets handles: {}",
                            address.clone()
                        );
                    }
                }
                GeyserTracker::Stop(address) => {
                    if let Some(task_found) = addresses_tracker_handles.get(&address.clone()) {
                        tracing::info!("Aborting tracking wallet {}", address.clone());
                        task_found.abort();
                        addresses_tracker_handles.remove(&address.clone());
                    } else {
                        tracing::info!(
                            "Wallet does not exist in tracker wallets handles: {}",
                            address.clone()
                        );
                    }
                }
            }
        }
    }));

    let _join_rs = join_all(thread_handles).await;
    Ok(())
}
