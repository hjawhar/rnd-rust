use std::error::Error;
use std::sync::Arc;

use solana_pubkey::Pubkey;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;
use solana_rpc_client_api::config::RpcSendTransactionConfig;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::message::VersionedMessage;
use solana_sdk::message::v0::Message as V0Message;
use solana_sdk::native_token::LAMPORTS_PER_SOL;
use solana_system_interface::instruction as system_instruction;
use solana_sdk::{signer::Signer, transaction::VersionedTransaction};

use crate::utils::helpers::{mul_f64_and_u64_to_u64, str_to_pk};

pub async fn disperse_sol(
    sim: bool,
    rpc: Arc<RpcClient>,
    sender_pk: String,
    receiver_pukey: String,
    value: f64,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    let sender = str_to_pk(sender_pk)?;
    let receiver = Pubkey::from_str_const(&receiver_pukey);

    let mut ixs = vec![];
    let blockhash = rpc.get_latest_blockhash().await;
    let blockhash = match blockhash {
        Ok(blockhash) => blockhash,
        Err(err) => {
            return Err(format!("Failed to fetch latest blockhash {err}").into());
        }
    };

    let balance = mul_f64_and_u64_to_u64(value, LAMPORTS_PER_SOL);

    ixs.push(ComputeBudgetInstruction::set_compute_unit_price(10_000));

    ixs.push(system_instruction::transfer(
        &sender.pubkey(),
        &receiver,
        balance,
    ));

    // println!("Signer {:#?}", user_address);
    let message = V0Message::try_compile(&sender.pubkey(), &ixs, &[], blockhash).unwrap();
    let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[sender]).unwrap();
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
