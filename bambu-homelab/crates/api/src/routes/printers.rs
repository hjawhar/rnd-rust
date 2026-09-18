use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use tracing::info;

use crate::auth::middleware::AuthUser;
use crate::db::models::{NewPrinter, NewPrinterAssignment, Printer, PrinterAssignment, User};
use crate::db::schema::{printers, printer_assignments, users};
use crate::models::{
    AddPrinterRequest, AssignPrinterRequest, AssignmentInfo, CommandRequest, PrinterConfig,
    PrinterWithStatus, WsServerMessage,
};
use crate::state::AppState;

/// Returns true if user has access to the given printer.
/// Admins always have access. Users must be assigned.
async fn user_has_printer_access(
    conn: &mut diesel_async::AsyncPgConnection,
    user_id: &uuid::Uuid,
    role: &str,
    printer_id: &str,
) -> Result<bool, StatusCode> {
    if role == "admin" {
        return Ok(true);
    }
    let count: i64 = printer_assignments::table
        .filter(printer_assignments::user_id.eq(user_id))
        .filter(printer_assignments::printer_id.eq(printer_id))
        .count()
        .get_result(conn)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(count > 0)
}

pub async fn list_printers(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<PrinterWithStatus>>, StatusCode> {
    let mut conn = state
        .db
        .get()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let db_printers: Vec<Printer> = if auth.role == "admin" {
        printers::table
            .select(Printer::as_select())
            .load(&mut conn)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    } else {
        printers::table
            .inner_join(
                printer_assignments::table.on(
                    printer_assignments::printer_id.eq(printers::id)
                        .and(printer_assignments::user_id.eq(auth.user_id))
                ),
            )
            .select(Printer::as_select())
            .load(&mut conn)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };

    let online = state.online_status.read().await;
    let cache = state.telemetry_cache.read().await;

    let result: Vec<PrinterWithStatus> = db_printers
        .into_iter()
        .map(|p| {
            let is_online = online.get(&p.id).map(|t| t.elapsed().as_secs() < 60).unwrap_or(false);
            let status = cache.get(&p.id).cloned();
            PrinterWithStatus {
                config: PrinterConfig {
                    id: p.id,
                    name: p.name,
                    ip: p.ip,
                    serial: p.serial,
                    access_code: p.access_code,
                    model: p.model,
                },
                online: is_online,
                status,
            }
        })
        .collect();

    Ok(Json(result))
}

pub async fn add_printer(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<AddPrinterRequest>,
) -> Result<(StatusCode, Json<PrinterConfig>), StatusCode> {
    let mut conn = state
        .db
        .get()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if auth.role != "admin" { return Err(StatusCode::FORBIDDEN); }

    let new_printer = NewPrinter {
        id: &req.serial,
        name: &req.name,
        ip: &req.ip,
        serial: &req.serial,
        access_code: &req.access_code,
        model: &req.model,
        owner_id: &auth.user_id,
    };

    diesel::insert_into(printers::table)
        .values(&new_printer)
        .execute(&mut conn)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;

    info!(id = %req.serial, name = %req.name, "registered printer");

    let config = PrinterConfig {
        id: req.serial.clone(),
        name: req.name,
        ip: req.ip,
        serial: req.serial,
        access_code: req.access_code,
        model: req.model,
    };

    // Notify bridge to connect to the new printer.
    let event = serde_json::json!({
        "event": "printer_added",
        "printer": {
            "id": &config.id,
            "name": &config.name,
            "ip": &config.ip,
            "serial": &config.serial,
            "access_code": &config.access_code,
            "model": &config.model,
        }
    });
    let _ = state.nats.publish("bridge.events", serde_json::to_vec(&event).unwrap_or_default().into()).await;

    Ok((StatusCode::CREATED, Json(config)))
}

pub async fn get_printer(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<PrinterWithStatus>, StatusCode> {
    let mut conn = state
        .db
        .get()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !user_has_printer_access(&mut conn, &auth.user_id, &auth.role, &id).await? {
        return Err(StatusCode::FORBIDDEN);
    }

    let p: Printer = printers::table
        .find(&id)
        .select(Printer::as_select())
        .first(&mut conn)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let online = state
        .online_status
        .read()
        .await
        .get(&id)
        .map(|t| t.elapsed().as_secs() < 60)
        .unwrap_or(false);
    let status = state.telemetry_cache.read().await.get(&id).cloned();

    Ok(Json(PrinterWithStatus {
        config: PrinterConfig {
            id: p.id,
            name: p.name,
            ip: p.ip,
            serial: p.serial,
            access_code: p.access_code,
            model: p.model,
        },
        online,
        status,
    }))
}

pub async fn remove_printer(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let mut conn = state
        .db
        .get()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if auth.role != "admin" { return Err(StatusCode::FORBIDDEN); }

    let count = diesel::delete(printers::table.find(&id))
        .execute(&mut conn)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if count > 0 {
        info!(id = %id, "removed printer");

        // Notify bridge to disconnect from the printer.
        let event = serde_json::json!({
            "event": "printer_removed",
            "id": &id
        });
        let _ = state.nats.publish("bridge.events", serde_json::to_vec(&event).unwrap_or_default().into()).await;

        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub async fn send_command(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(cmd): Json<CommandRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut conn = state
        .db
        .get()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if auth.role != "admin" { return Err((StatusCode::FORBIDDEN, "admin only".into())); }

    let _: Printer = printers::table
        .find(&id)
        .select(Printer::as_select())
        .first(&mut conn)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "printer not found".to_string()))?;

    let subject = format!("printers.{id}.cmd");
    let payload = serde_json::to_vec(&cmd).unwrap_or_default();

    state
        .nats
        .publish(subject, payload.into())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(printer_id = %id, command = %cmd.command, "command sent");
    Ok(StatusCode::ACCEPTED)
}


pub async fn get_print_history(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::db::models::PrintJob>>, StatusCode> {
    let mut conn = state
        .db
        .get()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !user_has_printer_access(&mut conn, &auth.user_id, &auth.role, &id).await? {
        return Err(StatusCode::FORBIDDEN);
    }

    use crate::db::schema::print_jobs;
    use crate::db::models::PrintJob;

    let jobs: Vec<PrintJob> = print_jobs::table
        .filter(print_jobs::printer_id.eq(&id))
        .order(print_jobs::started_at.desc())
        .limit(50)
        .select(PrintJob::as_select())
        .load(&mut conn)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(jobs))
}

#[derive(Debug, serde::Deserialize)]
pub struct PrintStartRequest {
    pub filename: String,
    #[serde(default = "default_plate")]
    pub plate: u64,
}

fn default_plate() -> u64 { 1 }

pub async fn start_print(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<PrintStartRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut conn = state.db.get().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if auth.role != "admin" { return Err((StatusCode::FORBIDDEN, "admin only".into())); }

    let _: Printer = printers::table
        .find(&id)
        .select(Printer::as_select())
        .first(&mut conn)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "printer not found".to_string()))?;

    let cmd = serde_json::json!({
        "command": "print_start",
        "params": {
            "filename": req.filename,
            "plate": req.plate
        }
    });

    let subject = format!("printers.{id}.cmd");
    state.nats
        .publish(subject, serde_json::to_vec(&cmd).unwrap_or_default().into())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!(printer_id = %id, filename = %req.filename, "print started");
    Ok(StatusCode::ACCEPTED)
}

#[derive(diesel::QueryableByName, Debug)]
struct StatsRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    total: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    completed: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    failed: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    total_seconds: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct PrinterStats {
    pub total_prints: i64,
    pub completed: i64,
    pub failed: i64,
    pub total_print_time_seconds: i64,
    pub avg_print_time_seconds: i64,
}

pub async fn get_printer_stats(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<PrinterStats>, StatusCode> {
    let mut conn = state.db.get().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !user_has_printer_access(&mut conn, &auth.user_id, &auth.role, &id).await? {
        return Err(StatusCode::FORBIDDEN);
    }

    let row = diesel::sql_query(
        "SELECT
            COUNT(*) as total,
            COUNT(*) FILTER (WHERE status = 'completed') as completed,
            COUNT(*) FILTER (WHERE status = 'failed') as failed,
            COALESCE(SUM(duration_seconds) FILTER (WHERE duration_seconds IS NOT NULL), 0) as total_seconds
         FROM print_jobs WHERE printer_id = $1"
    )
    .bind::<diesel::sql_types::VarChar, _>(&id)
    .get_result::<StatsRow>(&mut conn)
    .await
    .unwrap_or(StatsRow { total: 0, completed: 0, failed: 0, total_seconds: 0 });

    let avg = if row.completed > 0 { row.total_seconds / row.completed } else { 0 };

    Ok(Json(PrinterStats {
        total_prints: row.total,
        completed: row.completed,
        failed: row.failed,
        total_print_time_seconds: row.total_seconds,
        avg_print_time_seconds: avg,
    }))
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct FileEntry {
    pub name: String,
}

pub async fn list_files(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<FileEntry>>, (StatusCode, String)> {
    {
        let mut conn = state.db.get().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if !user_has_printer_access(&mut conn, &auth.user_id, &auth.role, &id).await
            .map_err(|e| (e, "access check failed".to_string()))?
        {
            return Err((StatusCode::FORBIDDEN, "not assigned to this printer".into()));
        }
    }
    let req = serde_json::json!({"printer_id": id});
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        state.nats.request("bridge.files.list", serde_json::to_vec(&req).unwrap_or_default().into())
    )
    .await
    .map_err(|_| (StatusCode::GATEWAY_TIMEOUT, "file listing timed out".to_string()))?
    .map_err(|e| (StatusCode::BAD_GATEWAY, format!("NATS request failed: {e}")))?;

    let result: serde_json::Value = serde_json::from_slice(&response.payload)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(err) = result.get("error").and_then(|v| v.as_str()) {
        return Err((StatusCode::BAD_GATEWAY, err.to_string()));
    }

    let files: Vec<FileEntry> = serde_json::from_value(
        result.get("files").cloned().unwrap_or(serde_json::json!([]))
    ).unwrap_or_default();

    Ok(Json(files))
}

pub async fn upload_file(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    mut multipart: axum::extract::Multipart,
) -> Result<StatusCode, (StatusCode, String)> {
    if auth.role != "admin" { return Err((StatusCode::FORBIDDEN, "admin only".into())); }
    let mut filename = String::new();
    let mut file_data = Vec::new();

    while let Some(field) = multipart.next_field().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))? {
        if field.name() == Some("file") {
            filename = field.file_name().unwrap_or("upload.3mf").to_string();
            file_data = field.bytes().await.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?.to_vec();
        }
    }

    if file_data.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "no file provided".to_string()));
    }

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&file_data);

    let req = serde_json::json!({
        "printer_id": id,
        "filename": filename,
        "data": encoded,
    });

    let response = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        state.nats.request("bridge.files.upload", serde_json::to_vec(&req).unwrap_or_default().into())
    )
    .await
    .map_err(|_| (StatusCode::GATEWAY_TIMEOUT, "file upload timed out".to_string()))?
    .map_err(|e| (StatusCode::BAD_GATEWAY, format!("NATS request failed: {e}")))?;

    let result: serde_json::Value = serde_json::from_slice(&response.payload)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(err) = result.get("error").and_then(|v| v.as_str()) {
        return Err((StatusCode::BAD_GATEWAY, err.to_string()));
    }

    Ok(StatusCode::CREATED)
}

pub async fn get_queue(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::db::models::QueueItem>>, StatusCode> {
    use crate::db::schema::print_queue;
    let mut conn = state.db.get().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !user_has_printer_access(&mut conn, &auth.user_id, &auth.role, &id).await? {
        return Err(StatusCode::FORBIDDEN);
    }
    let items = print_queue::table
        .filter(print_queue::printer_id.eq(&id))
        .filter(print_queue::status.eq("queued"))
        .order(print_queue::position.asc())
        .select(crate::db::models::QueueItem::as_select())
        .load(&mut conn).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(items))
}

#[derive(Debug, serde::Deserialize)]
pub struct QueueAddRequest {
    pub file_name: String,
    #[serde(default = "default_plate_i32")]
    pub plate: i32,
}
fn default_plate_i32() -> i32 { 1 }

pub async fn add_to_queue(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<QueueAddRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    use crate::db::schema::print_queue;
    let mut conn = state.db.get().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if auth.role != "admin" { return Err((StatusCode::FORBIDDEN, "admin only".into())); }
    let max_pos: Option<i32> = print_queue::table
        .filter(print_queue::printer_id.eq(&id))
        .select(diesel::dsl::max(print_queue::position))
        .first(&mut conn).await.ok().flatten();
    let item = crate::db::models::NewQueueItem {
        printer_id: &id, file_name: &req.file_name, plate_number: req.plate, position: max_pos.unwrap_or(-1) + 1,
    };
    diesel::insert_into(print_queue::table).values(&item).execute(&mut conn).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::CREATED)
}

pub async fn remove_from_queue(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((_, queue_id)): Path<(String, String)>,
) -> Result<StatusCode, StatusCode> {
    use crate::db::schema::print_queue;
    let mut conn = state.db.get().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if auth.role != "admin" { return Err(StatusCode::FORBIDDEN); }
    let uid = uuid::Uuid::parse_str(&queue_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let n = diesel::delete(print_queue::table.find(uid)).execute(&mut conn).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if n > 0 { Ok(StatusCode::NO_CONTENT) } else { Err(StatusCode::NOT_FOUND) }
}

#[derive(Debug, serde::Serialize)]
pub struct FilamentInfo {
    pub tray_id: String,
    pub filament_type: String,
    pub color: String,
    pub remain_pct: u32,
    pub sub_brand: String,
}

pub async fn get_filament_inventory(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<FilamentInfo>>, StatusCode> {
    let allowed_printers: Option<Vec<String>> = if auth.role != "admin" {
        let mut conn = state.db.get().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let ids: Vec<String> = printer_assignments::table
            .filter(printer_assignments::user_id.eq(auth.user_id))
            .select(printer_assignments::printer_id)
            .load(&mut conn)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Some(ids)
    } else {
        None
    };

    let cache = state.telemetry_cache.read().await;
    let mut inventory = Vec::new();

    for (printer_id, telemetry) in cache.iter() {
        if let Some(ref allowed) = allowed_printers {
            if !allowed.contains(printer_id) {
                continue;
            }
        }
        if let Some(ams) = telemetry.get("ams") {
            if let Some(units) = ams.get("units").and_then(|u| u.as_array()) {
                for unit in units {
                    if let Some(trays) = unit.get("trays").and_then(|t| t.as_array()) {
                        for tray in trays {
                            let filament_type = tray.get("filament_type").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            if filament_type.is_empty() { continue; }
                            inventory.push(FilamentInfo {
                                tray_id: format!("{}:{}",
                                    unit.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                                    tray.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                                ),
                                filament_type,
                                color: tray.get("color").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                remain_pct: tray.get("remain_pct").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                                sub_brand: tray.get("sub_brand").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(Json(inventory))
}

pub async fn start_timelapse(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut conn = state.db.get().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if auth.role != "admin" { return Err((StatusCode::FORBIDDEN, "admin only".into())); }
    let printer: Printer = printers::table.find(&id).select(Printer::as_select()).first(&mut conn).await
        .map_err(|_| (StatusCode::NOT_FOUND, "printer not found".to_string()))?;

    // Build authenticated RTSP URL from printer credentials
    let rtsp_url = format!("rtsps://bblp:{}@{}:322/streaming/live/1", printer.access_code, printer.ip);

    // Write control files for the timelapse service
    let dir = format!(".local/timelapses/{id}");
    tokio::fs::create_dir_all(&dir).await.ok();
    tokio::fs::write(format!("{dir}/.rtsp_url"), &rtsp_url).await.ok();
    tokio::fs::write(format!("{dir}/.active"), "").await.ok();

    // Also ensure the video-relay has a stream config for this printer
    let streams_dir = format!(".local/streams/{id}");
    tokio::fs::create_dir_all(&streams_dir).await.ok();
    let stream_conf = format!(".local/streams/{id}.conf");
    if !tokio::fs::try_exists(&stream_conf).await.unwrap_or(false) {
        tokio::fs::write(&stream_conf, format!("RTSP_URL={rtsp_url}")).await.ok();
    }

    info!(printer_id = %id, "timelapse started");
    Ok(StatusCode::OK)
}

pub async fn stop_timelapse(
    State(_state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    if auth.role != "admin" { return Err(StatusCode::FORBIDDEN); }
    let dir = format!(".local/timelapses/{id}");
    // Remove active flag, signal stitch
    tokio::fs::remove_file(format!("{dir}/.active")).await.ok();
    tokio::fs::write(format!("{dir}/.stitch"), "").await.ok();
    info!(printer_id = %id, "timelapse stopped, stitching");
    Ok(StatusCode::OK)
}

/// Ensure the video-relay stream is configured for a printer.
/// Creates the .conf file if it doesn't exist. The relay picks it up automatically.
pub async fn start_stream(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut conn = state.db.get().await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if auth.role != "admin" { return Err((StatusCode::FORBIDDEN, "admin only".into())); }
    let printer: Printer = printers::table.find(&id).select(Printer::as_select()).first(&mut conn).await
        .map_err(|_| (StatusCode::NOT_FOUND, "printer not found".to_string()))?;

    let rtsp_url = format!("rtsps://bblp:{}@{}:322/streaming/live/1", printer.access_code, printer.ip);
    let streams_dir = format!(".local/streams/{id}");
    tokio::fs::create_dir_all(&streams_dir).await.ok();
    tokio::fs::write(format!(".local/streams/{id}.conf"), format!("RTSP_URL={rtsp_url}")).await.ok();

    info!(printer_id = %id, "camera stream config created");
    Ok(StatusCode::OK)
}

pub async fn list_assignments(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<AssignmentInfo>>, (StatusCode, String)> {
    if auth.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "admin only".into()));
    }
    let mut conn = state.db.get().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let assignments: Vec<(PrinterAssignment, User)> = printer_assignments::table
        .filter(printer_assignments::printer_id.eq(&id))
        .inner_join(users::table.on(users::id.eq(printer_assignments::user_id)))
        .select((PrinterAssignment::as_select(), User::as_select()))
        .load(&mut conn)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let result: Vec<AssignmentInfo> = assignments
        .into_iter()
        .map(|(a, u)| AssignmentInfo {
            user_id: a.user_id.to_string(),
            username: u.username,
            assigned_at: a.created_at.to_string(),
        })
        .collect();

    Ok(Json(result))
}

pub async fn assign_printer(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<AssignPrinterRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if auth.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "admin only".into()));
    }
    let mut conn = state.db.get().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user_id = uuid::Uuid::parse_str(&req.user_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid user_id".into()))?;

    // Verify printer exists
    let printer: Printer = printers::table.find(&id).select(Printer::as_select()).first(&mut conn).await
        .map_err(|_| (StatusCode::NOT_FOUND, "printer not found".into()))?;

    // Insert assignment
    let new_assignment = NewPrinterAssignment {
        user_id: &user_id,
        printer_id: &id,
    };
    diesel::insert_into(printer_assignments::table)
        .values(&new_assignment)
        .execute(&mut conn)
        .await
        .map_err(|_| (StatusCode::CONFLICT, "already assigned".into()))?;

    info!(printer_id = %id, user_id = %user_id, "user assigned to printer");

    // Broadcast assignment event
    let _ = state.ws_broadcast.send(WsServerMessage::AssignmentAdded {
        printer_id: id,
        printer_name: printer.name,
        printer_model: printer.model,
        target_user_id: user_id,
    });

    Ok(StatusCode::CREATED)
}

pub async fn unassign_printer(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, user_id_str)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    if auth.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "admin only".into()));
    }
    let mut conn = state.db.get().await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user_id = uuid::Uuid::parse_str(&user_id_str)
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid user_id".into()))?;

    let count = diesel::delete(
        printer_assignments::table
            .filter(printer_assignments::printer_id.eq(&id))
            .filter(printer_assignments::user_id.eq(user_id)),
    )
    .execute(&mut conn)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if count == 0 {
        return Err((StatusCode::NOT_FOUND, "assignment not found".into()));
    }

    info!(printer_id = %id, user_id = %user_id, "user unassigned from printer");

    // Broadcast removal event (triggers real-time kick)
    let _ = state.ws_broadcast.send(WsServerMessage::AssignmentRemoved {
        printer_id: id,
        target_user_id: user_id,
    });

    Ok(StatusCode::NO_CONTENT)
}