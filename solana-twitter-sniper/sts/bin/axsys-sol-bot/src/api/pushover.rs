use reqwest::Client;
use std::collections::HashMap;
use std::error::Error;
use tokio::time::{sleep, Duration};

/// Sends a high priority alert to a single Pushover user key.
async fn send_high_priority_alert(
    client: &Client,
    message: &str,
    app_token: &str,
    user_key: &str,
) -> Result<(), Box<dyn Error>> {
    let url = "https://api.pushover.net/1/messages.json";

    // Prepare the form data
    let mut form = HashMap::new();
    form.insert("token", app_token);
    form.insert("user", user_key);
    form.insert("message", message);
    form.insert("priority", "1"); // High priority

    // Send the POST request
    let resp = client.post(url).form(&form).send().await?;

    // Check if the response was successful
    if resp.status().is_success() {
        tracing::info!("Alert sent successfully to {}!", user_key);
    } else {
        let text = resp.text().await?;
        tracing::info!("Failed to send alert to {}: {}", user_key, text);
    }

    Ok(())
}

pub async fn notify_all_users(msg: String) -> Result<(), Box<dyn Error + Send + Sync>> {
    let app_token = std::env::var("PUSHOVER_APP_TOKEN").expect("PUSHOVER_APP_TOKEN is required");
    let user_keys_str = std::env::var("PUSHOVER_USER_KEYS").expect("PUSHOVER_USER_KEYS is required");
    let user_keys: Vec<String> = user_keys_str.split(',').map(|s| s.trim().to_string()).collect();
    let alert_message = msg.clone();
    let client = Client::new();

    // Loop 5 times to send alerts
    for i in 0..10 {
        tracing::info!("Sending alert iteration {}", i + 1);
        for user_key in &user_keys {
            // Send alert to each user key
            let _ = send_high_priority_alert(&client, &alert_message, &app_token, user_key.as_str()).await;
        }
        // Wait for 1 second between iterations
        sleep(Duration::from_secs(5)).await;
    }
    Ok(())
}
