pub mod printers;
pub mod ws;

use axum::{
    routing::{delete, get, post},
    Router,
};

use crate::auth;
use crate::state::AppState;

pub fn build_router() -> Router<AppState> {
    Router::new()
        // Public routes
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/auth/login", post(auth::routes::login))
        // Protected routes (AuthUser extractor validates JWT)
        .route("/api/auth/password", post(auth::routes::change_password))
        .route("/api/printers", get(printers::list_printers))
        .route("/api/printers", post(printers::add_printer))
        .route("/api/printers/{id}", get(printers::get_printer))
        .route("/api/printers/{id}", delete(printers::remove_printer))
        .route("/api/printers/{id}/command", post(printers::send_command))
        .route("/api/printers/{id}/history", get(printers::get_print_history))
        .route("/api/printers/{id}/print", post(printers::start_print))
        .route("/api/printers/{id}/stats", get(printers::get_printer_stats))
        .route("/api/printers/{id}/files", get(printers::list_files))
        .route("/api/printers/{id}/upload", post(printers::upload_file))
        .route("/api/printers/{id}/queue", get(printers::get_queue).post(printers::add_to_queue))
        .route("/api/printers/{id}/queue/{queue_id}", delete(printers::remove_from_queue))
        .route("/api/users", get(auth::routes::list_users).post(auth::routes::create_user))
        .route("/api/users/{id}", delete(auth::routes::delete_user))
        .route("/api/filament", get(printers::get_filament_inventory))
        .route("/api/printers/{id}/timelapse/start", post(printers::start_timelapse))
        .route("/api/printers/{id}/timelapse/stop", post(printers::stop_timelapse))
        .route("/api/printers/{id}/stream/start", post(printers::start_stream))
        .route("/api/printers/{id}/assignments", get(printers::list_assignments).post(printers::assign_printer))
        .route("/api/printers/{id}/assignments/{user_id}", delete(printers::unassign_printer))
        .route("/api/ws", get(ws::ws_handler))
}
