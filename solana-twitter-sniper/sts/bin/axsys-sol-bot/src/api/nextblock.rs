use std::error::Error;

use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NextBlockResponse {
    pub signature: Option<String>,
    pub uuid: Option<String>,
    pub code: Option<i32>,
    pub message: Option<String>,
}

pub async fn submit_next_block(
    api_endpoint: String,
    api_key: String,
    signed_tx: String,
    frontrunning_protection: bool,
) -> Result<NextBlockResponse, Box<dyn Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let payload = json!({
      "transaction": {
        "content": signed_tx
      },
      "frontRunningProtection": frontrunning_protection
    });
    tracing::info!("Nextblock payload: {:#?}", payload);

    let result = client
        .post(api_endpoint)
        .json(&payload)
        .header("Authorization", api_key)
        .send()
        .await?;

    let text = result.text().await?;
    tracing::info!("Nextblock response: {text}");
    let response_json = serde_json::from_str::<NextBlockResponse>(text.as_str())?;
    Ok(response_json)
}
