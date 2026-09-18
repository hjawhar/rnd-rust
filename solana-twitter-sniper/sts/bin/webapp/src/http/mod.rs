use std::{net::SocketAddr, sync::Arc};

use auth::{login_user, login_user_nonce};
use axum::{
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Extension, Json, Router,
};

use blockchain::{get_block_leaders_req, get_pools_req, get_servers_req};
use middleware::{guard, guard_admin};
use serde_json::json;
use users::{add_user_req, get_users_req, whitelist_user_req};
use wallet::{
    create_nonce_account_req, delete_wallets_req, generate_wallets_req, get_wallets_req,
    import_wallets_req, view_wallets_req,
};

use crate::models::state::AppState;

pub mod auth;
pub mod blockchain;
pub mod buy;
pub mod middleware;
pub mod tasks;
pub mod users;
pub mod wallet;
pub mod ws;

use buy::buy_req;
use tasks::{
    add_task_req, delete_task_req, get_task_req, get_tasks_req, sync_tasks_req, update_task_req,
};
use tower_http::{
    cors::CorsLayer,
    trace::{DefaultMakeSpan, TraceLayer},
};
use ws::ws_handler;

pub async fn root() -> impl IntoResponse {
    let body = Json(json!({
        "uptime": 69
    }));
    (StatusCode::CREATED, body).into_response()
}

pub async fn handler_404() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "No content available.")
}

pub async fn start_app_router(state: Arc<AppState>) {
    let app = Router::new()
        .route("/users", get(get_users_req))
        .route("/users", post(add_user_req))
        .route("/users/{id}/whitelist", post(whitelist_user_req))
        .route("/tasks/sync", post(sync_tasks_req))
        .route_layer(axum::middleware::from_fn(guard_admin))
        .route("/", get(root))
        .route("/buy", post(buy_req))
        .route("/wallets/import", post(import_wallets_req))
        .route("/wallets/export", post(view_wallets_req))
        .route("/wallets/generate", post(generate_wallets_req))
        .route("/wallets/delete", post(delete_wallets_req))
        .route(
            "/wallets/{id}/create_nonce_account",
            post(create_nonce_account_req),
        )
        .route("/tasks", post(add_task_req))
        .route("/servers", get(get_servers_req))
        .route("/block_leaders", get(get_block_leaders_req))
        .route("/pools", get(get_pools_req))
        .route("/wallets", get(get_wallets_req))
        .route("/tasks", get(get_tasks_req))
        .route("/tasks/{id}", get(get_task_req))
        .route("/tasks/{id}", put(update_task_req))
        .route("/tasks/{id}", delete(delete_task_req))
        .route_layer(axum::middleware::from_fn(guard))
        .route("/ws", get(ws_handler))
        .route("/oauth", post(login_user))
        .route("/oauth/nonce", post(login_user_nonce))
        .layer(Extension(state.clone()))
        .layer(CorsLayer::permissive())
        // logging so we can see whats going on
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .fallback(handler_404)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}
