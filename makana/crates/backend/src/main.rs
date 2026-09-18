//! makana backend.
//!
//! Picks one transport at startup, fans every `Sample` out to all connected
//! WebSocket clients. Kept deliberately thin — the viewer is a separate
//! concern, firmware is a separate concern, this just brokers.

#![forbid(unsafe_code)]

use std::{path::PathBuf, time::Duration};

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use clap::Parser;
use futures::StreamExt;
use makana_backend::{Cli, TransportKind, recorder::Recorder};
use makana_common::Sample;
use makana_transport::{CarDataSource, Elm327Source, LogSource, MockSource};
use tokio::sync::broadcast;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::{error, info, warn};

#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<Sample>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "makana_backend=info,makana_transport=info,tower_http=info".into()
            }),
        )
        .init();

    let cli = Cli::parse();

    // Broadcast channel decouples the transport task from connected clients.
    // Slow clients get lagged (and logged) rather than blocking ingestion.
    let (tx, _) = broadcast::channel::<Sample>(1024);

    // Construct transport from CLI config.
    let source: Box<dyn CarDataSource> = match cli.transport {
        TransportKind::Mock => Box::new(MockSource::new(Duration::from_millis(cli.mock_tick_ms))),
        TransportKind::Elm327 => {
            let device = cli
                .device
                .as_deref()
                .ok_or_else(|| "--device is required when --transport elm327".to_string())?;
            let mut src = Elm327Source::new(device).with_baud(cli.baud);
            if let Some(pids) = cli.pids.clone() {
                src = src.with_pids(pids);
            }
            Box::new(src)
        }
        TransportKind::Logfile => {
            let device = cli
                .device
                .as_deref()
                .ok_or_else(|| "--device is required when --transport logfile".to_string())?;
            Box::new(
                LogSource::new(PathBuf::from(device))
                    .with_speed(cli.speed)
                    .with_repeat(cli.repeat),
            )
        }
    };
    info!(transport = ?cli.transport, source = source.name(), "starting source");

    // Recorder: warn about rotation flags that won't take effect, then (if
    // --record is set) open the file up front so a bad path fails fast.
    if cli.record.is_none() {
        if cli.record_rotate_mb.is_some() {
            warn!("--record-rotate-mb supplied without --record; ignoring");
        }
        if cli.record_keep.is_some() {
            warn!("--record-keep supplied without --record; ignoring");
        }
    } else if cli.record_rotate_mb.is_none() && cli.record_keep.is_some() {
        warn!("--record-keep supplied without --record-rotate-mb; ignoring");
    }

    if let Some(path) = cli.record.as_ref() {
        let abs = std::path::absolute(path).unwrap_or_else(|_| path.clone());
        let rotate_mb = cli.record_rotate_mb;
        let keep = cli.record_keep.unwrap_or(5) as usize;
        match Recorder::new(path.clone(), rotate_mb, keep).await {
            Ok(mut recorder) => {
                info!(
                    path = %abs.display(),
                    rotate_mb = ?rotate_mb,
                    keep,
                    "recording samples to JSONL"
                );
                let mut rx = tx.subscribe();
                tokio::spawn(async move {
                    loop {
                        match rx.recv().await {
                            Ok(sample) => recorder.write(&sample).await,
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!(dropped = n, "recorder lagging; dropped samples");
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
            }
            Err(e) => {
                // Don't crash the backend: live feed continues unrecorded.
                error!(error = %e, path = %abs.display(), "recorder init failed; continuing without recording");
            }
        }
    }

    // Spawn ingestion: pull samples, publish to broadcast.
    let tx_in = tx.clone();
    tokio::spawn(async move {
        let mut stream = match source.start().await {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "failed to start transport");
                return;
            }
        };
        while let Some(item) = stream.next().await {
            match item {
                Ok(sample) => {
                    // No subscribers yet is fine — broadcast just drops.
                    let _ = tx_in.send(sample);
                }
                Err(e) => warn!(error = %e, "transport error"),
            }
        }
        warn!("transport stream ended");
    });

    let state = AppState { tx };
    let app = Router::new()
        .route("/", get(index))
        .route("/health", get(|| async { "ok" }))
        .route("/ws", get(ws_handler))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    info!(bind = %cli.bind, "listening");
    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

const INDEX_HTML: &str = include_str!("../web/index.html");

async fn index() -> axum::response::Response {
    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(axum::body::Body::from(INDEX_HTML))
        .expect("static body")
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, state)))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.tx.subscribe();
    info!("client connected");

    loop {
        tokio::select! {
            // Outbound: push samples to the client.
            recv = rx.recv() => match recv {
                Ok(sample) => {
                    let json = match serde_json::to_string(&sample) {
                        Ok(j) => j,
                        Err(e) => {
                            warn!(error = %e, "serialize sample");
                            continue;
                        }
                    };
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(dropped = n, "ws client lagging; dropped samples");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            // Inbound: ignore payloads, but handle close/ping so we notice disconnects.
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(_)) => break,
                _ => {}
            },
        }
    }
    info!("client disconnected");
}
