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

use crate::rpc::fetch_tokens_balance::fetch_tokens_balance;
use crate::utils::helpers::str_to_pk;

pub async fn send_tokens(
    sim: bool,
    rpc: Arc<RpcClient>,
    token: String,
    decimals: u8,
    sender_pk: String,
    receiver_pk: String,
    token_program: Pubkey,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    let mint = Pubkey::from_str_const(&token);
    let sender = str_to_pk(sender_pk)?;
    let receiver = str_to_pk(receiver_pk)?;

    let mut ixs = vec![];
    let blockhash = rpc.get_latest_blockhash().await;
    let blockhash = match blockhash {
        Ok(blockhash) => blockhash,
        Err(err) => {
            return Err(format!("Failed to fetch latest blockhash {err}").into());
        }
    };

    let mut amount =
        fetch_tokens_balance(rpc.clone(), sender.pubkey().to_string(), mint.to_string()).await?;

    amount -= 10_i32.pow(decimals as u32 - 3) as f64;

    ixs.push(ComputeBudgetInstruction::set_compute_unit_price(10_000));
    let sender_ata = get_associated_token_address_with_program_id(&sender.pubkey(), &mint, &token_program);
    let receiver_ata = get_associated_token_address_with_program_id(&receiver.pubkey(), &mint, &token_program);

    if rpc.get_account(&receiver_ata).await.is_err() {
        ixs.push(create_associated_token_account(
            &sender.pubkey(),
            &receiver.pubkey(),
            &mint,
            &token_program,
        ));
    }
    // Build TransferChecked instruction manually — spl_token::instruction::transfer_checked()
    // rejects non-spl_token program IDs, so we construct it directly for Token-2022 compat
    let mut xfer_data = Vec::with_capacity(10);
    xfer_data.push(12u8); // TransferChecked opcode
    xfer_data.extend_from_slice(&(amount as u64).to_le_bytes());
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

    // println!("Signer {:#?}", user_address);
    let message = V0Message::try_compile(&receiver.pubkey(), &ixs, &[], blockhash).unwrap();
    let tx =
        VersionedTransaction::try_new(VersionedMessage::V0(message), &[sender, receiver]).unwrap();
    let signature = bs58::encode(tx.signatures[0]).into_string();

    if sim {
        let sim = rpc.simulate_transaction(&tx).await?;
        println!("{:#?}", sim.value);
    } else {
        let signature = rpc
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
        let _signature = match signature {
            Ok(signature) => signature,
            Err(err) => return Err(format!("Failed to send transaction {err:#?}").into()),
        };
    }
    Ok(signature)
}
