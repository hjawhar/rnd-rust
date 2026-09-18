use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use tonic::transport::ClientTlsConfig;
use yellowstone_grpc_proto::geyser::{
    SubscribeUpdateAccount, SubscribeUpdateSlot, SubscribeUpdateTransaction,
};
use {
    async_recursion::async_recursion,
    futures::{sink::SinkExt, stream::StreamExt},
    std::{collections::HashMap, error::Error, sync::Arc, time::Duration},
    tonic::{metadata::errors::InvalidMetadataValue, transport::Endpoint},
    tonic_health::pb::health_client::HealthClient,
    yellowstone_grpc_client::{GeyserGrpcClient, InterceptorXToken},
    yellowstone_grpc_proto::{
        geyser::{
            SubscribeRequest, SubscribeRequestFilterAccounts, SubscribeRequestFilterSlots,
            SubscribeRequestFilterTransactions, geyser_client::GeyserClient,
            subscribe_update::UpdateOneof,
        },
        prelude::{CommitmentLevel, SubscribeRequestPing},
    },
};

#[derive(Debug, Clone)]
pub enum NewEvent {
    Slot(SubscribeUpdateSlot),
    Transaction(SubscribeUpdateTransaction),
    Account(SubscribeUpdateAccount),
    SendTx(),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StreamType {
    SubscribeUpdateAccount(Vec<u8>),
    SubscribeUpdateTransaction(Vec<u8>),
}

struct GrpcStreamManager {
    client: GeyserGrpcClient<InterceptorXToken>,
    is_connected: bool,
    tx_mpsc: Arc<Sender<NewEvent>>,
}

impl GrpcStreamManager {
    async fn new(
        endpoint: &str,
        x_token: &str,
        tx_mpsc: Arc<Sender<NewEvent>>,
    ) -> Result<GrpcStreamManager, Box<dyn Error + Send + Sync>> {
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
            .await
            .map_err(|e| e)?;

        let client = GeyserGrpcClient::new(
            HealthClient::with_interceptor(channel.clone(), interceptor.clone())
                .max_decoding_message_size(1024 * 1024 * 128),
            GeyserClient::with_interceptor(channel, interceptor)
                .max_decoding_message_size(1024 * 1024 * 128),
        );

        Ok(GrpcStreamManager {
            client,
            is_connected: false,
            tx_mpsc,
        })
    }

    /// Establishes connection and handles the subscription stream
    ///
    /// # Arguments
    /// * `request` - The subscription request containing transaction filters and other parameters
    async fn connect(
        &mut self,
        request: SubscribeRequest,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let request = request.clone();
        let (mut subscribe_tx, mut stream) = self
            .client
            .subscribe_with_request(Some(request.clone()))
            .await?;

        self.is_connected = true;

        while let Some(message) = stream.next().await {
            match message {
                Ok(msg) => {
                    match msg.update_oneof {
                        Some(UpdateOneof::Slot(slot)) => {
                            let _ = self.tx_mpsc.send(NewEvent::Slot(slot)).await;
                        }
                        Some(UpdateOneof::Account(account)) => {
                            let _ = self.tx_mpsc.send(NewEvent::Account(account)).await;
                        }
                        Some(UpdateOneof::Transaction(transaction_update)) => {
                            let _ = self
                                .tx_mpsc
                                .send(NewEvent::Transaction(transaction_update))
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
                        Some(UpdateOneof::Pong(_)) => {} // Ignore pong responses
                        _ => {
                            tracing::info!("Other update received: {:?}", msg);
                        }
                    }
                }
                Err(err) => {
                    tracing::error!("Error: {:?}", err);
                    self.is_connected = false;
                    break;
                }
            }
        }

        Ok(())
    }
}

#[async_recursion]
pub async fn start_yellowstone_service(
    endpoint: String,
    token: String,
    accounts: Vec<String>,
    tx_mpsc: Arc<Sender<NewEvent>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
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
            tracing::info!("Reconnecting after 5 seconds... (closed connection)");
            tokio::time::sleep(Duration::from_secs(5)).await;
            let _ = start_yellowstone_service(endpoint, token, accounts.clone(), tx_mpsc).await;
        }
        Err(err) => {
            tracing::info!("Reconnecting after 5 seconds... (Error: {:#?})", err);
            tokio::time::sleep(Duration::from_secs(5)).await;
            let _ = start_yellowstone_service(endpoint, token, accounts.clone(), tx_mpsc).await;
        }
    };

    Ok(())
}
