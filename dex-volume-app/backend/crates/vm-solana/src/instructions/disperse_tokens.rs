use std::error::Error;
use std::sync::Arc;

use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::config::RpcSendTransactionConfig;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::message::VersionedMessage;
use solana_sdk::message::v0::Message as V0Message;
use solana_sdk::{signer::Signer, transaction::VersionedTransaction};
use spl_associated_token_account::get_associated_token_address_with_program_id;
use spl_associated_token_account::instruction::create_associated_token_account;

use crate::rpc::batched_rpc::check_ata_existence_batch;
use crate::utils::helpers::str_to_pk;

pub async fn disperse_tokens(
    sim: bool,
    rpc: Arc<RpcClient>,
    token: String,
    sender_pk: String,
    receiver_pukey: String,
    value: u64,
    decimals: u8,
    token_program: Pubkey,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    tracing::info!("[DISPERSE_TOKENS] start — mint={} receiver={} value={} decimals={} token_program={}", token, receiver_pukey, value, decimals, token_program);

    let mint = Pubkey::from_str_const(&token);
    let sender = str_to_pk(sender_pk)?;
    let receiver = Pubkey::from_str_const(&receiver_pukey);

    let sender_ata = get_associated_token_address_with_program_id(&sender.pubkey(), &mint, &token_program);
    let receiver_ata = get_associated_token_address_with_program_id(&receiver, &mint, &token_program);
    tracing::info!("[DISPERSE_TOKENS] sender={} sender_ata={} receiver_ata={}", sender.pubkey(), sender_ata, receiver_ata);

    // Batch RPC calls: blockhash + ATA existence check in parallel
    let ata_pubkeys = vec![receiver_ata];
    let (blockhash_result, ata_check_result) = tokio::join!(
        rpc.get_latest_blockhash(),
        check_ata_existence_batch(rpc.clone(), &ata_pubkeys)
    );

    let blockhash = match blockhash_result {
        Ok(blockhash) => blockhash,
        Err(err) => {
            tracing::error!("[DISPERSE_TOKENS] failed to fetch blockhash: {}", err);
            return Err(format!("Failed to fetch latest blockhash {err}").into());
        }
    };

    let ata_exists_map = match ata_check_result {
        Ok(map) => map,
        Err(err) => {
            tracing::error!("[DISPERSE_TOKENS] failed to check ATA existence: {}", err);
            return Err(err);
        }
    };
    let receiver_ata_exists = ata_exists_map.get(&receiver_ata).copied().unwrap_or(false);
    tracing::info!("[DISPERSE_TOKENS] receiver_ata_exists={}", receiver_ata_exists);

    let mut ixs = vec![];
    ixs.push(ComputeBudgetInstruction::set_compute_unit_price(10_000));

    if !receiver_ata_exists {
        tracing::info!("[DISPERSE_TOKENS] adding create_associated_token_account ix");
        ixs.push(create_associated_token_account(
            &sender.pubkey(),
            &receiver,
            &mint,
            &token_program,
        ));
    }
    // Build TransferChecked instruction manually — spl_token::instruction::transfer_checked()
    // rejects non-spl_token program IDs, so we construct it directly for Token-2022 compat
    let mut xfer_data = Vec::with_capacity(10);
    xfer_data.push(12u8); // TransferChecked opcode
    xfer_data.extend_from_slice(&value.to_le_bytes());
    xfer_data.push(decimals);
    ixs.push(solana_sdk::instruction::Instruction {
        program_id: token_program,
        accounts: vec![
            solana_sdk::instruction::AccountMeta::new(sender_ata, false),
            solana_sdk::instruction::AccountMeta::new_readonly(mint, false),
            solana_sdk::instruction::AccountMeta::new(receiver_ata, false),
            solana_sdk::instruction::AccountMeta::new_readonly(sender.pubkey(), true),
        ],
        data: xfer_data,
    });
    tracing::info!("[DISPERSE_TOKENS] built {} instructions (sim={})", ixs.len(), sim);

    let message = V0Message::try_compile(&sender.pubkey(), &ixs, &[], blockhash)?;
    let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[sender])?;
    let signature = bs58::encode(tx.signatures[0]).into_string();
    tracing::info!("[DISPERSE_TOKENS] tx signature={}", signature);

    if sim {
        let sim_result = rpc.simulate_transaction(&tx).await?;
        tracing::info!("[DISPERSE_TOKENS] simulation result: err={:?} units={:?}", sim_result.value.err, sim_result.value.units_consumed);
        if let Some(logs) = &sim_result.value.logs {
            for log in logs {
                tracing::info!("[DISPERSE_TOKENS] sim log: {}", log);
            }
        }
    } else {
        let send_result = rpc
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
        match &send_result {
            Ok(sig) => tracing::info!("[DISPERSE_TOKENS] tx sent successfully sig={}", sig),
            Err(err) => {
                tracing::error!("[DISPERSE_TOKENS] tx send failed: {:#?}", err);
                return Err(format!("Failed to send transaction {err:#?}").into());
            }
        }
    }
    Ok(signature)
}
