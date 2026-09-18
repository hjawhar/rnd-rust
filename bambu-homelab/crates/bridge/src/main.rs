mod bambu_mqtt;
mod bambu_types;
mod commands;
mod printer_manager;
mod translate;
mod ftp;

use anyhow::Context;
use diesel::prelude::*;
use diesel_async::pooled_connection::bb8;
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::AsyncPgConnection;
use diesel_async::RunQueryDsl;
use futures_util::StreamExt;
use tracing::info;

use bambu_shared::config::{self, BridgeConfig};
use printer_manager::{PrinterInfo, PrinterManager};

// Minimal schema for reading the printers table.
// Duplicated from the API crate to avoid a shared dependency.
diesel::table! {
    printers (id) {
        id -> Varchar,
        name -> Varchar,
        ip -> Varchar,
        serial -> Varchar,
        access_code -> Varchar,
        model -> Varchar,
    }
}

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = printers)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct DbPrinter {
    id: String,
    name: String,
    ip: String,
    serial: String,
    access_code: String,
    model: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,bambu_bridge=debug".parse().unwrap()),
        )
        .with_target(true)
        .init();

    info!("bambu-bridge starting");

    let cfg: BridgeConfig = config::load("BAMBU_BRIDGE")
        .context("failed to load bridge configuration")?;

    // Connect to NATS.
    let ca_cert = std::path::Path::new(&cfg.nats.ca_cert);
    let nats_client = async_nats::ConnectOptions::new()
        .nkey(cfg.nats.nkey_seed.clone())
        .add_root_certificates(ca_cert.into())
        .require_tls(true)
        .connect(&cfg.nats.url)
        .await
        .context("failed to connect to NATS")?;

    info!("connected to NATS");

    // Connect to Postgres.
    let db_config = AsyncDieselConnectionManager::<AsyncPgConnection>::new(&cfg.database_url);
    let db_pool = bb8::Pool::builder()
        .max_size(5)
        .build(db_config)
        .await
        .context("failed to create database pool")?;

    info!("connected to database");

    // Create printer manager.
    let manager = PrinterManager::new(nats_client.clone());

    // Load existing printers from DB.
    {
        let mut conn = db_pool.get().await.context("failed to get DB connection")?;
        let db_printers: Vec<DbPrinter> = printers::table
            .select(DbPrinter::as_select())
            .load(&mut conn)
            .await
            .context("failed to load printers from DB")?;

        info!(count = db_printers.len(), "loaded printers from database");

        for p in db_printers {
            manager
                .add_printer(PrinterInfo {
                    id: p.id,
                    name: p.name,
                    ip: p.ip,
                    serial: p.serial,
                    access_code: p.access_code,
                    model: p.model,
                })
                .await;
        }
    }

    info!(active = manager.active_count().await, "printer connections started");

    // Subscribe to all NATS subjects
    let mut events_sub = nats_client
        .subscribe("bridge.events")
        .await
        .context("failed to subscribe to bridge.events")?;
    let mut file_list_sub = nats_client
        .subscribe("bridge.files.list")
        .await
        .context("failed to subscribe to bridge.files.list")?;
    let mut file_upload_sub = nats_client
        .subscribe("bridge.files.upload")
        .await
        .context("failed to subscribe to bridge.files.upload")?;

    info!("listening for bridge events and file requests on NATS");

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            msg = events_sub.next() => {
                let Some(msg) = msg else {
                    info!("bridge events stream ended");
                    break;
                };

                #[derive(serde::Deserialize)]
                struct BridgeEvent {
                    event: String,
                    #[serde(default)]
                    printer: Option<PrinterInfo>,
                    #[serde(default)]
                    id: Option<String>,
                }

                match serde_json::from_slice::<BridgeEvent>(&msg.payload) {
                    Ok(event) => match event.event.as_str() {
                        "printer_added" => {
                            if let Some(printer) = event.printer {
                                info!(printer_id = %printer.id, "adding printer from event");
                                manager.add_printer(printer).await;
                            }
                        }
                        "printer_removed" => {
                            if let Some(id) = event.id {
                                info!(printer_id = %id, "removing printer from event");
                                manager.remove_printer(&id).await;
                            }
                        }
                        other => {
                            tracing::debug!(event = other, "unknown bridge event");
                        }
                    },
                    Err(e) => {
                        tracing::warn!(error = %e, "invalid bridge event payload");
                    }
                }
            }
            msg = file_list_sub.next() => {
                if let Some(msg) = msg {
                    let nats_reply = nats_client.clone();
                    let pool = db_pool.clone();
                    tokio::spawn(async move {
                        handle_file_list(nats_reply, pool, msg).await;
                    });
                }
            }
            msg = file_upload_sub.next() => {
                if let Some(msg) = msg {
                    let nats_reply = nats_client.clone();
                    let pool = db_pool.clone();
                    tokio::spawn(async move {
                        handle_file_upload(nats_reply, pool, msg).await;
                    });
                }
            }
            _ = &mut shutdown => {
                info!("shutting down");
                break;
            }
        }
    }

    Ok(())
}

async fn handle_file_list(
    nats: async_nats::Client,
    pool: diesel_async::pooled_connection::bb8::Pool<diesel_async::AsyncPgConnection>,
    msg: async_nats::Message,
) {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;

    let reply = match msg.reply {
        Some(r) => r,
        None => return,
    };

    #[derive(serde::Deserialize)]
    struct ListReq { printer_id: String }

    let req: ListReq = match serde_json::from_slice(&msg.payload) {
        Ok(r) => r,
        Err(e) => {
            let err = serde_json::json!({"error": e.to_string()});
            nats.publish(reply, serde_json::to_vec(&err).unwrap_or_default().into()).await.ok();
            return;
        }
    };

    info!(printer_id = %req.printer_id, "handling file list request");

    let mut conn = match pool.get().await {
        Ok(c) => c,
        Err(e) => {
            let err = serde_json::json!({"error": e.to_string()});
            nats.publish(reply, serde_json::to_vec(&err).unwrap_or_default().into()).await.ok();
            return;
        }
    };

    let printer: Option<DbPrinter> = printers::table
        .find(&req.printer_id)
        .select(DbPrinter::as_select())
        .first(&mut conn)
        .await
        .ok();

    let response = match printer {
        Some(p) => {
            let ip = p.ip.clone();
            let code = p.access_code.clone();
            match tokio::task::spawn_blocking(move || ftp::list_files(&ip, &code)).await {
                Ok(Ok(files)) => serde_json::json!({"files": files}),
                Ok(Err(e)) => serde_json::json!({"error": e.to_string()}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        None => serde_json::json!({"error": "printer not found"}),
    };

    nats.publish(reply, serde_json::to_vec(&response).unwrap_or_default().into()).await.ok();
}

async fn handle_file_upload(
    nats: async_nats::Client,
    pool: diesel_async::pooled_connection::bb8::Pool<diesel_async::AsyncPgConnection>,
    msg: async_nats::Message,
) {
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    use base64::Engine;

    let reply = match msg.reply {
        Some(r) => r,
        None => return,
    };

    #[derive(serde::Deserialize)]
    struct UploadReq { printer_id: String, filename: String, data: String }

    let req: UploadReq = match serde_json::from_slice(&msg.payload) {
        Ok(r) => r,
        Err(e) => {
            let err = serde_json::json!({"error": e.to_string()});
            nats.publish(reply, serde_json::to_vec(&err).unwrap_or_default().into()).await.ok();
            return;
        }
    };

    info!(printer_id = %req.printer_id, filename = %req.filename, "handling file upload request");

    let mut conn = match pool.get().await {
        Ok(c) => c,
        Err(e) => {
            let err = serde_json::json!({"error": e.to_string()});
            nats.publish(reply, serde_json::to_vec(&err).unwrap_or_default().into()).await.ok();
            return;
        }
    };

    let printer: Option<DbPrinter> = printers::table
        .find(&req.printer_id)
        .select(DbPrinter::as_select())
        .first(&mut conn)
        .await
        .ok();

    let response = match printer {
        Some(p) => {
            let data = match base64::engine::general_purpose::STANDARD.decode(&req.data) {
                Ok(d) => d,
                Err(e) => {
                    let err = serde_json::json!({"error": format!("base64 decode: {e}")});
                    nats.publish(reply, serde_json::to_vec(&err).unwrap_or_default().into()).await.ok();
                    return;
                }
            };
            let ip = p.ip.clone();
            let code = p.access_code.clone();
            let filename = req.filename.clone();
            match tokio::task::spawn_blocking(move || ftp::upload_file(&ip, &code, &filename, &data)).await {
                Ok(Ok(())) => serde_json::json!({"ok": true}),
                Ok(Err(e)) => serde_json::json!({"error": e.to_string()}),
                Err(e) => serde_json::json!({"error": e.to_string()}),
            }
        }
        None => serde_json::json!({"error": "printer not found"}),
    };

    nats.publish(reply, serde_json::to_vec(&response).unwrap_or_default().into()).await.ok();
}