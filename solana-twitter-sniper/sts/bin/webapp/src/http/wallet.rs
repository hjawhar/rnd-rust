use dotenv::dotenv;
use std::{collections::HashMap, error::Error, str::FromStr, sync::Arc};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use futures::future;
use serde_json::json;
use solana_pubkey::Pubkey;
use solana_rpc_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::{CommitmentConfig, CommitmentLevel},
    instruction::Instruction,
    message::Message,
    native_token::LAMPORTS_PER_SOL,
    nonce::State as NonceState,
    signature::Keypair,
    signer::Signer,
    system_instruction,
    transaction::Transaction,
};

use solana_account_decoder_client_types::UiAccountEncoding;
use solana_rpc_client_api::config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_rpc_client_api::filter::{Memcmp, RpcFilterType};
use solana_rpc_client_api::request::RpcRequest;
use solana_rpc_client_api::response::RpcKeyedAccount;

use tokio::sync::Mutex;

use crate::{
    models::{
        claims::Claims,
        state::AppState,
        wallet::{
            AddWalletsPayload, DeleteWalletsPayload, GenerateWalletPayload, NewWallet,
            ViewWalletsPayload, Wallet, WalletResponse,
        },
    },
    utils::encryption::{decrypt, encrypt},
};

use super::tasks::update_tasks_bots;

pub async fn get_wallets_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let wallets = state.get_db().get_wallets().await;
    let rpc: Arc<RpcClient> = Arc::new(RpcClient::new(state.rpc_endpoint.clone()));

    let balances: Arc<Mutex<HashMap<i32, WalletResponse>>> = Arc::new(Mutex::new(HashMap::new()));
    let a: Vec<_> = wallets
        .clone()
        .iter()
        .map(|w| {
            let rpc = rpc.clone();
            let b = balances.clone();
            let wr = w.clone();
            tokio::spawn(async move {
                let wallet_address = Pubkey::from_str_const(&wr.address);
                let balance =
                    rpc.get_balance(&wallet_address).unwrap_or(0) as f64 / LAMPORTS_PER_SOL as f64;
                let wallet_response = WalletResponse {
                    id: wr.id,
                    address: wr.address,
                    balance,
                    name: wr.name,
                    comments: wr.comments,
                    nonce_account_address: wr.nonce_account_address,
                };

                b.lock().await.insert(wr.id, wallet_response);
            })
        })
        .collect();

    future::join_all(a).await;
    let a = balances.lock().await.to_owned();
    let mut wallets_response: Vec<WalletResponse> = a.values().cloned().collect();
    wallets_response.sort_by(|a, b| b.balance.partial_cmp(&a.balance).unwrap());

    let body = Json(json!({
        "data": wallets_response
    }));
    return (StatusCode::OK, body);
}

pub async fn import_wallets_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AddWalletsPayload>,
) -> impl IntoResponse {
    let current_logged_in_user = state.get_db().get_user_by_id(claims.id).await;
    let current_logged_in_user = match current_logged_in_user {
        Some(user) => user,
        None => {
            let body = Json(json!({
                "error": format!("User not logged in properly")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }
    };

    let split_pks = payload.pks.trim().split(",");
    let mut new_wallets: Vec<NewWallet> = vec![];
    let mut idx = 1;
    for pk in split_pks {
        let encoded = bs58::decode(pk).into_vec();
        if let Ok(encoded) = encoded {
            let kp = Keypair::from_bytes(&encoded[..]);
            if let Ok(kp) = kp {
                if let Ok(encrypted_pk) = encrypt(pk.to_string()) {
                    let new_wallet = NewWallet {
                        user_id: claims.id,
                        pk: encrypted_pk,
                        address: kp.pubkey().to_string(),
                        name: format!("{}-{}", payload.name.clone(), idx),
                        comments: payload.comments.clone(),
                    };
                    new_wallets.push(new_wallet);
                    idx += 1;
                }
            }
        }
    }

    let rpc: Arc<RpcClient> = Arc::new(RpcClient::new(state.rpc_endpoint.clone()));

    let wallet_addresses = new_wallets
        .iter()
        .map(|x| x.address.clone())
        .collect::<Vec<String>>();
    let db_wallets = state
        .get_db()
        .get_wallet_by_addresses(
            if current_logged_in_user.group_id == 1 {
                None
            } else {
                Some(current_logged_in_user.id)
            },
            &wallet_addresses,
        )
        .await;

    let mut wallet_responses: Vec<WalletResponse> = vec![];
    for new_wallet in new_wallets {
        let found = db_wallets
            .iter()
            .find(|db_wallet| db_wallet.address == new_wallet.address);
        if found.is_none() {
            let created_wallet = state.get_db().add_wallet(&new_wallet).await;
            if let Ok(pubkey) = Pubkey::from_str(&created_wallet.address) {
                let balance =
                    rpc.get_balance(&pubkey).unwrap_or(0) as f64 / LAMPORTS_PER_SOL as f64;
                let wallet_response = WalletResponse {
                    id: created_wallet.id,
                    address: created_wallet.address,
                    balance,
                    name: created_wallet.name,
                    comments: created_wallet.comments,
                    nonce_account_address: None,
                };
                wallet_responses.push(wallet_response);
            }
        }
    }

    if wallet_responses.len() > 0 {
        let body = Json(json!({
            "data": wallet_responses
        }));
        return (StatusCode::OK, body);
    } else {
        let body = Json(json!({
            "error": "No wallets can be imported as some of them might be already existing"
        }));
        return (StatusCode::BAD_REQUEST, body);
    }
}

pub async fn generate_wallets_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<GenerateWalletPayload>,
) -> impl IntoResponse {
    let mut wallets: Vec<WalletResponse> = vec![];

    for idx in 1..=payload.value {
        let kp = Keypair::new();
        // let signer = PrivateKeySigner::random();
        // let pk = hex::encode(signer.credential().to_bytes().to_vec());
        let hash = encrypt(kp.to_base58_string());
        if let Ok(hash) = hash {
            let new_wallet = NewWallet {
                user_id: claims.id,
                pk: hash,
                address: kp.pubkey().to_string(),
                name: format!("{}-{}", payload.name.clone(), idx),
                comments: payload.comments.clone(),
            };
            let wallet = state.get_db().add_wallet(&new_wallet).await;
            let wallet_response = WalletResponse {
                id: wallet.id,
                address: wallet.address,
                balance: 0.0,
                name: wallet.name,
                comments: wallet.comments,
                nonce_account_address: wallet.nonce_account_address,
            };
            wallets.push(wallet_response);
        }
    }

    let body = Json(json!({
        "data": wallets
    }));
    return (StatusCode::OK, body).into_response();
}

pub async fn view_wallets_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ViewWalletsPayload>,
) -> impl IntoResponse {
    let current_logged_in_user = state.get_db().get_user_by_id(claims.id).await;
    let current_logged_in_user = match current_logged_in_user {
        Some(user) => user,
        None => {
            let body = Json(json!({
                "error": format!("User not logged in properly")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }
    };

    let wallets = state
        .get_db()
        .get_wallets_by_ids_full(
            if current_logged_in_user.group_id == 1 {
                None
            } else {
                Some(current_logged_in_user.id)
            },
            &payload.ids,
        )
        .await;
    let mut returned_wallets: Vec<Wallet> = vec![];
    for wallet in wallets {
        if let Ok(pk) = decrypt(wallet.pk) {
            let encoded = bs58::decode(pk.clone()).into_vec();
            if let Ok(encoded) = encoded {
                let kp = Keypair::from_bytes(&encoded[..]);
                if let Ok(_) = kp {
                    returned_wallets.push(Wallet { pk, ..wallet });
                }
            }
        }
    }

    if returned_wallets.len() > 0 {
        let body = Json(json!({
            "data": returned_wallets
        }));
        return (StatusCode::OK, body);
    } else {
        let body = Json(json!({
            "error": "No wallets found"
        }));
        return (StatusCode::BAD_REQUEST, body);
    }
}

pub async fn delete_wallets_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<DeleteWalletsPayload>,
) -> impl IntoResponse {
    let current_logged_in_user = state.get_db().get_user_by_id(claims.id).await;
    let current_logged_in_user = match current_logged_in_user {
        Some(user) => user,
        None => {
            let body = Json(json!({
                "error": format!("User not logged in properly")
            }));
            return (StatusCode::BAD_REQUEST, body);
        }
    };
    let deleted = state
        .get_db()
        .delete_wallets(
            if current_logged_in_user.group_id == 1 {
                None
            } else {
                Some(current_logged_in_user.id)
            },
            &payload.ids,
        )
        .await;

    if deleted {
        let body = Json(json!({
            "data": "Successfully deleted wallets"
        }));
        return (StatusCode::OK, body);
    } else {
        let body = Json(json!({
            "error": "Failed to delete wallets"
        }));
        return (StatusCode::BAD_REQUEST, body);
    }
}

pub fn get_existing_durable_nonce_account(
    user_account: Pubkey,
) -> Result<Option<Pubkey>, Box<dyn Error + Send + Sync>> {
    dotenv().ok();

    let rpc_endpoint = std::env::var("GPA_ENDPOINT")?;
    let client = RpcClient::new(rpc_endpoint);

    let token_account = Pubkey::from_str("11111111111111111111111111111111")?;
    // let user_account = Pubkey::from_str(user_account_str)?;

    let filters = vec![
        RpcFilterType::DataSize(80),
        RpcFilterType::Memcmp(Memcmp::new_raw_bytes(8, user_account.to_bytes().to_vec())),
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

pub fn create_nonce_account(payer_pk: String) -> Result<(), Box<dyn Error + Send + Sync>> {
    dotenv().ok();
    let encoded = bs58::decode(payer_pk).into_vec()?;
    let payer = Keypair::from_bytes(&encoded[..])?;
    let nonce_account_kp = Keypair::new();

    let rpc_endpoint = std::env::var("RPC_ENDPOINT").unwrap();
    let client = RpcClient::new(rpc_endpoint);

    println!("Creating durable nonce tx");
    let nonce_rent = client.get_minimum_balance_for_rent_exemption(NonceState::size())?;
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
    // println!("Latest block hash: {:#?}", blockhash);
    let test = tx.try_sign(&[payer, nonce_account_kp], blockhash);
    // println!("{test:#?}");
    // println!("{:#?}", "Signing tx");
    let sim = client.send_and_confirm_transaction(&tx)?;
    // println!("{sim:#?}");

    Ok(())
}

pub async fn create_nonce_account_req(
    claims: Claims,
    State(state): State<Arc<AppState>>,
    Path(id): Path<u32>,
) -> impl IntoResponse {
    let wallet_id = id.try_into().unwrap();
    let wallet = state.get_db().get_wallet(&wallet_id).await;
    let wallet = match wallet {
        Some(wallet) => wallet,
        None => {
            let body = Json(json!({
                "error": "Wallet not found"
            }));
            return (StatusCode::NOT_FOUND, body);
        }
    };

    tokio::task::spawn(async move {
        let user_account_pubkey = Pubkey::from_str_const(&wallet.address);
        let user_nonce_account = get_existing_durable_nonce_account(user_account_pubkey);
        let user_nonce_account = match user_nonce_account {
            Ok(user_nonce_account) => user_nonce_account,
            Err(err) => {
                return;
            }
        };

        if let Some(user_nonce_account) = user_nonce_account {
            let updated = state
                .get_db()
                .update_wallet_account_nonce(&wallet_id, &user_nonce_account.to_string())
                .await;
            if updated {
                update_tasks_bots(state.clone()).await;
            }
        } else {
            if let Ok(pk) = decrypt(wallet.pk.clone()) {
                let res = create_nonce_account(pk);
                match res {
                    Ok(_) => update_tasks_bots(state.clone()).await,
                    Err(errr) => {}
                }
            }
        }
    });
    let body = Json(json!({
        "message": "Successfully updated nonce account"
    }));
    return (StatusCode::OK, body);
}
