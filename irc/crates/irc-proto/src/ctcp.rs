//! CTCP (Client-To-Client Protocol) encapsulation.
//!
//! CTCP payloads hitch a ride inside a regular `PRIVMSG` or `NOTICE`
//! trailing text, delimited by `\x01`. The structure is
//! `\x01CMD [args]\x01` — the trailing `\x01` is optional in practice
//! but every well-formed emitter includes it.
//!
//! This module provides round-trip encode/decode without interpreting
//! the command semantics. Known commands (like `ACTION` for `/me`) are
//! surfaced as typed variants for the most common case.

use bytes::{BufMut, Bytes, BytesMut};

/// A decoded CTCP message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CtcpMessage {
    /// The command verb (e.g. `ACTION`, `VERSION`, `TIME`).
    pub command: Bytes,
    /// The remaining bytes after the command and (optional) space.
    /// Empty when there are no args (e.g. `CTCP VERSION` query).
    pub args: Bytes,
}

impl CtcpMessage {
    /// Parse a CTCP payload from a raw `PRIVMSG`/`NOTICE` text body.
    ///
    /// Returns `None` if the body is not a CTCP (does not start with
    /// `\x01`).
    #[must_use]
    pub fn parse(text: &Bytes) -> Option<Self> {
        let bytes: &[u8] = text.as_ref();
        if bytes.first() != Some(&0x01) {
            return None;
        }
        // Strip leading \x01 and optional trailing \x01.
        let inner_start = 1;
        let inner_end = if bytes.last() == Some(&0x01) {
            bytes.len() - 1
        } else {
            bytes.len()
        };
        if inner_end <= inner_start {
            // Empty CTCP — not useful but legal.
            return Some(Self {
                command: Bytes::new(),
                args: Bytes::new(),
            });
        }
        let inner = text.slice(inner_start..inner_end);
        let inner_bytes: &[u8] = inner.as_ref();
        match inner_bytes.iter().position(|b| *b == b' ') {
            Some(sp) => Some(Self {
                command: inner.slice(..sp),
                args: inner.slice(sp + 1..),
            }),
            None => Some(Self {
                command: inner,
                args: Bytes::new(),
            }),
        }
    }

    /// Serialize back into a `\x01CMD args\x01` payload suitable for
    /// placing in a `PRIVMSG`/`NOTICE` trailing.
    #[must_use]
    pub fn write(&self) -> Bytes {
        let mut out = BytesMut::with_capacity(2 + self.command.len() + 1 + self.args.len());
        out.put_u8(0x01);
        out.extend_from_slice(self.command.as_ref());
        if !self.args.is_empty() {
            out.put_u8(b' ');
            out.extend_from_slice(self.args.as_ref());
        }
        out.put_u8(0x01);
        out.freeze()
    }

    /// Return `true` when the verb matches `ACTION` (the `/me` CTCP).
    #[must_use]
    pub fn is_action(&self) -> bool {
        self.command.as_ref().eq_ignore_ascii_case(b"ACTION")
    }
}

/// Build a CTCP ACTION payload (`/me ...`).
#[must_use]
pub fn action(text: impl Into<Bytes>) -> Bytes {
    CtcpMessage {
        command: Bytes::from_static(b"ACTION"),
        args: text.into(),
    }
    .write()
}

#[cfg(test)]
mod tests {
    use super::{CtcpMessage, action};
    use bytes::Bytes;

    fn payload(s: &[u8]) -> Bytes {
        Bytes::copy_from_slice(s)
    }

    #[test]
    fn parses_action_with_args() {
        let body = payload(b"\x01ACTION waves\x01");
        let c = CtcpMessage::parse(&body).unwrap();
        assert_eq!(c.command.as_ref(), b"ACTION");
        assert_eq!(c.args.as_ref(), b"waves");
        assert!(c.is_action());
    }

    #[test]
    fn parses_without_trailing_terminator() {
        let body = payload(b"\x01VERSION");
        let c = CtcpMessage::parse(&body).unwrap();
        assert_eq!(c.command.as_ref(), b"VERSION");
        assert!(c.args.is_empty());
    }

    #[test]
    fn non_ctcp_returns_none() {
        assert!(CtcpMessage::parse(&payload(b"hello")).is_none());
        assert!(CtcpMessage::parse(&payload(b"")).is_none());
    }

    #[test]
    fn round_trip_action() {
        let body = payload(b"\x01ACTION waves\x01");
        let c = CtcpMessage::parse(&body).unwrap();
        assert_eq!(c.write(), body);
    }

    #[test]
    fn action_helper_produces_canonical_payload() {
        assert_eq!(
            action(Bytes::from_static(b"waves")).as_ref(),
            b"\x01ACTION waves\x01"
        );
    }

    #[test]
    fn is_action_is_case_insensitive() {
        let c = CtcpMessage::parse(&payload(b"\x01action waves\x01")).unwrap();
        assert!(c.is_action());
    }
}
