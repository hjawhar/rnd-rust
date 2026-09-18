//! JSONL log replay transport.
//!
//! Reads a file of `Sample` JSON lines (as produced by the backend's
//! `--record` flag) and replays them. Timing between samples is derived from
//! the recorded `timestamp_ms` deltas and can be scaled via `speed`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::stream::StreamExt;
use makana_common::{Millis, Sample, SamplePayload, SourceEvent, TransportError};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, warn};

use crate::{CarDataSource, SampleStream};

/// Replay a JSONL log file as a `CarDataSource`.
pub struct LogSource {
    path: PathBuf,
    speed: f32,
    repeat: bool,
    name: String,
}

impl LogSource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let name = format!("logfile:{}", file_tag(&path));
        Self {
            path,
            speed: 1.0,
            repeat: false,
            name,
        }
    }

    /// Playback speed multiplier. `2.0` = twice real-time. Must be > 0.
    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// When true, loops back to the start of the file on EOF.
    pub fn with_repeat(mut self, repeat: bool) -> Self {
        self.repeat = repeat;
        self
    }
}

fn file_tag(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("<unknown>")
        .to_string()
}

#[async_trait::async_trait]
impl CarDataSource for LogSource {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(self: Box<Self>) -> Result<SampleStream, TransportError> {
        if !self.speed.is_finite() || self.speed <= 0.0 {
            return Err(TransportError::Other(format!(
                "logfile: speed must be > 0, got {}",
                self.speed
            )));
        }

        // Open once up-front so start() fails loudly if the path is bad,
        // rather than surfacing as a silently-ended stream.
        let initial = File::open(&self.path).await.map_err(TransportError::Io)?;

        let (tx, rx) = mpsc::channel::<Result<Sample, TransportError>>(64);
        let source = self.name.clone();
        let path = self.path.clone();
        let speed = self.speed;
        let repeat = self.repeat;

        tokio::spawn(async move {
            // Announce liveness so consumers treat this like any other source.
            let _ = tx
                .send(Ok(Sample {
                    source: source.clone(),
                    timestamp_ms: 0,
                    payload: SamplePayload::Event(SourceEvent::Connected),
                }))
                .await;

            let mut file = initial;
            loop {
                let emitted = pump_file(&mut file, &source, speed, &tx).await;
                let Some(()) = emitted else {
                    // Consumer dropped.
                    return;
                };

                // EOF: announce disconnect.
                let _ = tx
                    .send(Ok(Sample {
                        source: source.clone(),
                        timestamp_ms: 0,
                        payload: SamplePayload::Event(SourceEvent::Disconnected),
                    }))
                    .await;

                if !repeat {
                    return;
                }

                tokio::time::sleep(Duration::from_secs(1)).await;
                match File::open(&path).await {
                    Ok(f) => {
                        file = f;
                        let _ = tx
                            .send(Ok(Sample {
                                source: source.clone(),
                                timestamp_ms: 0,
                                payload: SamplePayload::Event(SourceEvent::Connected),
                            }))
                            .await;
                    }
                    Err(e) => {
                        warn!(error = %e, "logfile: reopen failed; ending stream");
                        return;
                    }
                }
            }
        });

        Ok(ReceiverStream::new(rx).boxed())
    }
}

/// Read the file to EOF, forwarding each sample through `tx` with replay
/// timing. Returns `Some(())` on clean EOF, `None` if the consumer dropped.
async fn pump_file(
    file: &mut File,
    source: &str,
    speed: f32,
    tx: &mpsc::Sender<Result<Sample, TransportError>>,
) -> Option<()> {
    let reader = BufReader::new(file);
    let mut lines = reader.lines();
    let mut line_no: usize = 0;
    let mut prev_ts: Option<Millis> = None;

    loop {
        let next = match lines.next_line().await {
            Ok(n) => n,
            Err(e) => {
                warn!(error = %e, "logfile: read error");
                return Some(());
            }
        };
        let Some(line) = next else { return Some(()) };
        line_no += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut sample: Sample = match serde_json::from_str(trimmed) {
            Ok(s) => s,
            Err(e) => {
                warn!(line = line_no, error = %e, "logfile: malformed JSONL, skipping");
                continue;
            }
        };

        if let Some(prev) = prev_ts {
            let delta_ms = sample.timestamp_ms.saturating_sub(prev);
            if sample.timestamp_ms < prev {
                warn!(
                    line = line_no,
                    prev,
                    current = sample.timestamp_ms,
                    "logfile: timestamp went backwards; emitting immediately"
                );
            } else if delta_ms > 0 {
                let scaled = (delta_ms as f32) / speed;
                let dur = Duration::from_secs_f32(scaled / 1000.0);
                tokio::time::sleep(dur).await;
            }
        }
        prev_ts = Some(sample.timestamp_ms);

        sample.source = source.to_string();
        if tx.send(Ok(sample)).await.is_err() {
            debug!("logfile consumer dropped");
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use makana_common::{ObdValue, SamplePayload};
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_jsonl(samples: &[Sample]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        for s in samples {
            let line = serde_json::to_string(s).unwrap();
            writeln!(f, "{line}").unwrap();
        }
        f.flush().unwrap();
        f
    }

    fn obd_sample(ts: Millis, rpm: u32) -> Sample {
        Sample {
            source: "mock".into(),
            timestamp_ms: ts,
            payload: SamplePayload::Obd(ObdValue::Rpm(rpm)),
        }
    }

    #[tokio::test]
    async fn replays_samples_and_rewrites_source() {
        let file = write_jsonl(&[
            obd_sample(1000, 800),
            obd_sample(1010, 900),
            obd_sample(1020, 1000),
        ]);

        let src: Box<dyn CarDataSource> = Box::new(LogSource::new(file.path()).with_speed(1000.0));
        let mut stream = src.start().await.unwrap();

        // First: Connected event.
        let first = stream.next().await.unwrap().unwrap();
        assert!(matches!(
            first.payload,
            SamplePayload::Event(SourceEvent::Connected)
        ));

        for expected_rpm in [800u32, 900, 1000] {
            let s = stream.next().await.unwrap().unwrap();
            assert!(s.source.starts_with("logfile:"));
            match s.payload {
                SamplePayload::Obd(ObdValue::Rpm(r)) => assert_eq!(r, expected_rpm),
                other => panic!("unexpected payload {other:?}"),
            }
        }

        // Then: Disconnected on EOF.
        let last = stream.next().await.unwrap().unwrap();
        assert!(matches!(
            last.payload,
            SamplePayload::Event(SourceEvent::Disconnected)
        ));
    }

    #[tokio::test]
    async fn skips_malformed_lines() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "this is not json").unwrap();
        writeln!(f, "{}", serde_json::to_string(&obd_sample(1, 750)).unwrap()).unwrap();
        writeln!(f, "{{also not a Sample: true}}").unwrap();
        writeln!(f, "{}", serde_json::to_string(&obd_sample(2, 760)).unwrap()).unwrap();
        f.flush().unwrap();

        let src: Box<dyn CarDataSource> = Box::new(LogSource::new(f.path()).with_speed(1000.0));
        let mut stream = src.start().await.unwrap();

        let _connected = stream.next().await.unwrap().unwrap();
        let a = stream.next().await.unwrap().unwrap();
        let b = stream.next().await.unwrap().unwrap();
        assert!(matches!(a.payload, SamplePayload::Obd(ObdValue::Rpm(750))));
        assert!(matches!(b.payload, SamplePayload::Obd(ObdValue::Rpm(760))));
    }

    #[tokio::test]
    async fn rejects_nonpositive_speed() {
        let f = write_jsonl(&[obd_sample(1, 100)]);
        let src: Box<dyn CarDataSource> = Box::new(LogSource::new(f.path()).with_speed(0.0));
        let res = src.start().await;
        assert!(res.is_err(), "expected start() to reject speed=0");
    }

    #[tokio::test]
    async fn empty_file_emits_disconnect_and_ends() {
        let f = NamedTempFile::new().unwrap();
        let src: Box<dyn CarDataSource> = Box::new(LogSource::new(f.path()));
        let mut stream = src.start().await.unwrap();
        let a = stream.next().await.unwrap().unwrap();
        assert!(matches!(
            a.payload,
            SamplePayload::Event(SourceEvent::Connected)
        ));
        let b = stream.next().await.unwrap().unwrap();
        assert!(matches!(
            b.payload,
            SamplePayload::Event(SourceEvent::Disconnected)
        ));
        // Stream should end.
        assert!(stream.next().await.is_none());
    }
}
