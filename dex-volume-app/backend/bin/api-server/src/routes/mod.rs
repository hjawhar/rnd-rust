use std::{net::SocketAddr, sync::Arc};
use tokio_util::sync::CancellationToken;

use auth::{login_user, login_user_nonce, logout_handler, refresh_token_handler};
use axum::{
    Extension, Json, Router,
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
};
pub mod middleware;

use middleware::guard;
use serde_json::json;
use subscription::{
    create_subscription_req, update_subscription_req, delete_subscription_req,
    get_all_subscriptions_req, get_overdue_subscriptions_req,
    create_payment_req, get_payments_req, delete_payment_req,
    get_project_subscription_req, get_my_subscriptions_req,
};
use user::{add_user_req, get_audit_logs_req, get_users_req, grant_project_access_req, revoke_project_access_req, get_project_access_req, whitelist_user_req};

use crate::{
    models::state::AppState,
    routes::{
        logging::detailed_logging_middleware,
        middleware::guard_admin,
        projects::{
            collect_sol_req, collect_tokens_req, disperse_sol_req,
            disperse_tokens_req, get_project_financials_req,
            get_project_statistics_req, start_task_req,
            stop_task_req,
        },
        token::get_token_info_req,
        wallet::{delete_wallets_req, generate_wallets_req, import_wallets_req, view_wallets_req},
    },
};

pub mod auth;
pub mod health;
pub mod logging;
pub mod projects;
pub mod subscription;
pub mod token;
pub mod user;
pub mod wallet;
pub mod helpers;
pub mod ws;
pub mod workers;

use projects::{
    add_project_req, delete_project_req, get_all_projects_req, get_project_req,
    get_project_transactions_req, get_project_wallets_req, get_projects_req, update_project_req,
    lock_project_req, unlock_project_req,
};
use tower_http::{
    cors::CorsLayer,
    timeout::TimeoutLayer,
    trace::{DefaultMakeSpan, TraceLayer},
};
use ws::ws_handler;
use tower::limit::ConcurrencyLimitLayer;

pub async fn root() -> impl IntoResponse {
    let body = Json(json!({
        "uptime": 69
    }));
    (StatusCode::CREATED, body).into_response()
}

/// Returns true if the network string refers to an EVM chain.
pub fn is_evm_network(network: &str) -> bool {
    matches!(network, "ethereum" | "base" | "arbitrum" | "bsc" | "avalanche")
}

/// Middleware that generates a trace-id for each request and sets the
/// `vm_nats::TRACE_ID` task-local. All downstream NATS calls
/// automatically propagate the trace-id via NATS headers.
async fn trace_id_middleware(request: Request, next: Next) -> Response {
    let trace_id = uuid::Uuid::new_v4().to_string();
    vm_nats::TRACE_ID
        .scope(trace_id, next.run(request))
        .await
}

pub async fn handler_404() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, "No content available.")
}

pub async fn start_app_router(state: Arc<AppState>, shutdown: CancellationToken, port: u16, max_concurrent_requests: usize) {
    let app = Router::new()
        .route("/users", get(get_users_req))
        .route("/users", post(add_user_req))
        .route("/users/{id}/whitelist", post(whitelist_user_req))
        .route("/users/{user_id}/projects/{project_id}/access", post(grant_project_access_req))
        .route("/users/{user_id}/projects/{project_id}/access", delete(revoke_project_access_req))
        .route("/projects/{project_id}/access", get(get_project_access_req))
        .route("/projects/all", get(get_all_projects_req))
        .route("/admin/projects/{id}/subscription", post(create_subscription_req))
        .route("/admin/projects/{id}/subscription", patch(update_subscription_req))
        .route("/admin/projects/{id}/subscription", delete(delete_subscription_req))
        .route("/admin/subscriptions", get(get_all_subscriptions_req))
        .route("/admin/subscriptions/overdue", get(get_overdue_subscriptions_req))
        .route("/admin/subscriptions/{id}/payments", post(create_payment_req))
        .route("/admin/subscriptions/{id}/payments", get(get_payments_req))
        .route("/admin/subscriptions/{subscription_id}/payments/{payment_id}", delete(delete_payment_req))
        .route("/admin/projects/{id}/lock", post(lock_project_req))
        .route("/admin/projects/{id}/lock", delete(unlock_project_req))
        .route("/audit", get(get_audit_logs_req))
        .route("/admin/workers", get(workers::get_workers_req))
        .route_layer(axum::middleware::from_fn(guard_admin))
        .route("/", get(root))
        .route("/projects", post(add_project_req))
        .route("/projects", get(get_projects_req))
        .route("/projects/{id}", get(get_project_req))
        .route("/projects/{id}/wallets", get(get_project_wallets_req))
        .route("/projects/{id}/financials", get(get_project_financials_req))
        .route("/projects/{id}/collect/tokens", post(collect_tokens_req))
        .route("/projects/{id}/collect/sol", post(collect_sol_req))
        .route("/projects/{id}/disperse/tokens", post(disperse_tokens_req))
        .route("/projects/{id}/disperse/sol", post(disperse_sol_req))
        .route("/projects/{id}/start", post(start_task_req))
        .route("/projects/{id}/stop", post(stop_task_req))
        .route(
            "/projects/{id}/transactions",
            get(get_project_transactions_req),
        )
        .route("/projects/{id}/statistics", get(get_project_statistics_req))
        .route("/projects/{id}/subscription", get(get_project_subscription_req))
        .route("/subscriptions", get(get_my_subscriptions_req))
        .route("/projects/{id}", put(update_project_req))
        .route("/projects/{id}", delete(delete_project_req))
        .route("/projects/{id}/wallets/import", post(import_wallets_req))
        .route("/projects/{id}/wallets/export", post(view_wallets_req))
        .route(
            "/projects/{id}/wallets/generate",
            post(generate_wallets_req),
        )
        .route("/projects/{id}/wallets/delete", post(delete_wallets_req))
        .route("/tokens/{token}", get(get_token_info_req))
        .route("/system/status", get(health::system_status))
        .route("/oauth/logout", post(logout_handler))
        .layer(axum::middleware::from_fn(detailed_logging_middleware))
        .route_layer(axum::middleware::from_fn(guard))
        .route("/health", get(health::health_check))
        .route("/ping", get(health::ping))
        .route("/ws", get(ws_handler))
        .route("/oauth", post(login_user))
        .route("/oauth/nonce", post(login_user_nonce))
        .route("/oauth/refresh", post(refresh_token_handler))
        .layer(Extension(state.clone()))
        .layer(axum::middleware::from_fn(trace_id_middleware))
        .layer({
            use tower_http::cors::AllowOrigin;
            let app_env = std::env::var("APP_ENV").unwrap_or_default();
            if app_env == "production" {
                CorsLayer::new()
                    .allow_origin(AllowOrigin::list([
                        // TODO: replace with your production domain
                        "http://localhost:4200".parse().unwrap(),
                    ]))
                    .allow_methods([axum::http::Method::GET, axum::http::Method::POST, axum::http::Method::PUT, axum::http::Method::PATCH, axum::http::Method::DELETE, axum::http::Method::OPTIONS])
                    .allow_headers([
                        axum::http::header::CONTENT_TYPE,
                        axum::http::header::AUTHORIZATION,
                        axum::http::header::ACCEPT,
                        axum::http::header::ORIGIN,
                    ])
                    .allow_credentials(true)
            } else {
                CorsLayer::permissive()
            }
        })
        .layer(TimeoutLayer::with_status_code(axum::http::StatusCode::GATEWAY_TIMEOUT, std::time::Duration::from_secs(120)))
        // logging so we can see whats going on
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .layer(ConcurrencyLimitLayer::new(max_concurrent_requests))
        .fallback(handler_404)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .unwrap();
    tracing::info!("listening on {} (max_concurrent={})", listener.local_addr().unwrap(), max_concurrent_requests);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown.cancelled_owned())
    .await
    .unwrap();
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evm_networks_recognized() {
        assert!(is_evm_network("ethereum"));
        assert!(is_evm_network("base"));
        assert!(is_evm_network("arbitrum"));
        assert!(is_evm_network("bsc"));
        assert!(is_evm_network("avalanche"));
    }

    #[test]
    fn solana_is_not_evm() {
        assert!(!is_evm_network("solana"));
    }

    #[test]
    fn unknown_networks_are_not_evm() {
        assert!(!is_evm_network(""));
        assert!(!is_evm_network("polygon"));
        assert!(!is_evm_network("Ethereum")); // case-sensitive
    }
}