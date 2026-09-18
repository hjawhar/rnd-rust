use std::{error::Error, str::FromStr};

use base64::{self, prelude::BASE64_STANDARD, Engine};
use bs58;
use reqwest::Client;
use serde_json::json;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_instruction,
    transaction::{Transaction, VersionedTransaction},
};
use tokio;

use dotenv::dotenv;
pub async fn send_zero_slot_tx(
    transaction: VersionedTransaction,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    dotenv().ok();
    let client = reqwest::Client::new();
    let endpoint = std::env::var("ZERO_SLOT_ENDPOINT").expect("Zero slot endpoint is required");

    // Serialize the transaction to a base64-encoded string
    let serialized_transaction = bincode::serialize(&transaction).unwrap();
    let base64_encoded_transaction = BASE64_STANDARD.encode(serialized_transaction);

    // Build the JSON-RPC request
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendTransaction",
        "params": [
            base64_encoded_transaction,
            {
                "encoding": "base64",
                "skipPreflight": true,
            }
        ]
    });

    // Send the request
    let response = client.post(endpoint).json(&request_body).send().await?;

    // Parse the response
    let response_json: serde_json::Value = response.json().await?;
    if let Some(result) = response_json.get("result") {
        return Ok(result.as_str().unwrap().to_string());
    } else if let Some(error) = response_json.get("error") {
        return Err(format!("Failed to send transaction: {}", error).into());
    } else {
        return Err(format!("Something went wrong").into());
    }
}
