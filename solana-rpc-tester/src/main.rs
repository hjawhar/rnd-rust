use std::{collections::HashMap, env, error::Error, ops::Mul, sync::Arc, time::Duration};

use base64::{Engine, prelude::BASE64_STANDARD};
use dotenv::dotenv;
use futures::future;
use serde_json::json;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::config::RpcSendTransactionConfig;
use solana_sdk::{
    bs58,
    commitment_config::{CommitmentConfig, CommitmentLevel},
    compute_budget::ComputeBudgetInstruction,
    message::{VersionedMessage, v0::Message},
    native_token::LAMPORTS_PER_SOL,
    system_instruction,
    transaction::VersionedTransaction,
};
use solana_signer::Signer;
use tokio::sync::mpsc;

use crate::yellowstone::{NewEvent, start_yellowstone_service};
pub mod yellowstone;
pub const JITO_TIP_ACCOUNT: &str = "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe";

pub async fn send_jito_bundle(
    transaction: VersionedTransaction,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    dotenv().ok();
    let client = reqwest::Client::new();
    let endpoint = std::env::var("JITO_API_ENDPOINT").expect("JITO API endpoint is required");

    let api_key = std::env::var("JITO_API_KEY").expect("JITO API key is required");
    let serialized_transaction =
        bincode::serde::encode_to_vec(&transaction, bincode::config::standard())?;
    let base64_encoded_transaction = BASE64_STANDARD.encode(serialized_transaction);

    // Build the JSON-RPC request
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendBundle",
        "params": [
            [
                base64_encoded_transaction
            ],
            {
                "encoding": "base64"
            }
        ]
    });

    // Send the request
    let response = client
        .post(format!("{endpoint}?uuid={api_key}"))
        .header("x-jito-auth", api_key)
        .json(&request_body)
        .send()
        .await?;

    // Parse the response
    let response_json: serde_json::Value = response.json().await?;
    if let Some(result) = response_json.get("result") {
        return Ok(result.as_str().unwrap().to_string());
    } else {
        return Err(format!("Something went wrong {response_json:#?}").into());
    }
}

#[tokio::main]
async fn main() {
    dotenv().ok();

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_thread_names(true)
        .with_thread_ids(true)
        .with_target(false)
        .init();

    let (tx_ys, mut rx_ys) = mpsc::channel::<NewEvent>(1000);

    let private_tx = true;
    let tip = 0.00002;
    let geyser_endpoint = env::var("GEYSER_ENDPOINT").expect("GEYSER_ENDPOINT is required");
    let geyser_api_key = env::var("GEYSER_TOKEN").expect("GEYSER_TOKEN is required");
    let rpc_client_endpoint = env::var("RPC_ENDPOINT").expect("RPC_ENDPOINT is required");
    let swqos_client_endpoint = env::var("SWQOS_ENDPOINT").expect("SWQOS_ENDPOINT is required");
    let pk = env::var("pk").expect("Private key is required");

    let mut join_handles = vec![];
    let kp = Keypair::from_base58_string(&pk);

    let kp_clone1 = kp.insecure_clone();
    let kp_clone2 = kp.insecure_clone();

    let tx_ys_clone1 = tx_ys.clone();
    let tx_ys_clone2 = tx_ys.clone();
    {
        let a = tokio::task::spawn(async move {
            let _ = start_yellowstone_service(
                geyser_endpoint.clone(),
                geyser_api_key.clone(),
                vec![kp_clone1.pubkey().to_string()],
                Arc::new(tx_ys_clone1.clone()),
            )
            .await;
        });
        join_handles.push(a);
    }

    {
        let a = tokio::task::spawn(async move {
            let mut sent_txs: HashMap<String, u64> = HashMap::new();
            let mut current_slot = 0;
            let rpc_client = Arc::new(RpcClient::new_with_commitment(
                rpc_client_endpoint,
                CommitmentConfig {
                    commitment: CommitmentLevel::Processed,
                },
            ));
            let swqos_client = Arc::new(RpcClient::new_with_commitment(
                swqos_client_endpoint,
                CommitmentConfig {
                    commitment: CommitmentLevel::Processed,
                },
            ));
            loop {
                tokio::select! {
                    val = rx_ys.recv() => {
                        handle_events(private_tx, tip, rpc_client.clone(), swqos_client.clone(), val, &mut sent_txs, &mut current_slot, kp_clone2.insecure_clone()).await;
                    }
                }
            }
        });
        join_handles.push(a);
    }

    {
        let a = tokio::task::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            loop {
                let _ = tx_ys_clone2.send(NewEvent::SendTx()).await;
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }

    future::join_all(join_handles).await;
}

pub async fn handle_events(
    private_tx: bool,
    tip: f64,
    rpc_client: Arc<RpcClient>,
    swqos_client: Arc<RpcClient>,
    val: Option<NewEvent>,
    sent_txs: &mut HashMap<String, u64>,
    current_slot: &mut u64,
    kp: Keypair,
) {
    if let Some(val) = val {
        match val {
            NewEvent::Slot(subscribe_update_slot) => {
                *current_slot = subscribe_update_slot.slot;
            }
            NewEvent::Transaction(subscribe_update_transaction) => {
                if let Some(tx) = subscribe_update_transaction.transaction {
                    let sig = bs58::encode(tx.signature).into_string();
                    if let Some(_found) = sent_txs.get(&sig) {
                        tracing::info!(
                            "Transaction {sig} confirmed at {}",
                            subscribe_update_transaction.slot
                        );
                    }
                }
            }
            NewEvent::SendTx() => {
                let blockhash = rpc_client.get_latest_blockhash().await;
                if let Ok(blockhash) = blockhash {
                    let mut ixs = vec![];

                    if private_tx {
                        ixs.push(system_instruction::transfer(
                            &kp.pubkey(),
                            &Pubkey::from_str_const(JITO_TIP_ACCOUNT),
                            mul_f64_and_u64_to_u64(tip, LAMPORTS_PER_SOL),
                        ));
                    } else {
                        ixs.push(ComputeBudgetInstruction::set_compute_unit_price(50_000));
                    }

                    ixs.push(system_instruction::transfer(
                        &kp.pubkey(),
                        &kp.pubkey(),
                        1_000_000,
                    ));

                    let message =
                        Message::try_compile(&kp.pubkey(), &ixs, &vec![], blockhash).unwrap();
                    let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[&kp])
                        .unwrap();

                    if private_tx {
                        let signature = bs58::encode(tx.signatures[0]).into_string();
                        tracing::info!(
                            "Successfully sent transaction {} at {current_slot}",
                            signature.to_string()
                        );
                        sent_txs.insert(signature.to_string(), *current_slot);

                        let jito_response = send_jito_bundle(tx).await;
                        println!("Buy jito response: {jito_response:#?}");
                    } else {
                        let signature = swqos_client
                            .send_transaction_with_config(
                                &tx,
                                RpcSendTransactionConfig {
                                    skip_preflight: true,
                                    preflight_commitment: None,
                                    encoding: None,
                                    max_retries: None,
                                    min_context_slot: None,
                                },
                            )
                            .await;

                        match signature {
                            Ok(signature) => {
                                sent_txs.insert(signature.to_string(), *current_slot);
                                tracing::info!(
                                    "Successfully sent transaction {} at {current_slot}",
                                    signature.to_string()
                                )
                            }
                            Err(err) => tracing::error!("Error sending transaction {err:#?}"),
                        }
                    }
                }
            }
            _ => {}
        };
    }
}

pub fn mul_f64_and_u64_to_u64(x: f64, y: u64) -> u64 {
    (x * y as f64) as u64
}
