use std::{net::SocketAddr, sync::Arc};

use axum::{http::StatusCode, response::IntoResponse, routing::get, Json, Router};

use serde_json::json;
use tower_http::{
    cors::CorsLayer,
    trace::{DefaultMakeSpan, TraceLayer},
};
use ws::ws_handler;

use crate::models::state::AppState;
pub mod middleware;
pub mod ws;

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
        .route("/ws", get(ws_handler))
        .layer(CorsLayer::permissive())
        // logging so we can see whats going on
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .fallback(handler_404)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}
