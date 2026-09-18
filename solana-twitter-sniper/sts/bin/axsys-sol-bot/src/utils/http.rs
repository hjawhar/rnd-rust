use std::error::Error;

use serde_json::Value;

pub async fn send_data(
    endpoint: String,
    payload: Value,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let _result = client.post(endpoint).json(&payload).send().await;
    Ok(())
}
