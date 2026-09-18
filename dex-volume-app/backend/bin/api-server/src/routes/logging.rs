use std::{net::SocketAddr, time::Instant};

use axum::{
    extract::{ConnectInfo, Request},
    middleware::Next,
    response::Response,
};

/// Lightweight request/response logging — header-only, no body buffering.
pub async fn detailed_logging_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let start = Instant::now();
    let method = req.method().clone();
    let uri = req.uri().clone();

    let ip_address = addr.ip().to_string();
    let real_ip = req
        .headers()
        .get("x-real-ip")
        .or_else(|| req.headers().get("x-forwarded-for"))
        .and_then(|header| header.to_str().ok())
        .and_then(|header_str| header_str.split(',').next().map(|s| s.trim()))
        .unwrap_or(&ip_address)
        .to_string();

    let response = next.run(req).await;
    let duration = start.elapsed();
    let status = response.status();

    tracing::info!(
        "[{}] {} {} - {} ({:?})",
        real_ip,
        method,
        uri,
        status,
        duration
    );

    response
}
