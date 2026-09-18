use anyhow::Context;
use async_nats::jetstream;
use futures_util::StreamExt;
use prost::Message;
use tracing::{error, info, warn};

use bambu_shared::telemetry::TelemetrySnapshot;

const STREAM_NAME: &str = "PRINTER_TELEMETRY";
const CONSUMER_NAME: &str = "gateway-telemetry";

/// Ensure the JetStream stream exists, creating it if necessary.
pub async fn ensure_stream(js: &jetstream::Context) -> anyhow::Result<jetstream::stream::Stream> {
    let stream = js
        .get_or_create_stream(jetstream::stream::Config {
            name: STREAM_NAME.to_string(),
            subjects: vec!["printers.*.telemetry".to_string()],
            retention: jetstream::stream::RetentionPolicy::Limits,
            max_age: std::time::Duration::from_secs(7 * 24 * 3600),
            storage: jetstream::stream::StorageType::File,
            ..Default::default()
        })
        .await
        .context("failed to create/get telemetry stream")?;

    info!(stream = STREAM_NAME, "JetStream stream ready");
    Ok(stream)
}

/// Subscribe to the telemetry stream and process messages indefinitely.
pub async fn subscribe(js: &jetstream::Context) -> anyhow::Result<()> {
    let stream = ensure_stream(js).await?;

    let consumer = stream
        .get_or_create_consumer(
            CONSUMER_NAME,
            jetstream::consumer::pull::Config {
                durable_name: Some(CONSUMER_NAME.to_string()),
                filter_subject: "printers.*.telemetry".to_string(),
                ..Default::default()
            },
        )
        .await
        .context("failed to create/get telemetry consumer")?;

    info!(consumer = CONSUMER_NAME, "telemetry consumer ready, waiting for messages");

    let mut messages = consumer
        .messages()
        .await
        .context("failed to start message stream")?;

    while let Some(msg_result) = messages.next().await {
        let msg = match msg_result {
            Ok(m) => m,
            Err(e) => {
                error!(error = %e, "error receiving message from JetStream");
                continue;
            }
        };

        match TelemetrySnapshot::decode(msg.payload.as_ref()) {
            Ok(snapshot) => {
                let printer_id = snapshot
                    .printer
                    .as_ref()
                    .map(|p| p.printer_id.as_str())
                    .unwrap_or("unknown");
                let state = snapshot.state();

                info!(
                    printer_id,
                    state = ?state,
                    nozzle = snapshot.nozzle_temp.as_ref().map(|t| t.current).unwrap_or(0.0),
                    bed = snapshot.bed_temp.as_ref().map(|t| t.current).unwrap_or(0.0),
                    progress = snapshot.print_progress_pct,
                    "telemetry received"
                );
            }
            Err(e) => {
                warn!(
                    error = %e,
                    subject = %msg.subject,
                    payload_len = msg.payload.len(),
                    "failed to decode telemetry snapshot"
                );
            }
        }

        if let Err(e) = msg.ack().await {
            error!(error = %e, "failed to ack message");
        }
    }

    warn!("telemetry message stream ended unexpectedly");
    Ok(())
}
