use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{prelude::BASE64_STANDARD, Engine};
use futures::future::join_all;
use jito_sdk_rust::JitoJsonRpcSDK;
use jupiter_swap_api_client::{swap::SwapInstructionsResponse, ClientError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{
    address_lookup_table::{state::AddressLookupTable, AddressLookupTableAccount},
    commitment_config::{CommitmentConfig, CommitmentLevel},
    instruction::{AccountMeta, Instruction},
    message::{v0::Message, VersionedMessage},
    native_token::LAMPORTS_PER_SOL,
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    system_instruction,
    transaction::VersionedTransaction,
};
use spl_associated_token_account::get_associated_token_address;
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    api::{
        bloxroute::submit_bloxroute_bundle_sol, jupiter::get_swap_instruction,
        nextblock::submit_next_block, zero_slot::send_zero_slot_tx,
    },
    models::{
        bundle_types::BundleType,
        buy::BuyRequest,
        monitor::MonitorTx,
        mpsc::{LogsType, MpscLogs, MpscTask, TaskLogs, TxStatus},
        server::Server,
        state::AppState,
    },
    utils::helpers::{get_current_time_ms, mul_f64_and_u64_to_u64},
};
use solana_sdk::hash::Hash;
use solana_sdk::pubkey;

// use jito_searcher_client::{
//     get_searcher_client_auth, get_searcher_client_no_auth, send_bundle_no_wait,
//     BlockEngineConnectionError,
// };

pub fn is_jito(input: &Pubkey) -> bool {
    let jito_pubkeys: Vec<Pubkey> = vec![
        Pubkey::from_str_const("HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe"),
        Pubkey::from_str_const("Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY"),
        Pubkey::from_str_const("ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt"),
        Pubkey::from_str_const("ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49"),
        Pubkey::from_str_const("96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5"),
        Pubkey::from_str_const("DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh"),
        Pubkey::from_str_const("3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT"),
        Pubkey::from_str_const("DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL"),
    ];
    jito_pubkeys.iter().find(|x| x.eq(&input)).is_some()
}

pub fn is_nextblock(input: &Pubkey) -> bool {
    let nextblock_pubkeys: Vec<Pubkey> = vec![
        Pubkey::from_str_const("NextbLoCkVtMGcV47JzewQdvBpLqT9TxQFozQkN98pE"),
        Pubkey::from_str_const("NexTbLoCkWykbLuB1NkjXgFWkX9oAtcoagQegygXXA2"),
        Pubkey::from_str_const("NeXTBLoCKs9F1y5PJS9CKrFNNLU1keHW71rfh7KgA1X"),
        Pubkey::from_str_const("NexTBLockJYZ7QD7p2byrUa6df8ndV2WSd8GkbWqfbb"),
        Pubkey::from_str_const("neXtBLock1LeC67jYd1QdAa32kbVeubsfPNTJC1V5At"),
        Pubkey::from_str_const("nEXTBLockYgngeRmRrjDV31mGSekVPqZoMGhQEZtPVG"),
        Pubkey::from_str_const("NEXTbLoCkB51HpLBLojQfpyVAMorm3zzKg7w9NFdqid"),
        Pubkey::from_str_const("nextBLoCkPMgmG8ZgJtABeScP35qLa2AMCNKntAP7Xc"),
    ];
    nextblock_pubkeys.iter().find(|x| x.eq(&input)).is_some()
}

const JITO_TIP_ACCOUNT: &Pubkey = &pubkey!("96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5");
const NEXTBLOCK_TIP_ACCOUNT: &Pubkey = &pubkey!("NextbLoCkVtMGcV47JzewQdvBpLqT9TxQFozQkN98pE");
const BLOXROUTE_TIP_ACCOUNT: &Pubkey = &pubkey!("HWEoBxYs7ssKuudEjzjmpfJVX7Dvi7wescFsVx2L5yoY");
const ZERO_SLOT_TIP_ACCOUNT: &Pubkey = &pubkey!("Eb2KpSC8uMt9GmzyAEm5Eb1AAAgTjRaXWFjKyFXHZxF3");

const CPI_SWAP_PROGRAM_ID: Pubkey = pubkey!("8KQG1MYXru73rqobftpFjD3hBD8Ab3jaag8wbjZG63sx");
const JUPITER_PROGRAM_ID: Pubkey = pubkey!("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");
const NATIVE_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
use spl_token::ID as TOKEN_PROGRAM_ID;

use super::pushover::notify_all_users;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TransactionState {
    pub confirmed: bool,
    pub fail: bool,
    pub logs: Option<MpscLogs>,
}

pub async fn get_address_lookup_table_accounts(
    rpc_client: &Arc<RpcClient>,
    addresses: Vec<Pubkey>,
) -> Result<Vec<AddressLookupTableAccount>, ClientError> {
    let mut accounts = Vec::new();
    for key in addresses {
        if let Ok(account) = rpc_client.get_account(&key) {
            if let Ok(address_lookup_table_account) = AddressLookupTable::deserialize(&account.data)
            {
                accounts.push(AddressLookupTableAccount {
                    key,
                    addresses: address_lookup_table_account.addresses.to_vec(),
                });
            }
        }
    }
    Ok(accounts)
}

pub async fn full_buy(state: Arc<AppState>, payload: BuyRequest) {
    tracing::info!(
        "Selected pool: {} - Mint: {}",
        payload.selected_pool,
        payload.mint
    );
    if payload.selected_pool == "PUMPFUN_ONLY".to_string() && !payload.mint.ends_with("pump") {
        return;
    }
    if payload.selected_pool == "EXCLUDE_PUMPFUN".to_string() && payload.mint.ends_with("pump") {
        return;
    }
    if let Some(servers) = payload.servers.clone() {
        let servers: Vec<String> = servers.split(",").map(|x| x.to_string()).collect();
        let found = servers
            .iter()
            .find(|server| **server == state.clone().server_name);
        if let None = found {
            tracing::info!("Server not selected.");
            return;
        }
    }

    let mut bundle_types: Vec<BundleType> = vec![];
    if let Some(block_builders) = payload.block_leaders.clone() {
        let block_builders: Vec<String> =
            block_builders.split(",").map(|x| x.to_string()).collect();
        if let Some(_) = block_builders.iter().find(|x| *x == "NEXTBLOCK") {
            bundle_types.push(BundleType::NEXTBLOCK);
        };
        if let Some(_) = block_builders.iter().find(|x| *x == "JITO") {
            bundle_types.push(BundleType::JITO);
        };
        if let Some(_) = block_builders.iter().find(|x| *x == "BLOXROUTE") {
            bundle_types.push(BundleType::BLOXROUTE);
        };
        if let Some(_) = block_builders.iter().find(|x| *x == "ZERO_SLOT") {
            bundle_types.push(BundleType::ZERO_SLOT);
        };
    }

    tracing::info!("Executing buy on {}", state.server_name.clone());
    let p1 = payload.clone();
    if payload.enable_alerts {
        tokio::task::spawn(async move {
            let msg = format!("Buying {} SOL with {} tip", p1.value, p1.tip);
            let _ = notify_all_users(msg).await;
        });
    }
    tokio::task::spawn(async move {
        let start_time_task = get_current_time_ms();
        let kp = Keypair::from_base58_string(&payload.private_key);
        let mint = Pubkey::from_str_const(&payload.mint);
        let nonce_account_pubkey = Pubkey::from_str_const(&payload.nonce_account_address);

        tracing::info!("Sender: {:#?}", kp.pubkey().to_string());
        tracing::info!("Mint: {:#?}", mint.to_string());

        let _ = state
            .tx_logs
            .send(MpscLogs::Tx(TaskLogs {
                logs_type: LogsType::BUY,
                tx_hash: None,
                token: None,
                bundle_hash: None,
                sender: None,
                text: format!("Starting buy task"),
                confirmed: false,
                status: TxStatus::INIT,
                timestamp: get_current_time_ms(),
            }))
            .await;

        let value = mul_f64_and_u64_to_u64(payload.value, LAMPORTS_PER_SOL);
        let tip = mul_f64_and_u64_to_u64(payload.tip, LAMPORTS_PER_SOL);

        let rpc: Arc<RpcClient> = Arc::new(RpcClient::new(state.rpc_endpoint.clone()));

        let instr_advance_nonce_account =
            system_instruction::advance_nonce_account(&nonce_account_pubkey, &kp.pubkey());

        let mut jupiter_tasks = vec![];
        let mutex_instructions: Arc<Mutex<Vec<Instruction>>> = Arc::new(Mutex::new(vec![]));
        let mutex_instructions_1 = mutex_instructions.clone();

        let mutex_address_lookup_table_accounts: Arc<Mutex<Vec<AddressLookupTableAccount>>> =
            Arc::new(Mutex::new(vec![]));
        let mutex_address_lookup_table_accounts_1 = mutex_address_lookup_table_accounts.clone();

        let mutex_node_block_hash: Arc<Mutex<Option<Hash>>> = Arc::new(Mutex::new(None));
        let mutex_node_block_hash_1 = mutex_node_block_hash.clone();

        let jup_kp = kp.insecure_clone();
        let jup_rpc = rpc.clone();
        let state_jup = state.clone();
        jupiter_tasks.push(tokio::task::spawn(async move {
            tracing::info!("Fetching jupiter instructions");
            let mut swap_instructions: Option<SwapInstructionsResponse> = None;
            let mut swap_error: Option<ClientError> = None;
            for i in 0..20 {
                let swap_instructions_req = get_swap_instruction(
                    state_jup.jupiter_endpoint.clone(),
                    state_jup.jupiter_api_key.clone(),
                    mint,
                    jup_kp.pubkey(),
                    value,
                    (payload.slippage * 100) as u16,
                )
                .await;

                match swap_instructions_req {
                    Ok(instructions) => {
                        swap_instructions = Some(instructions);
                        break;
                    }
                    Err(err) => {
                        swap_error = Some(err);
                    }
                }
            }

            let swap_instructions = match swap_instructions {
                Some(swap_instructions) => swap_instructions,
                None => {
                    if let Some(err) = swap_error {
                        let _ = state_jup
                            .tx_logs
                            .send(MpscLogs::Tx(TaskLogs {
                                logs_type: LogsType::BUY,
                                tx_hash: None,
                                token: None,
                                bundle_hash: None,
                                sender: None,
                                text: format!("Jupiter error {}", err.to_string()),
                                confirmed: false,
                                status: TxStatus::ERROR,
                                timestamp: get_current_time_ms(),
                            }))
                            .await;
                        tracing::info!("Jupiter error {}", err.to_string());
                    }
                    return;
                }
            };
            tracing::info!("Successfully fetched jupiter instructions");

            let mut instructions: Vec<Instruction> = vec![];
            for setup_instruction in swap_instructions.clone().setup_instructions {
                instructions.push(setup_instruction);
            }

            instructions.push(swap_instructions.clone().swap_instruction);

            let (vault, _) = Pubkey::find_program_address(&[b"vault"], &CPI_SWAP_PROGRAM_ID);
            tracing::info!("Fetching ALTA");
            let address_lookup_table_accounts = get_address_lookup_table_accounts(
                &jup_rpc,
                swap_instructions.address_lookup_table_addresses,
            )
            .await
            .unwrap();
            tracing::info!("Fetched ALTA");
            let input_token_account = get_associated_token_address(&vault, &NATIVE_MINT);
            let output_token_account = get_associated_token_address(&vault, &mint);

            let mut accounts = vec![
                AccountMeta::new_readonly(NATIVE_MINT, false), // input mint
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false), // input mint program (for now, just hardcoded to SPL and not SPL 2022)
                AccountMeta::new_readonly(mint, false),             // output mint
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false), // output mint program (for now, just hardcoded to SPL and not SPL 2022)
                AccountMeta::new(vault, false),                     // vault
                AccountMeta::new(input_token_account, false),       // vault input token account
                AccountMeta::new(output_token_account, false),      // vault output token account
                AccountMeta::new_readonly(JUPITER_PROGRAM_ID, false), // jupiter program
            ];
            let remaining_accounts = swap_instructions.swap_instruction.accounts;
            accounts.extend(remaining_accounts.into_iter().map(|mut account| {
                account.is_signer = false;
                account
            }));
            *mutex_instructions_1.lock().await = instructions;
            *mutex_address_lookup_table_accounts_1.lock().await = address_lookup_table_accounts;
        }));

        let jup_rpc = rpc.clone();
        jupiter_tasks.push(tokio::task::spawn(async move {
            tracing::info!("Fetching latest block hash");
            let nonce_account = rpc
                .get_account(&nonce_account_pubkey)
                .expect("No nonce account found");
            let nonce_data = solana_rpc_client_nonce_utils::data_from_account(&nonce_account)
                .expect("Failed to fetch data from account");
            let blockhash = nonce_data.blockhash();
            *mutex_node_block_hash_1.lock().await = Some(blockhash);
            tracing::info!("Fetched latest block hash");
        }));

        let _join_rs = join_all(jupiter_tasks).await;
        tracing::info!("Starting concurrent tasks...");
        let instructions = mutex_instructions.lock().await;
        let address_lookup_table_accounts = mutex_address_lookup_table_accounts.lock().await;
        let node_block_hash = mutex_node_block_hash.lock().await.unwrap();

        if instructions.is_empty() {
            tracing::info!("Stopping task since no instructions were found");
            return;
        }

        tracing::info!("Constructing buy tx");

        let mut thread_handles: Vec<JoinHandle<()>> = vec![];
        let mut final_instructions = vec![];
        final_instructions.push(instr_advance_nonce_account);
        for ix in instructions.iter() {
            final_instructions.push(ix.clone());
        }

        for bundle_type in bundle_types {
            thread_handles.push(sign_and_send(
                state.clone(),
                payload.clone(),
                kp.insecure_clone(),
                tip,
                final_instructions.clone(),
                address_lookup_table_accounts.clone(),
                node_block_hash.clone(),
                bundle_type,
                start_time_task.clone(),
            ));
        }
        let _join_rs = join_all(thread_handles).await;
    });
}

fn sign_and_send(
    state: Arc<AppState>,
    payload: BuyRequest,
    kp: Keypair,
    tip: u64,
    incoming_instructions: Vec<Instruction>,
    address_lookup_table_accounts: Vec<AddressLookupTableAccount>,
    node_block_hash: Hash,
    bundle_type: BundleType,
    start_time_task: u128,
) -> tokio::task::JoinHandle<()> {
    return tokio::task::spawn(async move {
        let incoming_instructions = incoming_instructions.clone();
        let mut instructions = incoming_instructions.clone();

        match bundle_type {
            BundleType::JITO => {
                tracing::info!("Starting jito instructions");
                tracing::info!("Adding jito tip account instruction");
                instructions.push(system_instruction::transfer(
                    &kp.pubkey(),
                    &*JITO_TIP_ACCOUNT,
                    tip,
                ));

                tracing::info!("Initializing jito sdk");
                let jito_sdk = Arc::new(JitoJsonRpcSDK::new(
                    &state.jito_api_endpoint,
                    Some(state.jito_api_key.to_string()),
                ));

                tracing::info!("Compiling jito message");
                let message = Message::try_compile(
                    &kp.pubkey(),
                    &instructions,
                    &address_lookup_table_accounts,
                    node_block_hash,
                )
                .unwrap();

                tracing::info!("Signing jito transaction");
                let tx =
                    VersionedTransaction::try_new(VersionedMessage::V0(message), &[&kp]).unwrap();

                tracing::info!("Serializing jito transaction");
                let mut serialized_txs_first_bundle: Vec<String> = vec![];
                let serialized_tx = bs58::encode(bincode::serialize(&tx).unwrap()).into_string();
                serialized_txs_first_bundle.push(serialized_tx);
                let first_bundle = json!(serialized_txs_first_bundle);
                tracing::info!("Sending jito bundle with 1 transaction...");
                let timestamp_jito = get_current_time_ms();
                let jito_response = jito_sdk
                    .send_bundle(Some(first_bundle), Some(&state.jito_api_key.clone()))
                    .await;
                // let jito_result = send_bundle_no_wait(&[tx.clone()], &mut jito_client).await;
                let jito_response = match jito_response {
                    Ok(response) => response,
                    Err(err) => {
                        let _ = state
                            .tx_logs
                            .send(MpscLogs::Tx(TaskLogs {
                                logs_type: LogsType::BUY,
                                tx_hash: None,
                                token: None,
                                bundle_hash: None,
                                sender: None,
                                text: format!("Jito error {err:#?}"),
                                confirmed: false,
                                status: TxStatus::ERROR,
                                timestamp: get_current_time_ms(),
                            }))
                            .await;
                        tracing::info!("Jito error {err:#?}");
                        return;
                    }
                };
                let bundle_uuid = jito_response["result"].as_str().expect("Jito bundle hash");
                tracing::info!("JITO bundle sent with UUID: {}", bundle_uuid);
                let signature_jito = bs58::encode(tx.signatures[0]).into_string();
                let _ = state
                    .tx_task
                    .send(MpscTask::TxSent(MonitorTx {
                        logs_type: LogsType::BUY,
                        uuid: payload.uuid.clone(),
                        signature: signature_jito,
                        bundle_hash: bundle_uuid.to_string(),
                        confirmed: false,
                        success: false,
                        checked: false,
                        slot: None,
                        bundle_type: BundleType::JITO,
                        task: Some(payload.clone()),
                        timestamp: timestamp_jito,
                        timelapsed: get_current_time_ms() - start_time_task,
                    }))
                    .await;
            }
            BundleType::NEXTBLOCK => {
                tracing::info!("Starting nextblock instructions");
                tracing::info!("Adding nextblock tip account instruction");
                instructions.push(system_instruction::transfer(
                    &kp.pubkey(),
                    &*NEXTBLOCK_TIP_ACCOUNT,
                    tip,
                ));

                tracing::info!("Compiling nextblock message");
                let message = Message::try_compile(
                    &kp.pubkey(),
                    &instructions,
                    &address_lookup_table_accounts,
                    node_block_hash,
                )
                .unwrap();

                tracing::info!("Signing nextblock transaction");
                let tx =
                    VersionedTransaction::try_new(VersionedMessage::V0(message), &[&kp]).unwrap();
                let signature = bs58::encode(tx.signatures[0]).into_string();
                let serialized_tx = BASE64_STANDARD.encode(bincode::serialize(&tx).unwrap());

                tracing::info!("Sending nextblock bundle with 1 transaction...");
                let nextblock_response = submit_next_block(
                    state.nextblock_endpoint.clone(),
                    state.nextblock_api_key.clone(),
                    serialized_tx,
                    payload.frontrunning_protection,
                )
                .await;
                let timestamp_nextblock = get_current_time_ms();

                let nextblock_response = match nextblock_response {
                    Ok(response) => response,
                    Err(err) => {
                        let _ = state
                            .tx_logs
                            .send(MpscLogs::Tx(TaskLogs {
                                logs_type: LogsType::BUY,
                                tx_hash: None,
                                token: None,
                                bundle_hash: None,
                                sender: None,
                                text: format!("Nextblock error {err:#?}"),
                                confirmed: false,
                                status: TxStatus::ERROR,
                                timestamp: get_current_time_ms(),
                            }))
                            .await;
                        tracing::info!("Nextblock error {err:#?}");
                        return;
                    }
                };

                if let Some(nextblock_signature) = nextblock_response.signature {
                    tracing::info!("Nextblock bundle sent with UUID: {}", nextblock_signature);
                    let _ = state
                        .tx_task
                        .send(MpscTask::TxSent(MonitorTx {
                            logs_type: LogsType::BUY,
                            uuid: payload.uuid.clone(),
                            signature,
                            bundle_hash: nextblock_signature.clone(),
                            confirmed: false,
                            success: false,
                            checked: false,
                            slot: None,
                            bundle_type: BundleType::NEXTBLOCK,
                            task: Some(payload.clone()),
                            timestamp: timestamp_nextblock,
                            timelapsed: get_current_time_ms() - start_time_task,
                        }))
                        .await;
                } else if let Some(message) = nextblock_response.message {
                    let _ = state
                        .tx_logs
                        .send(MpscLogs::Tx(TaskLogs {
                            logs_type: LogsType::BUY,
                            tx_hash: Some(bs58::encode(tx.signatures[0]).into_string()),
                            token: None,
                            bundle_hash: None,
                            sender: None,
                            text: format!("Failed to send nextblock bundle {}", message),
                            confirmed: false,
                            status: TxStatus::ERROR,
                            timestamp: get_current_time_ms(),
                        }))
                        .await;
                }
            }
            BundleType::BLOXROUTE => {
                tracing::info!("Starting bloxroute instructions");
                tracing::info!("Adding bloxroute tip account instruction");
                instructions.push(system_instruction::transfer(
                    &kp.pubkey(),
                    &*BLOXROUTE_TIP_ACCOUNT,
                    tip,
                ));

                tracing::info!("Compiling bloxroute message");
                let message = Message::try_compile(
                    &kp.pubkey(),
                    &instructions,
                    &address_lookup_table_accounts,
                    node_block_hash,
                )
                .unwrap();

                tracing::info!("Signing bloxroute transaction");
                let tx =
                    VersionedTransaction::try_new(VersionedMessage::V0(message), &[&kp]).unwrap();

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
                        let _ = state
                            .tx_logs
                            .send(MpscLogs::Tx(TaskLogs {
                                logs_type: LogsType::BUY,
                                tx_hash: None,
                                token: None,
                                bundle_hash: None,
                                sender: None,
                                text: format!("Bloxroute error {err:#?}"),
                                confirmed: false,
                                status: TxStatus::ERROR,
                                timestamp: get_current_time_ms(),
                            }))
                            .await;
                        tracing::info!("Bloxroute error {err:#?}");
                        return;
                    }
                };
                tracing::info!("Bloxroute bundle sent with UUID: {}", bundle_uuid);
                let signature_bloxroute = bs58::encode(tx.signatures[0]).into_string();
                let _ = state
                    .tx_task
                    .send(MpscTask::TxSent(MonitorTx {
                        logs_type: LogsType::BUY,
                        uuid: payload.uuid.clone(),
                        signature: signature_bloxroute,
                        bundle_hash: bundle_uuid.to_string(),
                        confirmed: false,
                        success: false,
                        checked: false,
                        slot: None,
                        bundle_type: BundleType::BLOXROUTE,
                        task: Some(payload.clone()),
                        timestamp: timestamp_bloxroute,
                        timelapsed: get_current_time_ms() - start_time_task,
                    }))
                    .await;
            }
            BundleType::ZERO_SLOT => {
                tracing::info!("Starting zero slot instructions");
                tracing::info!("Adding zero slot tip account instruction");
                instructions.push(system_instruction::transfer(
                    &kp.pubkey(),
                    &*ZERO_SLOT_TIP_ACCOUNT,
                    tip,
                ));

                tracing::info!("Compiling zero slot message");
                let message = Message::try_compile(
                    &kp.pubkey(),
                    &instructions,
                    &address_lookup_table_accounts,
                    node_block_hash,
                )
                .unwrap();

                tracing::info!("Signing zero slot transaction");
                let tx =
                    VersionedTransaction::try_new(VersionedMessage::V0(message), &[&kp]).unwrap();

                tracing::info!("Serializing zero slot transaction");
                tracing::info!("Sending zero slot bundle with 1 transaction...");
                let timestamp_zero_slot = get_current_time_ms();
                let zero_slot_result = send_zero_slot_tx(tx.clone()).await;
                let bundle_uuid = match zero_slot_result {
                    Ok(response) => response.clone(),
                    Err(err) => {
                        let _ = state
                            .tx_logs
                            .send(MpscLogs::Tx(TaskLogs {
                                logs_type: LogsType::BUY,
                                tx_hash: None,
                                token: None,
                                bundle_hash: None,
                                sender: None,
                                text: format!("Zero slot error {err:#?}"),
                                confirmed: false,
                                status: TxStatus::ERROR,
                                timestamp: get_current_time_ms(),
                            }))
                            .await;
                        tracing::info!("Zero slot error {err:#?}");
                        return;
                    }
                };
                tracing::info!("Zero slot bundle sent with UUID: {}", bundle_uuid);
                let signature_bloxroute = bs58::encode(tx.signatures[0]).into_string();
                let _ = state
                    .tx_task
                    .send(MpscTask::TxSent(MonitorTx {
                        logs_type: LogsType::BUY,
                        uuid: payload.uuid.clone(),
                        signature: signature_bloxroute,
                        bundle_hash: bundle_uuid.to_string(),
                        confirmed: false,
                        success: false,
                        checked: false,
                        slot: None,
                        bundle_type: BundleType::ZERO_SLOT,
                        task: Some(payload.clone()),
                        timestamp: timestamp_zero_slot,
                        timelapsed: get_current_time_ms() - start_time_task,
                    }))
                    .await;
            }
        }
    });
}
