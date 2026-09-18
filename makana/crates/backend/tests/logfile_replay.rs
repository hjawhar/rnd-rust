//! End-to-end: record live samples to JSONL, then replay them through a
//! second backend and verify they arrive on the WS feed with the `logfile:`
//! source marker.

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

fn spawn_with_drained_pipes(mut cmd: Command) -> tokio::process::Child {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn backend");
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
    child
}

#[tokio::test]
async fn record_then_replay_roundtrip() {
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let log_path = tmp_dir.path().join("replay.jsonl");

    // Phase 1: record from the mock transport.
    let record_port = 18_781u16;
    let record_bind: SocketAddr = format!("127.0.0.1:{record_port}").parse().unwrap();

    let mut rec_cmd = Command::new(bin_path());
    rec_cmd.args([
        "--transport",
        "mock",
        "--mock-tick-ms",
        "10",
        "--bind",
        &record_bind.to_string(),
        "--record",
        log_path.to_str().unwrap(),
    ]);
    let mut rec_child = spawn_with_drained_pipes(rec_cmd);
    assert!(
        wait_for_port(record_bind, Duration::from_secs(5)).await,
        "record-phase backend did not start"
    );

    // Run long enough to capture at least ~30 samples at 10ms cadence.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let _ = rec_child.kill().await;
    let _ = rec_child.wait().await;

    let contents = tokio::fs::read_to_string(&log_path)
        .await
        .expect("read recorded log");
    let non_empty_lines: Vec<&str> = contents.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        non_empty_lines.len() >= 10,
        "expected >= 10 recorded samples, got {}",
        non_empty_lines.len()
    );
    for (i, line) in non_empty_lines.iter().enumerate() {
        let _: makana_common::Sample =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("line {i} parse: {e} — {line}"));
    }

    // Phase 2: replay the recorded file through a second backend.
    let replay_port = 18_782u16;
    let replay_bind: SocketAddr = format!("127.0.0.1:{replay_port}").parse().unwrap();

    let mut rep_cmd = Command::new(bin_path());
    rep_cmd.args([
        "--transport",
        "logfile",
        "--device",
        log_path.to_str().unwrap(),
        "--speed",
        "100.0",
        "--bind",
        &replay_bind.to_string(),
    ]);
    let mut rep_child = spawn_with_drained_pipes(rep_cmd);
    assert!(
        wait_for_port(replay_bind, Duration::from_secs(5)).await,
        "replay-phase backend did not start"
    );

    let url = format!("ws://{replay_bind}/ws");
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("ws connect");

    let mut obd_seen = false;
    let mut source_checked = false;
    for _ in 0..30 {
        let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
            .await
            .expect("ws recv timeout")
            .expect("ws stream ended")
            .expect("ws error");
        let Message::Text(text) = msg else { continue };
        let sample: makana_common::Sample =
            serde_json::from_str(&text).expect("sample parses back");
        assert!(
            sample.source.starts_with("logfile:"),
            "expected logfile: prefix, got {}",
            sample.source
        );
        source_checked = true;
        if matches!(sample.payload, makana_common::SamplePayload::Obd(_)) {
            obd_seen = true;
            break;
        }
    }
    assert!(source_checked, "received no ws samples at all");
    assert!(obd_seen, "expected at least one OBD sample from replay");

    let _ = rep_child.kill().await;
    let _ = rep_child.wait().await;
}
