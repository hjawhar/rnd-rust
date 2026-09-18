//! Transport abstraction for makana.
//!
//! A `CarDataSource` produces a stream of `Sample`s. The backend doesn't care
//! whether they come from an ELM327, a CAN dongle, an ESP32 over WebSocket, or
//! a replayed log. Adding a new source is writing one impl.

#![forbid(unsafe_code)]

use std::pin::Pin;

use futures::stream::Stream;
use makana_common::{Sample, TransportError};

pub mod elm327;
pub mod logfile;
pub mod mock;

pub use elm327::Elm327Source;
pub use logfile::LogSource;
pub use mock::MockSource;

/// Stream of samples from a source. Items are `Result` so transient errors can
/// surface without killing the feed.
pub type SampleStream = Pin<Box<dyn Stream<Item = Result<Sample, TransportError>> + Send>>;

/// A source of vehicle telemetry.
///
/// Implementations own their own IO, polling loops, reconnection policy.
/// The caller only consumes the stream.
#[async_trait::async_trait]
pub trait CarDataSource: Send {
    /// Identifier surfaced in every `Sample.source`. Usually `"kind:device"`.
    fn name(&self) -> &str;

    /// Start the source. After this returns, the stream is live.
    ///
    /// Consuming this `self` prevents calling `start` twice — each source is
    /// single-shot. Create a new instance to reconnect.
    async fn start(self: Box<Self>) -> Result<SampleStream, TransportError>;
}
