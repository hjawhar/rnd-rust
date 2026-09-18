//! Manages concurrent MQTT connections to multiple printers.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use prost::Message;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{error, info};


use crate::bambu_mqtt;
use crate::commands;
use crate::translate::{self, PrinterContext};

/// Info needed to connect to a printer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PrinterInfo {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub serial: String,
    pub access_code: String,
    pub model: String,
}

struct PrinterHandle {
    _task: JoinHandle<()>,
}

/// Manages active printer connections.
pub struct PrinterManager {
    nats: async_nats::Client,
    printers: Arc<RwLock<HashMap<String, PrinterHandle>>>,
}

impl PrinterManager {
    pub fn new(nats: async_nats::Client) -> Self {
        Self {
            nats,
            printers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a printer and start its MQTT connection.
    pub async fn add_printer(&self, printer: PrinterInfo) {
        let id = printer.id.clone();

        // Remove existing connection if any.
        self.remove_printer(&id).await;

        let nats = self.nats.clone();
        let info = printer.clone();

        let task = tokio::spawn(async move {
            loop {
                match run_printer_bridge(&nats, &info).await {
                    Ok(()) => {
                        info!(printer_id = %info.id, "printer bridge ended cleanly");
                        break;
                    }
                    Err(e) => {
                        error!(printer_id = %info.id, error = %e, "printer bridge failed, reconnecting in 10s");
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    }
                }
            }
        });

        self.printers.write().await.insert(
            id.clone(),
            PrinterHandle { _task: task },
        );

        info!(printer_id = %id, "printer connection started");
    }

    /// Remove a printer and stop its MQTT connection.
    pub async fn remove_printer(&self, id: &str) {
        if let Some(handle) = self.printers.write().await.remove(id) {
            handle._task.abort();
            info!(printer_id = %id, "printer connection stopped");
        }
    }

    pub async fn active_count(&self) -> usize {
        self.printers.read().await.len()
    }
}

/// Run the bridge for a single printer. Returns on disconnect or error.
async fn run_printer_bridge(
    nats: &async_nats::Client,
    printer: &PrinterInfo,
) -> anyhow::Result<()> {
    let (mqtt_client, mut reports) = bambu_mqtt::connect(
        &printer.ip,
        8883,
        &printer.serial,
        &printer.access_code,
    )
    .await
    .context("failed to connect to printer MQTT")?;

    info!(printer_id = %printer.id, ip = %printer.ip, "connected to printer");

    // Spawn command handler for this printer.
    commands::spawn_command_handler(
        nats.clone(),
        mqtt_client.clone(),
        &printer.id,
        &printer.serial,
    )
    .await
    .context("failed to start command handler")?;

    let ctx = PrinterContext {
        printer_id: printer.id.clone(),
        serial_number: printer.serial.clone(),
        name: printer.name.clone(),
        model: printer.model.clone(),
    };

    let nats_subject = format!("printers.{}.telemetry", printer.id);
    let heartbeat_subject = format!("printers.{}.heartbeat", printer.id);
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));

    loop {
        tokio::select! {
            report = reports.recv() => {
                let Some(report) = report else { break; };
                if let Some(status) = report.print {
                    if status.command != "push_status" {
                        continue;
                    }

                    let snapshot = translate::to_telemetry(&status, &ctx);

                    let mut buf = Vec::with_capacity(snapshot.encoded_len());
                    snapshot.encode(&mut buf)?;

                    nats.publish(nats_subject.clone(), buf.into())
                        .await
                        .context("failed to publish to NATS")?;
                }
            }
            _ = heartbeat.tick() => {
                nats.publish(heartbeat_subject.clone(), "alive".into()).await.ok();
            }
        }
    }

    anyhow::bail!("MQTT stream ended")
}
