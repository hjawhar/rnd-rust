//! End-to-end smoke: spawn the backend against the mock source, connect over
//! WebSocket, assert at least one sample arrives and parses back into a
//! `Sample`. If this passes, the whole `transport → backend → WS → client`
//! chain is wired correctly.

use std::{
    net::SocketAddr,
    process::Stdio,
    time::{Duration, Instant},
};

use futures::StreamExt;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpStream,
    process::Command,
};
use tokio_tungstenite::tungstenite::Message;

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_makana-backend")
}

async fn wait_for_port(addr: SocketAddr, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect(addr).await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

#[tokio::test]
async fn mock_samples_reach_ws_clients() {
    // Fixed high port; integration tests run serially by default in a single
    // crate, so collisions are unlikely. If this ever flakes, switch to `:0`
    // + parse the bound port from the backend's stderr.
    let port = 18_765u16;
    let bind: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

    let mut child = Command::new(bin_path())
        .args([
            "--transport",
            "mock",
            "--mock-tick-ms",
            "10",
            "--bind",
            &bind.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn backend");

    // Drain logs so the subprocess doesn't stall on a full pipe.
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(_)) = lines.next_line().await {}
        });
    }
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(_)) = lines.next_line().await {}
        });
    }

    assert!(
        wait_for_port(bind, Duration::from_secs(5)).await,
        "backend did not start"
    );

    let url = format!("ws://{bind}/ws");
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("ws connect");

    let mut obd_seen = false;
    for _ in 0..10 {
        let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
            .await
            .expect("ws recv timeout")
            .expect("ws stream ended")
            .expect("ws error");
        let Message::Text(text) = msg else {
            continue;
        };
        let sample: makana_common::Sample =
            serde_json::from_str(&text).expect("sample parses back");
        assert_eq!(sample.source, "mock");
        if matches!(sample.payload, makana_common::SamplePayload::Obd(_)) {
            obd_seen = true;
            break;
        }
    }
    assert!(obd_seen, "expected at least one OBD sample");

    let _ = child.kill().await;
}
