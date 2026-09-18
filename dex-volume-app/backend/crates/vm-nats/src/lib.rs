pub mod subscribe_loop;
pub mod subjects;
pub mod worker_bootstrap;

use async_nats::{Client, HeaderMap, Message};
use futures::StreamExt;
use std::env;
use std::time::Duration;

// ── Distributed Tracing ─────────────────────────────────────────────────────

// Task-local trace ID for distributed tracing across services.
// Set by api-server middleware via `TRACE_ID.scope(id, fut)`.
// `request_with_timeout` automatically injects it as a NATS header.
// Workers extract it from incoming NATS message headers.
tokio::task_local! {
    pub static TRACE_ID: String;
}

/// Header name used for propagating trace IDs through NATS messages.
pub const TRACE_ID_HEADER: &str = "trace-id";

/// Extract a trace-id from NATS message headers (if present).
pub fn extract_trace_id(msg: &Message) -> Option<String> {
    msg.headers
        .as_ref()
        .and_then(|h| h.get(TRACE_ID_HEADER))
        .map(|v| v.to_string())
}

// ── Connection ──────────────────────────────────────────────────────────────

/// Connect to NATS using the `NATS_URL` environment variable.
/// Retries on initial connection failure with exponential backoff.
///
/// Buffer tuning via env vars:
/// - `NATS_SUBSCRIPTION_CAPACITY` — per-subscription incoming message buffer (default: 8192)
/// - `NATS_READ_BUFFER_CAPACITY` — client read buffer capacity in KiB (optional, u16)
pub async fn connect_nats() -> Result<Client, async_nats::ConnectError> {
    let nats_url = env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    let sub_capacity: usize = env::var("NATS_SUBSCRIPTION_CAPACITY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8192);
    let read_buf_capacity: Option<u16> = env::var("NATS_READ_BUFFER_CAPACITY")
        .ok()
        .and_then(|v| v.parse().ok());

    tracing::info!(
        "[NATS] Connecting to {} (subscription_capacity={}{})",
        nats_url,
        sub_capacity,
        read_buf_capacity.map_or(String::new(), |v| format!(", read_buffer_capacity={}", v))
    );

    let mut opts = async_nats::ConnectOptions::new()
        .subscription_capacity(sub_capacity)
        .retry_on_initial_connect();
    if let Some(cap) = read_buf_capacity {
        opts = opts.read_buffer_capacity(cap);
    }
    let client = opts
        .event_callback(|event| async move {
            match event {
                async_nats::Event::Connected => {
                    tracing::info!("[NATS] Connected to server");
                }
                async_nats::Event::Disconnected => {
                    tracing::warn!("[NATS] Disconnected from server");
                }
                async_nats::Event::LameDuckMode => {
                    tracing::warn!("[NATS] Server entered lame duck mode");
                }
                async_nats::Event::SlowConsumer(subject) => {
                    tracing::warn!("[NATS] Slow consumer on subject: {}", subject);
                }
                async_nats::Event::ServerError(err) => {
                    tracing::error!("[NATS] Server error: {}", err);
                }
                async_nats::Event::ClientError(err) => {
                    tracing::error!("[NATS] Client error: {}", err);
                }
                other => {
                    tracing::debug!("[NATS] Event: {:?}", other);
                }
            }
        })
        .connect(&nats_url)
        .await?;

    tracing::info!("[NATS] Connected successfully");
    Ok(client)
}

/// Create the JetStream context and ensure the VM_SOL_COMMANDS stream exists.
pub async fn setup_jetstream(
    client: &Client,
) -> Result<async_nats::jetstream::Context, Box<dyn std::error::Error + Send + Sync>> {
    let jetstream = async_nats::jetstream::new(client.clone());


    // Create or update the VM_SOL_COMMANDS stream
    let stream_config = async_nats::jetstream::stream::Config {
        name: "VM_SOL_COMMANDS".to_string(),
        subjects: vec![subjects::cmd::sol::ALL.to_string()],
        retention: async_nats::jetstream::stream::RetentionPolicy::WorkQueue,
        max_age: Duration::from_secs(3600), // 1 hour
        storage: async_nats::jetstream::stream::StorageType::File,
        ..Default::default()
    };

    jetstream
        .get_or_create_stream(stream_config)
        .await?;

    tracing::info!("[NATS] JetStream VM_SOL_COMMANDS stream ready");
    Ok(jetstream)
}

/// Create the JetStream context and ensure the VM_EVM_COMMANDS stream exists.
pub async fn setup_evm_jetstream(
    client: &Client,
) -> Result<async_nats::jetstream::Context, Box<dyn std::error::Error + Send + Sync>> {
    let jetstream = async_nats::jetstream::new(client.clone());

    let stream_config = async_nats::jetstream::stream::Config {
        name: "VM_EVM_COMMANDS".to_string(),
        subjects: vec![subjects::cmd::evm::ALL.to_string()],
        retention: async_nats::jetstream::stream::RetentionPolicy::WorkQueue,
        max_age: Duration::from_secs(3600),
        storage: async_nats::jetstream::stream::StorageType::File,
        ..Default::default()
    };

    jetstream.get_or_create_stream(stream_config).await?;

    tracing::info!("[NATS] JetStream VM_EVM_COMMANDS stream ready");
    Ok(jetstream)
}

// ── Request-Reply ───────────────────────────────────────────────────────────

/// Build NATS headers with trace-id if the task-local is set.
fn build_trace_headers() -> Option<HeaderMap> {
    TRACE_ID.try_with(|id| {
        let mut headers = HeaderMap::new();
        headers.insert(TRACE_ID_HEADER, id.as_str());
        headers
    }).ok()
}

/// Make a NATS request with a custom timeout.
///
/// Automatically injects `trace-id` NATS header when the `TRACE_ID` task-local
/// is set (by api-server middleware). Zero changes needed at call sites.
///
/// **For timeouts ≤30s:** Uses standard `client.request()` wrapped in `tokio::time::timeout()`
/// **For timeouts >30s:** Uses manual inbox pattern to bypass NATS client's built-in timeout
pub async fn request_with_timeout(
    client: &Client,
    subject: impl Into<String>,
    payload: Vec<u8>,
    timeout: Duration,
) -> Result<Message, Box<dyn std::error::Error + Send + Sync>> {
    let subject = subject.into();
    let headers = build_trace_headers();

    if timeout <= Duration::from_secs(30) {
        let result = if let Some(headers) = headers {
            tokio::time::timeout(timeout, client.request_with_headers(subject, headers, payload.into())).await
        } else {
            tokio::time::timeout(timeout, client.request(subject, payload.into())).await
        };
        match result {
            Ok(Ok(msg)) => Ok(msg),
            Ok(Err(e)) => Err(format!("NATS request failed: {}", e).into()),
            Err(_) => Err(format!("Request timed out after {:?}", timeout).into()),
        }
    } else {
        // For long timeouts, use manual inbox pattern to bypass client timeout
        let inbox = client.new_inbox();
        let mut subscription = client.subscribe(inbox.clone()).await?;
        if let Some(headers) = headers {
            client.publish_with_reply_and_headers(subject, inbox, headers, payload.into()).await?;
        } else {
            client.publish_with_reply(subject, inbox, payload.into()).await?;
        }

        let result = match tokio::time::timeout(timeout, subscription.next()).await {
            Ok(Some(msg)) => Ok(msg),
            Ok(None) => Err("Subscription closed without receiving response".into()),
            Err(_) => Err(format!("Request timed out after {:?}", timeout).into()),
        };

        // Clean up subscription to prevent leak
        let _ = subscription.unsubscribe().await;

        result
    }
}
