use base64::{prelude::BASE64_STANDARD, Engine};
use dotenv::dotenv;
use serde_json::{json, Value};
use solana_sdk::transaction::VersionedTransaction;
use std::error::Error;

pub async fn submit_bloxroute_bundle_evm(
    api_key: String,
    raw_tx_hashes: Vec<String>,
    block_number: String,
) -> Result<Value, Box<dyn Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let payload = json!({
     "id": "1",
     "method": "blxr_submit_bundle",
     "params": {
        "transaction": raw_tx_hashes,
        "blockchain_network": "BSC-Mainnet",
        "block_number": block_number,
        "mev_builders": {
            "all": ""
        }
      }
    });

    let result = client
        .post("https://api.blxrbdn.com")
        .json(&payload)
        .header("Authorization", api_key)
        .send()
        .await?;

    let text = result.text().await?;
    let response_json = serde_json::from_str::<Value>(text.as_str())?;
    Ok(response_json)
}

pub async fn submit_bloxroute_bundle_sol(
    tx: VersionedTransaction,
) -> Result<Value, Box<dyn Error + Send + Sync>> {
    dotenv().ok();

    let api_key = std::env::var("BLOXROUTE_API_KEY").unwrap();
    let endpoint = std::env::var("BLOXROUTE_API_ENDPOINT_SOL").unwrap();

    let base64tx = BASE64_STANDARD.encode(bincode::serialize(&tx).unwrap());
    let client = reqwest::Client::new();
    let payload = json!({
      "transaction": {
        "content": base64tx
      },
      "skipPreFlight": true,
      "fastBestEffort": true
      // "frontRunningProtection": false,
      // "useStakedRPCs": false,
      // "allowBackRun": "True",
      // "sniping": "True",
      // "revenueAddress": "6d...FY"
    });

    let result = client
        .post(endpoint)
        .json(&payload)
        .header("Authorization", api_key)
        .send()
        .await?;

    tracing::info!("Bloxroute response: {result:#?}");
    let text = result.text().await?;
    let response_json = serde_json::from_str::<Value>(text.as_str())?;
    Ok(response_json)
}
