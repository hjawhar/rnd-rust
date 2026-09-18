//! A synthetic source. Produces plausible OBD samples at a fixed cadence so
//! the frontend can be developed with zero hardware and zero emulator setup.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::stream::StreamExt;
use makana_common::{ObdValue, Sample, SamplePayload, SourceEvent, TransportError};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::debug;

use crate::{CarDataSource, SampleStream};

pub struct MockSource {
    name: String,
    tick: Duration,
}

impl MockSource {
    pub fn new(tick: Duration) -> Self {
        Self {
            name: "mock".into(),
            tick,
        }
    }
}

impl Default for MockSource {
    fn default() -> Self {
        Self::new(Duration::from_millis(100))
    }
}

#[async_trait::async_trait]
impl CarDataSource for MockSource {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(self: Box<Self>) -> Result<SampleStream, TransportError> {
        let (tx, rx) = mpsc::channel(64);
        let name = self.name.clone();
        let tick = self.tick;

        // Emit a connect event immediately so consumers see liveness.
        let connect = Sample {
            source: name.clone(),
            timestamp_ms: now_ms(),
            payload: SamplePayload::Event(SourceEvent::Connected),
        };
        let _ = tx.send(Ok(connect)).await;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tick);
            let mut step: u64 = 0;
            loop {
                interval.tick().await;
                // One sample per cycle, rotating through a few PIDs so the UI
                // sees varied traffic without us having to model a vehicle.
                let value = match step % 4 {
                    0 => ObdValue::Rpm(750 + ((step % 60) * 50) as u32),
                    1 => ObdValue::SpeedKph(((step % 120) as u16).min(120)),
                    2 => ObdValue::CoolantTempC(85 + ((step % 10) as i16)),
                    _ => ObdValue::BatteryVolts(13.8 + ((step % 4) as f32) * 0.05),
                };
                let sample = Sample {
                    source: name.clone(),
                    timestamp_ms: now_ms(),
                    payload: SamplePayload::Obd(value),
                };
                if tx.send(Ok(sample)).await.is_err() {
                    debug!("mock consumer dropped");
                    break;
                }
                step = step.wrapping_add(1);
            }
        });

        Ok(ReceiverStream::new(rx).boxed())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn mock_emits_samples() {
        let src: Box<dyn CarDataSource> = Box::new(MockSource::new(Duration::from_millis(5)));
        let mut stream = src.start().await.unwrap();

        // First sample is always the Connected event.
        let first = stream.next().await.unwrap().unwrap();
        assert!(matches!(
            first.payload,
            SamplePayload::Event(SourceEvent::Connected)
        ));

        // Subsequent samples should be OBD readings.
        for _ in 0..3 {
            let s = stream.next().await.unwrap().unwrap();
            assert!(matches!(s.payload, SamplePayload::Obd(_)));
        }
    }
}
