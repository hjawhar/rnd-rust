use serde_json::{json, Value};
use std::error::Error;

pub async fn request_ocr_data(
    api_endpoint: String,
    img_url: String,
) -> Result<Value, Box<dyn Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let payload = json!({
      "url": img_url
    });

    let result = client.post(api_endpoint).json(&payload).send().await?;

    let text = result.text().await?;
    let response_json = serde_json::from_str::<Value>(text.as_str())?;
    Ok(response_json)
}
