//! Wire-level [`Message`] type with parse and serialize entry points.
//!
//! Grammar per RFC 1459 / RFC 2812 plus IRCv3 `message-tags`:
//!
//! ```text
//! message = ["@" tags SPACE] [":" prefix SPACE] command [params] [CRLF]
//! ```
//!
//! The parser is strict about the single-space separators between the
//! main regions (tags, prefix, command) and about the forbidden bytes
//! (NUL, CR, LF) inside any region. The codec layer (added later in
//! Phase 1) is responsible for stripping the trailing CRLF before
//! invoking [`Message::parse`].

use bytes::{Bytes, BytesMut};

use crate::error::ParseError;
use crate::params::{Params, parse_params, write_params};
use crate::prefix::{Prefix, parse_prefix, write_prefix};
use crate::tags::{Tags, parse_tags, write_tags};
use crate::verb::{Verb, parse_verb, write_verb};

/// A single IRC message in its wire-level form.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Message {
    /// Optional tag set (IRCv3).
    pub tags: Tags,
    /// Optional origin.
    pub prefix: Option<Prefix>,
    /// Command verb (alphabetic word or three-digit numeric).
    pub verb: Verb,
    /// Ordered params.
    pub params: Params,
}

impl Default for Verb {
    fn default() -> Self {
        Self::Word(Bytes::new())
    }
}

impl Message {
    /// Parse a single message from `buf`. The caller is responsible for
    /// stripping any trailing CRLF.
    pub fn parse(buf: &Bytes) -> Result<Self, ParseError> {
        if buf.is_empty() {
            return Err(ParseError::Empty);
        }
        let data = buf.as_ref();
        let mut pos = 0usize;

        let tags = if data[pos] == b'@' {
            pos += 1;
            let t = parse_tags(buf, &mut pos)?;
            // parse_tags leaves pos on the space.
            if data.get(pos) != Some(&b' ') {
                return Err(ParseError::MissingSpace(pos));
            }
            pos += 1;
            t
        } else {
            Tags::new()
        };

        let prefix = if data.get(pos) == Some(&b':') {
            pos += 1;
            let p = parse_prefix(buf, &mut pos)?;
            if data.get(pos) != Some(&b' ') {
                return Err(ParseError::MissingSpace(pos));
            }
            pos += 1;
            Some(p)
        } else {
            None
        };

        let verb = parse_verb(buf, &mut pos)?;
        let params = parse_params(buf, &mut pos)?;

        // Belt-and-braces: we should now be at end-of-input.
        if pos != data.len() {
            return Err(ParseError::MissingSpace(pos));
        }

        Ok(Self {
            tags,
            prefix,
            verb,
            params,
        })
    }

    /// Parse from a borrowed byte slice by copying into an owned [`Bytes`].
    /// Convenience for tests and call sites that don't already have a
    /// [`Bytes`] in hand.
    pub fn parse_slice(buf: &[u8]) -> Result<Self, ParseError> {
        Self::parse(&Bytes::copy_from_slice(buf))
    }

    /// Serialize this message into `out`. Does **not** append CRLF; the
    /// codec layer owns that responsibility.
    pub fn write(&self, out: &mut BytesMut) {
        write_tags(&self.tags, out);
        if let Some(p) = &self.prefix {
            write_prefix(p, out);
        }
        write_verb(&self.verb, out);
        write_params(&self.params, out);
    }

    /// Convenience: serialize into a fresh [`Bytes`].
    #[must_use]
    pub fn to_bytes(&self) -> Bytes {
        let mut out = BytesMut::new();
        self.write(&mut out);
        out.freeze()
    }
}

#[cfg(test)]
mod tests {
    use super::Message;
    use crate::params::Params;
    use crate::prefix::Prefix;
    use crate::verb::Verb;
    use bytes::Bytes;

    fn parse(s: &str) -> Message {
        Message::parse_slice(s.as_bytes()).unwrap_or_else(|e| panic!("parse {s:?}: {e}"))
    }

    #[test]
    fn bare_command() {
        let m = parse("PING");
        assert!(m.tags.is_empty());
        assert!(m.prefix.is_none());
        assert_eq!(m.verb, Verb::Word(Bytes::from_static(b"PING")));
        assert!(m.params.is_empty());
    }

    #[test]
    fn command_with_middle_only() {
        let m = parse("PING xyz");
        assert_eq!(m.verb, Verb::Word(Bytes::from_static(b"PING")));
        assert_eq!(m.params.len(), 1);
        assert_eq!(&*m.params[0], b"xyz");
        assert!(!m.params.is_trailing());
    }

    #[test]
    fn command_with_trailing_only() {
        let m = parse("PING :hello world");
        assert_eq!(m.params.len(), 1);
        assert_eq!(&*m.params[0], b"hello world");
        assert!(m.params.is_trailing());
    }

    #[test]
    fn numeric_command_with_trailing() {
        let m = parse(":irc.example.net 001 alice :Welcome to ExampleNet alice!~al@host");
        assert_eq!(
            m.prefix,
            Some(Prefix::Server(Bytes::from_static(b"irc.example.net")))
        );
        assert_eq!(m.verb, Verb::Numeric(1));
        assert_eq!(m.params.len(), 2);
        assert_eq!(&*m.params[0], b"alice");
        assert_eq!(&*m.params[1], b"Welcome to ExampleNet alice!~al@host");
    }

    #[test]
    fn tagged_privmsg() {
        let m =
            parse("@time=2026-01-01T00:00:00.000Z;+client=abc :alice!~al@host PRIVMSG #rust :hi");
        assert_eq!(m.tags.len(), 2);
        assert_eq!(
            m.tags.get(b"time").unwrap().value.as_deref(),
            Some(&b"2026-01-01T00:00:00.000Z"[..])
        );
        assert!(m.tags.get(b"client").unwrap().key.client_only);
        assert_eq!(m.verb, Verb::Word(Bytes::from_static(b"PRIVMSG")));
        assert_eq!(m.params.len(), 2);
        assert_eq!(&*m.params[0], b"#rust");
        assert_eq!(&*m.params[1], b"hi");
    }

    #[test]
    fn round_trip_bare_command() {
        let s = b"PING";
        let m = Message::parse_slice(s).unwrap();
        assert_eq!(m.to_bytes().as_ref(), &s[..]);
    }

    #[test]
    fn round_trip_full_shape() {
        let s = b"@time=2026-01-01T00:00:00.000Z :alice!~al@host PRIVMSG #rust :hello world";
        let m = Message::parse_slice(s).unwrap();
        assert_eq!(m.to_bytes().as_ref(), &s[..]);
    }

    #[test]
    fn rejects_empty() {
        use crate::error::ParseError;
        assert_eq!(Message::parse_slice(b""), Err(ParseError::Empty));
    }

    #[test]
    fn rejects_nul_inside_params() {
        assert!(Message::parse_slice(b"PRIVMSG #a :he\0lo").is_err());
    }

    #[test]
    fn rejects_lf_inside_tag_value() {
        assert!(Message::parse_slice(b"@k=va\nlue FOO").is_err());
    }

    #[test]
    fn command_word_must_be_alphabetic() {
        assert!(Message::parse_slice(b"PRIV1 a").is_err());
    }

    #[test]
    fn numeric_must_be_exactly_three_digits() {
        assert!(Message::parse_slice(b"12 a").is_err());
        assert!(Message::parse_slice(b"1234 a").is_err());
    }

    #[test]
    fn params_round_trip_preserves_trailing_flag() {
        // `PRIVMSG #chan hi` vs `PRIVMSG #chan :hi` must stay distinct.
        let without_colon = Message::parse_slice(b"PRIVMSG #chan hi").unwrap();
        let with_colon = Message::parse_slice(b"PRIVMSG #chan :hi").unwrap();
        assert_ne!(without_colon, with_colon);
        assert_eq!(
            without_colon.to_bytes().as_ref(),
            b"PRIVMSG #chan hi".as_ref()
        );
        assert_eq!(
            with_colon.to_bytes().as_ref(),
            b"PRIVMSG #chan :hi".as_ref()
        );
    }

    #[test]
    fn construction_via_builder_style() {
        let m = Message {
            tags: crate::Tags::new(),
            prefix: None,
            verb: Verb::word(Bytes::from_static(b"JOIN")),
            params: Params::from_iter_middle(vec![Bytes::from_static(b"#rust")]),
        };
        assert_eq!(m.to_bytes().as_ref(), b"JOIN #rust");
    }
}
