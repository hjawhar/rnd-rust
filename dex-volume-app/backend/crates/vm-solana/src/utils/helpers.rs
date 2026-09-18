use solana_sdk::signature::Keypair;
use std::{
    error::Error,
    ops::Mul,
};

pub fn mul_f64_and_u64_to_u64(x: f64, y: u64) -> u64 {
    let result = x.mul(y as f64).floor();
    if result.is_nan() || result.is_infinite() || result < 0.0 {
        return 0;
    }
    result as u64
}

pub fn str_to_pk(private_key: String) -> Result<Keypair, Box<dyn Error + Send + Sync>> {
    let encoded = bs58::decode(private_key).into_vec();
    let encoded_pk = match encoded {
        Ok(encoded_pk) => encoded_pk,
        Err(err) => {
            return Err(format!("Error decoding private key {err:#?}").into());
        }
    };
    let kp = Keypair::try_from(&encoded_pk[..]);
    let kp = match kp {
        Ok(kp) => kp,
        Err(err) => {
            return Err(format!("Error decoding keypair {err:#?}").into());
        }
    };

    Ok(kp)
}


use base64::{Engine, prelude::BASE64_STANDARD};
use serde_json::json;
use solana_sdk::transaction::VersionedTransaction;

use dotenv::dotenv;

/// Send a single transaction as a Jito bundle (convenience wrapper).
pub async fn send_jito_bundle(
    transaction: VersionedTransaction,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    send_jito_bundle_multi(vec![transaction]).await
}

/// Send multiple transactions as a single atomic Jito bundle.
/// Transactions execute in order; if any fails, the entire bundle is dropped.
pub async fn send_jito_bundle_multi(
    transactions: Vec<VersionedTransaction>,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    dotenv().ok();
    let client = reqwest::Client::new();
    let endpoint = std::env::var("JITO_API_ENDPOINT").expect("JITO API endpoint is required");
    let api_key = std::env::var("JITO_API_KEY").expect("JITO API key is required");

    let encoded_txs: Vec<String> = transactions
        .iter()
        .map(|tx| {
            let serialized = bincode::serde::encode_to_vec(tx, bincode::config::standard())
                .expect("Failed to serialize transaction");
            BASE64_STANDARD.encode(serialized)
        })
        .collect();

    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendBundle",
        "params": [
            encoded_txs,
            {
                "encoding": "base64"
            }
        ]
    });

    let response = client
        .post(format!("{endpoint}?uuid={api_key}"))
        .header("x-jito-auth", api_key)
        .json(&request_body)
        .send()
        .await?;

    let response_json: serde_json::Value = response.json().await?;
    if let Some(result) = response_json.get("result") {
        Ok(result.as_str().unwrap().to_string())
    } else {
        Err(format!("Something went wrong {response_json:#?}").into())
    }
}

/// Simulate a Jito bundle via a QuickNode-compatible `simulateBundle` RPC.
/// Requires `BUNDLE_SIMULATION_RPC` env var. Returns Ok(()) if all txs succeed,
/// Err with details if any tx fails or the RPC errors.
/// `labels` maps tx index to a human-readable name (e.g. ["BUY", "SELL"]).
pub async fn simulate_jito_bundle(
    transactions: &[VersionedTransaction],
    labels: &[&str],
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let endpoint = std::env::var("BUNDLE_SIMULATION_RPC")
        .map_err(|_| "BUNDLE_SIMULATION_RPC not set")?;

    let encoded_txs: Vec<String> = transactions
        .iter()
        .map(|tx| {
            let serialized = bincode::serde::encode_to_vec(tx, bincode::config::standard())
                .expect("Failed to serialize transaction");
            BASE64_STANDARD.encode(serialized)
        })
        .collect();

    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "simulateBundle",
        "params": [{
            "encodedTransactions": encoded_txs
        }]
    });

    let client = reqwest::Client::new();
    let response = client
        .post(&endpoint)
        .json(&request_body)
        .send()
        .await?;

    let response_json: serde_json::Value = response.json().await?;

    if let Some(error) = response_json.get("error") {
        return Err(format!("Bundle simulation RPC error: {}", error).into());
    }

    // Check each tx result for errors, include program logs
    if let Some(result) = response_json.get("result")
        && let Some(value) = result.get("value")
            && let Some(arr) = value.as_array() {
                for (i, tx_result) in arr.iter().enumerate() {
                    let label = labels.get(i).unwrap_or(&"UNKNOWN");

                    // Collect program logs
                    let logs: Vec<&str> = tx_result
                        .get("logs")
                        .and_then(|l| l.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                        .unwrap_or_default();

                    if let Some(err) = tx_result.get("err")
                        && !err.is_null() {
                            let logs_str = if logs.is_empty() {
                                "no logs".to_string()
                            } else {
                                logs.join("\n  ")
                            };
                            return Err(format!(
                                "Bundle simulation: {} (tx {}) failed: {}\n  Logs:\n  {}",
                                label, i, err, logs_str
                            ).into());
                        }
                }
            }

    Ok(())
}

pub async fn send_zero_slot_tx(
    transaction: VersionedTransaction,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    dotenv().ok();
    let client = reqwest::Client::new();
    let endpoint = std::env::var("ZERO_SLOT_ENDPOINT").expect("Zero slot endpoint is required");

    // Serialize the transaction to a base64-encoded string
    let serialized_transaction =
        bincode::serde::encode_to_vec(&transaction, bincode::config::standard())?;
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
        Ok(result.as_str().unwrap().to_string())
    } else if let Some(error) = response_json.get("error") {
        Err(format!("Failed to send transaction: {}", error).into())
    } else {
        Err("Something went wrong".to_string().into())
    }
}


pub async fn fetch_sol_price() -> Result<f64, Box<dyn std::error::Error + Send + Sync>> {
    let endpoint = "https://api.binance.com/api/v3/avgPrice?symbol=SOLUSDT";
    let reqwest_client = reqwest::Client::new();
    let uri_data = reqwest_client.get(endpoint).send().await?;
    let uri_data = uri_data.text().await?;
    let response_json = serde_json::from_str::<serde_json::Value>(uri_data.as_str())?;
    if let Some(price) = response_json.get("price")
        && let Some(price) = price.as_str()
            && let Ok(parsed) = price.parse::<f64>() {
                return Ok(parsed);
            }
    Err("Failed to fetch SOL price".into())
}