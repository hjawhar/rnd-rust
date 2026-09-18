use std::collections::VecDeque;

use bytes::Bytes;
use irc_proto::Message;

/// Default maximum number of messages retained per target.
const DEFAULT_MAX: usize = 5000;

/// Per-target ring buffer of timestamped IRC messages.
///
/// Each entry is an `(iso8601_timestamp, message)` pair.  When the buffer
/// exceeds its configured capacity the oldest entries are evicted.
pub struct MessageBuffer {
    entries: VecDeque<(Bytes, Message)>,
    max: usize,
}

impl MessageBuffer {
    /// Create a new buffer with the default capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX)
    }

    /// Create a new buffer with the given maximum entry count.
    pub fn with_capacity(max: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(max.min(1024)),
            max,
        }
    }

    /// Append a timestamped message, evicting the oldest if at capacity.
    pub fn push(&mut self, timestamp: Bytes, message: Message) {
        if self.entries.len() >= self.max {
            self.entries.pop_front();
        }
        self.entries.push_back((timestamp, message));
    }

    /// Return all messages whose timestamp is lexicographically `>= since`.
    ///
    /// ISO 8601 timestamps sort correctly under byte-level comparison, so this
    /// is a simple linear scan from the back.
    pub fn since(&self, since: &[u8]) -> Vec<(Bytes, Message)> {
        // Timestamps are monotonically increasing, so we can binary-search for
        // the first entry >= `since`.
        let idx = self
            .entries
            .iter()
            .position(|(ts, _)| ts.as_ref() >= since)
            .unwrap_or(self.entries.len());
        self.entries.iter().skip(idx).cloned().collect()
    }

    /// Return the last `n` entries (or fewer if the buffer is smaller).
    pub fn last_n(&self, n: usize) -> Vec<(Bytes, Message)> {
        let start = self.entries.len().saturating_sub(n);
        self.entries.iter().skip(start).cloned().collect()
    }

    /// Number of entries currently buffered.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

impl Default for MessageBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use irc_proto::Message;

    fn msg(text: &str) -> Message {
        Message::parse_slice(format!("PRIVMSG #test :{text}").as_bytes())
            .expect("test message should parse")
    }

    #[test]
    fn push_and_last_n() {
        let mut buf = MessageBuffer::new();
        buf.push(Bytes::from_static(b"2025-01-01T00:00:00Z"), msg("a"));
        buf.push(Bytes::from_static(b"2025-01-01T00:01:00Z"), msg("b"));
        buf.push(Bytes::from_static(b"2025-01-01T00:02:00Z"), msg("c"));

        let last2 = buf.last_n(2);
        assert_eq!(last2.len(), 2);
        assert_eq!(last2[0].0.as_ref(), b"2025-01-01T00:01:00Z");
        assert_eq!(last2[1].0.as_ref(), b"2025-01-01T00:02:00Z");
    }

    #[test]
    fn since_filters_correctly() {
        let mut buf = MessageBuffer::new();
        buf.push(Bytes::from_static(b"2025-01-01T00:00:00Z"), msg("a"));
        buf.push(Bytes::from_static(b"2025-01-01T00:05:00Z"), msg("b"));
        buf.push(Bytes::from_static(b"2025-01-01T00:10:00Z"), msg("c"));

        let result = buf.since(b"2025-01-01T00:05:00Z");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0.as_ref(), b"2025-01-01T00:05:00Z");
        assert_eq!(result[1].0.as_ref(), b"2025-01-01T00:10:00Z");
    }

    #[test]
    fn overflow_evicts_oldest() {
        let mut buf = MessageBuffer::with_capacity(3);
        for i in 0..5 {
            let ts = format!("2025-01-01T00:{i:02}:00Z");
            buf.push(Bytes::from(ts), msg(&format!("msg{i}")));
        }
        assert_eq!(buf.len(), 3);
        let all = buf.last_n(10);
        assert_eq!(all[0].0.as_ref(), b"2025-01-01T00:02:00Z");
    }

    #[test]
    fn since_empty_buffer() {
        let buf = MessageBuffer::new();
        assert!(buf.since(b"2025-01-01T00:00:00Z").is_empty());
    }
}
