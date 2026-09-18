//! Tokio codec wrapping the wire parser / serializer.
//!
//! [`IrcCodec`] speaks line-delimited IRC on top of a byte stream:
//!
//! - **Decoder**: pulls a line from a `BytesMut`, accepts `\r\n` (strict)
//!   or bare `\n` (lenient), strips the terminator, and delegates to
//!   [`Message::parse`]. Enforces [`MAX_LINE_BYTES`] — oversize lines
//!   are drained from the buffer and surfaced as
//!   [`CodecError::LineTooLong`].
//! - **Encoder**: serializes a [`Message`], appends `\r\n`, and errors
//!   out if the total exceeds the byte limit (with the just-written
//!   bytes rolled back so the buffer stays consistent for retries).

use bytes::{Buf, BytesMut};
use thiserror::Error;
use tokio_util::codec::{Decoder, Encoder};

use crate::error::ParseError;
use crate::limits::MAX_LINE_BYTES;
use crate::message::Message;

/// Errors produced by [`IrcCodec`].
#[derive(Debug, Error)]
pub enum CodecError {
    /// A decoded or encoded line exceeded [`MAX_LINE_BYTES`].
    #[error("line exceeds {MAX_LINE_BYTES} byte limit")]
    LineTooLong,
    /// The wire-level parser rejected the line.
    #[error(transparent)]
    Parse(#[from] ParseError),
    /// Transport I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Line-delimited IRC codec.
#[derive(Debug, Clone, Copy)]
pub struct IrcCodec {
    max_bytes: usize,
}

impl Default for IrcCodec {
    fn default() -> Self {
        Self {
            max_bytes: MAX_LINE_BYTES,
        }
    }
}

impl IrcCodec {
    /// Construct with the protocol default limit.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_bytes: MAX_LINE_BYTES,
        }
    }

    /// Construct with a custom byte limit (for testing or constrained
    /// deployments). Values larger than [`MAX_LINE_BYTES`] are legal
    /// but make the codec more permissive than the protocol allows.
    #[must_use]
    pub const fn with_limit(max_bytes: usize) -> Self {
        Self { max_bytes }
    }

    /// Current byte limit.
    #[must_use]
    pub const fn limit(&self) -> usize {
        self.max_bytes
    }
}

impl Decoder for IrcCodec {
    type Item = Message;
    type Error = CodecError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Message>, Self::Error> {
        loop {
            let Some(nl) = buf.iter().position(|b| *b == b'\n') else {
                // No terminator yet. If we've already accumulated more
                // than the limit without seeing one, reject.
                if buf.len() > self.max_bytes {
                    // Drop everything up to the next `\n` we eventually
                    // see, so the stream can recover. We haven't seen
                    // one yet — clear the over-long buffer wholesale.
                    buf.clear();
                    return Err(CodecError::LineTooLong);
                }
                return Ok(None);
            };
            // Treat the bytes up to (but not including) `\n` as the
            // content. Drop a trailing `\r` if present.
            let content_end = if nl > 0 && buf[nl - 1] == b'\r' {
                nl - 1
            } else {
                nl
            };
            if content_end > self.max_bytes {
                buf.advance(nl + 1);
                return Err(CodecError::LineTooLong);
            }
            // Split off the line (content + terminator).
            let mut line = buf.split_to(nl + 1);
            line.truncate(content_end);
            let content = line.freeze();
            if content.is_empty() {
                // Tolerate empty keepalive lines silently.
                continue;
            }
            return Message::parse(&content).map(Some).map_err(Into::into);
        }
    }
}

impl Encoder<Message> for IrcCodec {
    type Error = CodecError;

    fn encode(&mut self, msg: Message, out: &mut BytesMut) -> Result<(), Self::Error> {
        let start = out.len();
        msg.write(out);
        let body_len = out.len() - start;
        // +2 for CRLF.
        if body_len + 2 > self.max_bytes {
            out.truncate(start);
            return Err(CodecError::LineTooLong);
        }
        out.extend_from_slice(b"\r\n");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{CodecError, IrcCodec};
    use crate::Message;
    use bytes::{BufMut, BytesMut};
    use tokio_util::codec::{Decoder, Encoder};

    fn feed(s: &[u8]) -> BytesMut {
        let mut buf = BytesMut::new();
        buf.put_slice(s);
        buf
    }

    #[test]
    fn decodes_single_crlf_terminated_line() {
        let mut codec = IrcCodec::new();
        let mut buf = feed(b"PING :abc\r\n");
        let msg = codec.decode(&mut buf).unwrap().expect("have message");
        assert_eq!(
            msg.verb,
            crate::Verb::word(bytes::Bytes::from_static(b"PING"))
        );
        assert!(buf.is_empty(), "all bytes consumed");
    }

    #[test]
    fn decodes_bare_lf_leniently() {
        let mut codec = IrcCodec::new();
        let mut buf = feed(b"PING :abc\n");
        let msg = codec.decode(&mut buf).unwrap().expect("have message");
        assert_eq!(&*msg.params[0], b"abc");
    }

    #[test]
    fn empty_lines_are_skipped_silently() {
        let mut codec = IrcCodec::new();
        let mut buf = feed(b"\r\n\r\nPING :abc\r\n");
        let msg = codec.decode(&mut buf).unwrap().expect("have message");
        assert_eq!(&*msg.params[0], b"abc");
    }

    #[test]
    fn partial_line_returns_none() {
        let mut codec = IrcCodec::new();
        let mut buf = feed(b"PING :ab");
        assert!(codec.decode(&mut buf).unwrap().is_none());
        // Complete the line on the next chunk.
        buf.extend_from_slice(b"c\r\n");
        let msg = codec.decode(&mut buf).unwrap().expect("have message");
        assert_eq!(&*msg.params[0], b"abc");
    }

    #[test]
    fn oversize_line_errors() {
        let mut codec = IrcCodec::with_limit(10);
        let mut buf = feed(b"PING :a_very_long_string\r\n");
        match codec.decode(&mut buf) {
            Err(CodecError::LineTooLong) => {}
            other => panic!("expected LineTooLong, got {other:?}"),
        }
        // Subsequent valid line still decodes.
        let mut codec = IrcCodec::with_limit(32);
        let mut buf = feed(b"PING :x\r\n");
        codec.decode(&mut buf).unwrap().unwrap();
    }

    #[test]
    fn oversize_without_newline_clears_buffer() {
        let mut codec = IrcCodec::with_limit(5);
        let mut buf = feed(b"PING :this-is-long-without-newline");
        match codec.decode(&mut buf) {
            Err(CodecError::LineTooLong) => {}
            other => panic!("expected LineTooLong, got {other:?}"),
        }
        assert!(buf.is_empty());
    }

    #[test]
    fn encodes_with_crlf_terminator() {
        let mut codec = IrcCodec::new();
        let mut buf = BytesMut::new();
        let msg = Message::parse_slice(b"PING :abc").unwrap();
        codec.encode(msg, &mut buf).unwrap();
        assert_eq!(&buf[..], b"PING :abc\r\n");
    }

    #[test]
    fn encode_rolls_back_on_overflow() {
        let mut codec = IrcCodec::with_limit(8);
        let mut buf = BytesMut::new();
        let msg = Message::parse_slice(b"PING :abc").unwrap();
        let result = codec.encode(msg, &mut buf);
        assert!(matches!(result, Err(CodecError::LineTooLong)));
        assert!(buf.is_empty(), "buffer rolled back on error");
    }
}
