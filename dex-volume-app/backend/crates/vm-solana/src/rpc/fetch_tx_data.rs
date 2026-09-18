use serde::{Deserialize, Serialize};
use solana_native_token::LAMPORTS_PER_SOL;
use solana_pubkey::Pubkey;
use solana_sdk::transaction::VersionedTransaction;
use std::error::Error;
use yellowstone_grpc_proto::geyser::SubscribeUpdateTransaction;

use crate::utils::constants::{USDC, WSOL};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxDataOutput {
    pub tx: Option<VersionedTransaction>,
    pub slot: u64,
    pub value: f64,
    pub tokens: f64,
    pub address: String,
    pub tx_hash: String,
    pub tx_type: String,
    pub token_in: String,
    pub token_out: String,
}
fn vec_to_array(vec: &[u8]) -> [u8; 32] {
    let mut array = [0u8; 32];

    // Copy elements or handle differently based on vec length
    let len = std::cmp::min(vec.len(), 32);
    array[..len].copy_from_slice(&vec[..len]);

    array
}

pub fn get_tx_data(
    tx_update: SubscribeUpdateTransaction,
) -> Result<Option<TxDataOutput>, Box<dyn Error + Send + Sync>> {
    // tracing::info!("Tx: {:#?}", tx_update);
    let tx = tx_update.transaction;
    let slot = tx_update.slot;

    let tx = match tx {
        Some(tx) => tx,
        None => return Ok(None),
    };

    let transaction = tx.transaction.clone().unwrap();
    let message = transaction.message.clone().unwrap();

    let account_keys: Vec<_> = message
        .clone()
        .account_keys
        .iter()
        .map(|x| Pubkey::new_from_array(vec_to_array(x)))
        .collect();
    let signer = account_keys[0];

    let meta = &tx.meta;
    let meta = match meta {
        Some(meta) => meta,
        None => {
            return Err("Failed to decode meta".into());
        }
    };

    let tx_type: String;
    let token_in: String;
    let token_out: String;
    let value: f64;
    let mut tokens_before = 0.0;
    let mut tokens_after = 0.0;
    let mut tokens_diff = 0.0;

    if message.instructions.len() == 1 {
        token_in = WSOL.to_string();
        token_out = WSOL.to_string();
        tx_type = "TRANSFER".to_string();
        let v: [u8; 8] = message.instructions[0].data[4..]
            .iter()
            .as_slice()
            .try_into()?;
        value = u64::from_le_bytes(v) as f64 / LAMPORTS_PER_SOL as f64;
    } else {
        let pre_token_balances_owner: Vec<_> = meta
            .clone()
            .pre_token_balances
            .iter()
            .filter(|x| x.owner == signer.to_string()).cloned()
            .collect();

        let post_token_balances_owner: Vec<_> = meta
            .clone()
            .post_token_balances
            .iter()
            .filter(|x| x.owner == signer.to_string()).cloned()
            .collect();

        let pre_balances = meta.clone().pre_balances;
        let post_balances = meta.clone().post_balances;

        let mut usdc_before = 0.0;
        let mut usdc_after = 0.0;
        #[allow(unused_assignments)]
        let mut usdc_diff = 0.0;

        let mut token: Option<String> = None;

        {
            let found_token_before = pre_token_balances_owner
                .iter()
                .find(|x| x.mint != USDC && x.mint != WSOL);

            let found_token_after = post_token_balances_owner
                .iter()
                .find(|x| x.mint != USDC && x.mint != WSOL);

            if let Some(found_token_before) = found_token_before
                && let Some(ui_token_amount) = &found_token_before.ui_token_amount {
                    tokens_before = ui_token_amount.ui_amount;
                }

            if let Some(found_token_after) = found_token_after {
                token = Some(found_token_after.mint.clone());
                if let Some(ui_token_amount) = &found_token_after.ui_token_amount {
                    tokens_after = ui_token_amount.ui_amount;
                }
            }

            tokens_diff = tokens_after - tokens_before;
        }

        {
            let found_token_before = pre_token_balances_owner
                .iter()
                .find(|x| x.mint == USDC);

            let found_token_after = post_token_balances_owner
                .iter()
                .find(|x| x.mint == USDC);

            if let Some(found_token_before) = found_token_before
                && let Some(ui_token_amount) = &found_token_before.ui_token_amount {
                    usdc_before = ui_token_amount.ui_amount;
                }

            if let Some(found_token_after) = found_token_after
                && let Some(ui_token_amount) = &found_token_after.ui_token_amount {
                    usdc_after = ui_token_amount.ui_amount;
                }

            usdc_diff = usdc_after - usdc_before;
        }

        let balance_diff = post_balances[0] as f64 - pre_balances[0] as f64;

        // println!("{:#?}", account_keys);
        // println!("Total keys: {:#?}", account_keys.len());
        // println!(
        //     "Balances pre ({:#?}) - post ({:#?})",
        //     pre_balances.len(),
        //     post_balances.len()
        // );
        // println!(
        //     "{:#?}",
        //     bs58::encode(transaction.signatures[0].clone()).into_string()
        // );

        // println!("Tokens diff: {:#?}", tokens_diff);
        // println!("Balance diff: {:#?}", balance_diff);
        tx_type = if tokens_diff > 0.0 {
            "BUY".to_string()
        } else {
            "SELL".to_string()
        };

        if tx_type == "BUY" {
            if usdc_diff.abs() > 0.0 {
                token_in = USDC.to_string();
                token_out = token.unwrap_or("".to_string());
                value = usdc_diff.abs() / 1e6;
            } else {
                token_in = WSOL.to_string();
                token_out = token.unwrap_or("".to_string());
                value = balance_diff.abs() / 1e9;
            }
        } else if usdc_diff.abs() > 0.0 {
            token_in = token.unwrap_or("".to_string());
            token_out = USDC.to_string();
            value = usdc_diff.abs() / 1e6;
        } else {
            token_in = token.unwrap_or("".to_string());
            token_out = WSOL.to_string();
            value = balance_diff.abs() / 1e9;
        }
    }

    Ok(Some(TxDataOutput {
        tx: None,
        slot,
        value,
        tokens: tokens_diff.abs(),
        address: signer.to_string(),
        tx_hash: bs58::encode(transaction.signatures[0].clone()).into_string(),
        tx_type,
        token_in,
        token_out,
    }))
}
