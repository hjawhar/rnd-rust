use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use dotenv::dotenv;
use jito_sdk_rust::JitoJsonRpcSDK;
use serde_json::json;
use solana_account_decoder_client_types::UiAccountEncoding;
use solana_rpc_client::rpc_client::RpcClient;
use solana_rpc_client_api::config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_rpc_client_api::filter::{Memcmp, RpcFilterType};
use solana_rpc_client_api::request::RpcRequest;
use solana_rpc_client_api::response::RpcKeyedAccount;
use solana_sdk::commitment_config::{CommitmentConfig, CommitmentLevel};
use solana_sdk::message::v0::Message as V0Message;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_sdk::pubkey;
use solana_sdk::{
    instruction::Instruction,
    message::{Message, VersionedMessage},
    nonce::State,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_instruction,
    transaction::{Transaction, VersionedTransaction},
};
use std::error::Error;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
const JITO_TIP_ACCOUNT: &Pubkey = &pubkey!("96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5");
const NEXTBLOCK_TIP_ACCOUNT: &Pubkey = &pubkey!("NextbLoCkVtMGcV47JzewQdvBpLqT9TxQFozQkN98pE");
const BLOXROUTE_TIP_ACCOUNT: &Pubkey = &pubkey!("HWEoBxYs7ssKuudEjzjmpfJVX7Dvi7wescFsVx2L5yoY");
use crate::api::bloxroute::submit_bloxroute_bundle_sol;
use crate::api::nextblock::submit_next_block;
use crate::models::bundle_types::BundleType;
use crate::utils::helpers::mul_f64_and_u64_to_u64;
use client::get_current_time_ms;

pub async fn get_existing_durable_nonce_account(
) -> Result<Option<Pubkey>, Box<dyn Error + Send + Sync>> {
    dotenv().ok();

    let rpc_endpoint = std::env::var("GPA_ENDPOINT")?;
    let client = RpcClient::new(rpc_endpoint);

    let token_account = Pubkey::from_str("11111111111111111111111111111111")?;
    let mint_str = std::env::var("NONCE_AUTHORITY_PUBKEY").expect("NONCE_AUTHORITY_PUBKEY is required");
    let mint_account = Pubkey::from_str(&mint_str)?;

    let filters = vec![
        RpcFilterType::DataSize(80),
        RpcFilterType::Memcmp(Memcmp::new_raw_bytes(8, mint_account.to_bytes().to_vec())),
    ];
    let filters = RpcProgramAccountsConfig {
        filters: Some(filters),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::JsonParsed),
            data_slice: None,
            commitment: Some(CommitmentConfig {
                commitment: CommitmentLevel::Finalized,
            }),
            min_context_slot: None,
        },
        ..Default::default()
    };
    let all_user_accounts = client.send::<Vec<RpcKeyedAccount>>(
        RpcRequest::GetProgramAccounts,
        json!([token_account.to_string(), filters]),
    )?;

    if all_user_accounts.iter().len() > 0 {
        Ok(Some(Pubkey::from_str_const(&all_user_accounts[0].pubkey)))
    } else {
        Ok(None)
    }
}

pub async fn create_transfer_tx_with_nonce(
    nonce_account_pubkey: &Pubkey,
    payer: &Keypair,
    receiver: &Pubkey,
    amount: u64,
    incoming_tip: f64,
    bundle_type: BundleType,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    dotenv().ok();
    let nextblock_endpoint =
        std::env::var("NEXTBLOCK_ENDPOINT").expect("Nextblock endpoint is required");
    let nextblock_api_key =
        std::env::var("NEXTBLOCK_API_KEY").expect("Nextblock API key is required");
    let jito_api_endpoint = std::env::var("JITO_API_ENDPOINT").expect("JITO endpoint is required");
    let jito_api_key = std::env::var("JITO_API_KEY").expect("Jito API key is required");
    let tip = mul_f64_and_u64_to_u64(incoming_tip, LAMPORTS_PER_SOL);

    let rpc_endpoint = std::env::var("RPC_ENDPOINT").unwrap();
    let client = RpcClient::new(rpc_endpoint);
    let instr_transfer = system_instruction::transfer(&payer.pubkey(), receiver, amount);

    // In this example, `payer` is `nonce_account_pubkey`'s authority
    let instr_advance_nonce_account =
        system_instruction::advance_nonce_account(&nonce_account_pubkey, &payer.pubkey());

    // The `advance_nonce_account` instruction must be the first issued in
    // the transaction.

    let mut instructions: Vec<Instruction> = vec![];
    instructions.push(instr_advance_nonce_account);
    instructions.push(instr_transfer);
    // let message = Message::new(&instructions, Some(&payer.pubkey()));

    // let mut tx = Transaction::new_unsigned(message);
    // Sign the tx with nonce_account's `blockhash` instead of the
    // network's latest blockhash.
    let nonce_account = client.get_account(&nonce_account_pubkey)?;
    let nonce_data = solana_rpc_client_nonce_utils::data_from_account(&nonce_account)?;
    let blockhash = nonce_data.blockhash();

    // let message =
    //     V0Message::try_compile(&payer.pubkey(), &instructions, &vec![], blockhash).unwrap();

    // let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[&payer]).unwrap();

    // // println!("Latest block hash: {:#?}", blockhash);
    // // tx.try_sign(&[payer], blockhash)?;
    // // println!("{:#?}", "Signing tx");
    // let sim = client.send_and_confirm_transaction(&tx)?;
    // println!("{sim:#?}");

    match bundle_type {
        BundleType::JITO => {
                        tracing::info!("Starting jito instructions");
                        tracing::info!("Adding jito tip account instruction");
                        instructions.push(system_instruction::transfer(
                            &payer.pubkey(),
                            &*JITO_TIP_ACCOUNT,
                            tip,
                        ));
        
                        tracing::info!("Initializing jito sdk");
                        let jito_sdk = Arc::new(JitoJsonRpcSDK::new(
                            &jito_api_endpoint,
                            Some(jito_api_key.to_string()),
                        ));
        
                        tracing::info!("Compiling jito message");
                        let message =
                            V0Message::try_compile(&payer.pubkey(), &instructions, &vec![], blockhash).unwrap();
        
                        tracing::info!("Signing jito transaction");
                        let tx =
                            VersionedTransaction::try_new(VersionedMessage::V0(message), &[&payer]).unwrap();
        
                        tracing::info!("Serializing jito transaction");
                        let mut serialized_txs_first_bundle: Vec<String> = vec![];
                        let serialized_tx = bs58::encode(bincode::serialize(&tx).unwrap()).into_string();
                        serialized_txs_first_bundle.push(serialized_tx);
                        let first_bundle = json!(serialized_txs_first_bundle);
                        tracing::info!("Sending jito bundle with 1 transaction...");
                        let timestamp_jito = get_current_time_ms();
                        let jito_response = jito_sdk
                            .send_bundle(Some(first_bundle), Some(&jito_api_key.clone()))
                            .await;
                        // let jito_result = send_bundle_no_wait(&[tx.clone()], &mut jito_client).await;
                        let jito_response = match jito_response {
                            Ok(response) => response,
                            Err(err) => {
                                panic!("Jito error {err:#?}");
                            }
                        };
                        let bundle_uuid = jito_response["result"].as_str().expect("Jito bundle hash");
                        tracing::info!("JITO bundle sent with UUID: {}", bundle_uuid);
                        let signature_jito = bs58::encode(tx.signatures[0]).into_string();
            }
        BundleType::NEXTBLOCK => {
                tracing::info!("Starting nextblock instructions");
                tracing::info!("Adding nextblock tip account instruction");
                instructions.push(system_instruction::transfer(
                    &payer.pubkey(),
                    &*NEXTBLOCK_TIP_ACCOUNT,
                    tip,
                ));

                tracing::info!("Compiling nextblock message");
                let message =
                    V0Message::try_compile(&payer.pubkey(), &instructions, &vec![], blockhash).unwrap();

                tracing::info!("Signing nextblock transaction");
                let tx =
                    VersionedTransaction::try_new(VersionedMessage::V0(message), &[&payer]).unwrap();
                let signature = bs58::encode(tx.signatures[0]).into_string();
                let serialized_tx = BASE64_STANDARD.encode(bincode::serialize(&tx).unwrap());

                tracing::info!("Sending nextblock bundle with 1 transaction...");
                let nextblock_response = submit_next_block(
                    nextblock_endpoint.clone(),
                    nextblock_api_key.clone(),
                    serialized_tx,
                    true,
                )
                .await;
                let timestamp_nextblock = get_current_time_ms();

                let nextblock_response = match nextblock_response {
                    Ok(response) => response,
                    Err(err) => {
                        panic!("Nextblock error {err:#?}");
                    }
                };

                if let Some(nextblock_signature) = nextblock_response.signature {
                    tracing::info!("Nextblock bundle sent with UUID: {}", nextblock_signature);
                } else if let Some(message) = nextblock_response.message {
                }
            }
        BundleType::BLOXROUTE => {
                tracing::info!("Starting bloxroute instructions");
                tracing::info!("Adding bloxroute tip account instruction");
                instructions.push(system_instruction::transfer(
                    &payer.pubkey(),
                    &*BLOXROUTE_TIP_ACCOUNT,
                    tip,
                ));

                tracing::info!("Compiling bloxroute message");
                let message =
                    V0Message::try_compile(&payer.pubkey(), &instructions, &vec![], blockhash).unwrap();

                tracing::info!("Signing bloxroute transaction");
                let tx =
                    VersionedTransaction::try_new(VersionedMessage::V0(message), &[&payer]).unwrap();

                tracing::info!("Serializing bloxroute transaction");
                tracing::info!("Sending bloxroute bundle with 1 transaction...");
                let timestamp_bloxroute = get_current_time_ms();
                let bloxroute_result = submit_bloxroute_bundle_sol(tx.clone()).await;
                let bundle_uuid = match bloxroute_result {
                    Ok(response) => response
                        .get("signature")
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_string()
                        .clone(),
                    Err(err) => {
                        panic!("Bloxroute error {err:#?}");
                    }
                };
                tracing::info!("Bloxroute bundle sent with UUID: {}", bundle_uuid);
                let signature_bloxroute = bs58::encode(tx.signatures[0]).into_string();
            }
        BundleType::ZERO_SLOT => todo!(),
    }

    Ok(())
}

pub fn create_nonce_account(
    payer: &Keypair,
    receiver: &Pubkey,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    dotenv().ok();
    let nonce_key = std::env::var("NONCE_ACCOUNT_KEY").expect("NONCE_ACCOUNT_KEY is required");
    let encoded = bs58::decode(&nonce_key).into_vec()?;
    let nonce_account_kp = Keypair::from_bytes(&encoded[..])?;
    let rpc_endpoint = std::env::var("RPC_ENDPOINT").unwrap();
    let client = RpcClient::new(rpc_endpoint);

    println!("Creating durable nonce tx");
    let nonce_rent = client.get_minimum_balance_for_rent_exemption(State::size())?;
    let mut instr_create_nonce_account = system_instruction::create_nonce_account(
        &payer.pubkey(),
        &nonce_account_kp.pubkey(),
        &payer.pubkey(), // Make the fee payer the nonce account authority
        nonce_rent,
    );
    println!("Nonce rent: {:#?}", nonce_rent);

    // In this example, `payer` is `nonce_account_pubkey`'s authority
    let instr_advance_nonce_account =
        system_instruction::advance_nonce_account(&nonce_account_kp.pubkey(), &payer.pubkey());

    // The `advance_nonce_account` instruction must be the first issued in
    // the transaction.

    let mut instructions: Vec<Instruction> = vec![];
    instructions.append(&mut instr_create_nonce_account);
    let message = Message::new(&instructions, Some(&payer.pubkey()));

    let mut tx = Transaction::new_unsigned(message);

    let blockhash = client.get_latest_blockhash()?;
    println!("Latest block hash: {:#?}", blockhash);
    let test = tx.try_sign(&[payer, &nonce_account_kp], blockhash);
    println!("{test:#?}");
    println!("{:#?}", "Signing tx");
    let sim = client.simulate_transaction(&tx)?;
    println!("{sim:#?}");

    Ok(())
}
