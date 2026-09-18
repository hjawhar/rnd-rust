//! End-to-end against the real ELM327-emulator (Python).
//!
//! Proves `transport=elm327 → serial/pty → backend → WS → parse` works, not
//! just that the decoder unit-tests pass on synthesized strings.
//!
//! Requires `ELM327-emulator` installed. The test is `#[ignore]`d by default;
//! run it explicitly:
//!
//! ```sh
//! cargo test -p makana-backend --test elm327_emulator -- --ignored --nocapture
//! ```
//!
//! Override the emulator binary with `MAKANA_ELM_BIN` (defaults to
//! `./.venv/bin/elm` relative to the workspace root).

use std::{
    net::SocketAddr,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

use futures::StreamExt;
use makana_common::{ObdValue, Sample, SamplePayload, SourceEvent};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpStream,
    process::{Child, Command},
};
use tokio_tungstenite::tungstenite::Message;

fn backend_bin() -> &'static str {
    env!("CARGO_BIN_EXE_makana-backend")
}

fn elm_bin() -> PathBuf {
    if let Ok(p) = std::env::var("MAKANA_ELM_BIN") {
        return PathBuf::from(p);
    }
    // CARGO_MANIFEST_DIR = crates/backend; workspace root is two up.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(".venv/bin/elm")
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

/// Read the pty path from elm's batch-mode output file. First line carries
/// the device (e.g. `/dev/ttys003`). Polls with a deadline.
async fn await_pty_path(batch_file: &std::path::Path, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(contents) = tokio::fs::read_to_string(batch_file).await
            && let Some(first) = contents.lines().next()
        {
            let trimmed = first.trim();
            if trimmed.starts_with("/dev/") {
                return Some(trimmed.to_string());
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    None
}

fn drain_child_output(child: &mut Child) {
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(_)) = lines.next_line().await {}
        });
    }
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(_)) = lines.next_line().await {}
        });
    }
}

#[tokio::test]
#[ignore = "requires ELM327-emulator; run with --ignored"]
async fn elm327_emulator_produces_decoded_samples() {
    let elm = elm_bin();
    assert!(
        elm.exists(),
        "ELM327 emulator not found at {}. Install with: \
         python3 -m venv .venv && .venv/bin/pip install 'setuptools<70' && \
         .venv/bin/pip install --no-build-isolation ELM327-emulator",
        elm.display()
    );

    // Batch-mode output file; emulator writes the pty path on its first line.
    let batch = tempfile::Builder::new()
        .prefix("makana-elm-")
        .suffix(".txt")
        .tempfile()
        .expect("tempfile");
    let batch_path = batch.path().to_path_buf();
    // Close the handle but keep the path; the emulator opens it itself.
    drop(batch);
    let _ = std::fs::remove_file(&batch_path);

    // 1. Spawn the emulator.
    let mut emu = Command::new(&elm)
        .arg("-b")
        .arg(&batch_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn elm emulator");
    drain_child_output(&mut emu);

    let pty = await_pty_path(&batch_path, Duration::from_secs(10))
        .await
        .expect("emulator did not announce a pty path");
    eprintln!("emulator pty: {pty}");

    // 2. Spawn the backend pointing at the pty.
    let port = 18_766u16;
    let bind: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let mut backend = Command::new(backend_bin())
        .args([
            "--transport",
            "elm327",
            "--device",
            &pty,
            "--bind",
            &bind.to_string(),
        ])
        .env("RUST_LOG", "makana_backend=info,makana_transport=debug")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn backend");
    drain_child_output(&mut backend);

    assert!(
        wait_for_port(bind, Duration::from_secs(10)).await,
        "backend did not start"
    );

    // 3. Consume the WS feed until we see a decoded OBD value (not just Raw).
    let url = format!("ws://{bind}/ws");
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("ws connect");

    let mut saw_connected = false;
    let mut decoded_seen = 0usize;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline && decoded_seen < 2 {
        let msg = match tokio::time::timeout(Duration::from_secs(3), ws.next()).await {
            Ok(Some(Ok(m))) => m,
            Ok(Some(Err(e))) => panic!("ws error: {e}"),
            Ok(None) => panic!("ws stream ended"),
            Err(_) => panic!("ws recv timeout; no samples within 3s"),
        };
        let Message::Text(text) = msg else { continue };
        let sample: Sample = serde_json::from_str(&text).expect("sample parses");
        assert!(
            sample.source.starts_with("elm327:"),
            "unexpected source {:?}",
            sample.source
        );
        match sample.payload {
            SamplePayload::Event(SourceEvent::Connected) => saw_connected = true,
            SamplePayload::Event(SourceEvent::Error { ref message }) => {
                panic!("transport error event: {message}")
            }
            SamplePayload::Obd(ref v) if !matches!(v, ObdValue::Raw { .. }) => {
                eprintln!("decoded: {v:?}");
                decoded_seen += 1;
            }
            _ => {}
        }
    }

    assert!(saw_connected, "never received Connected event");
    assert!(
        decoded_seen >= 2,
        "expected >=2 decoded OBD samples, saw {decoded_seen}"
    );

    let _ = backend.kill().await;
    let _ = emu.kill().await;
}
