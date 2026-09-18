//! Command verb: either an alphabetic word or a three-digit numeric.
//!
//! Grammar per RFC 1459 / 2812:
//!
//! ```text
//! command = 1*letter / 3digit
//! ```
//!
//! The parser enforces the exact shape; everything else is rejected.

use bytes::{BufMut, Bytes, BytesMut};

use crate::error::ParseError;

/// Command verb slot of a [`crate::Message`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verb {
    /// Three-digit reply code (`000`-`999`).
    Numeric(u16),
    /// Alphabetic command word, uppercased by convention but not by this
    /// parser (the wire bytes are preserved verbatim).
    Word(Bytes),
}

impl Verb {
    /// Construct a word verb from raw bytes.
    #[must_use]
    pub fn word(bytes: impl Into<Bytes>) -> Self {
        Self::Word(bytes.into())
    }

    /// Construct a numeric verb. Values outside `0..=999` are still
    /// representable; serialization will pad to three digits with
    /// leading zeros and truncate the high digits if needed.
    #[must_use]
    pub const fn numeric(code: u16) -> Self {
        Self::Numeric(code)
    }
}

/// Parse the verb starting at `*pos`. On success, `*pos` is left pointing
/// either at the mandatory space that precedes the params list or at the
/// end of input (no params).
pub(crate) fn parse_verb(buf: &Bytes, pos: &mut usize) -> Result<Verb, ParseError> {
    let data = buf.as_ref();
    let start = *pos;
    if start >= data.len() {
        return Err(ParseError::UnexpectedEnd);
    }
    let first = data[start];
    // Three-digit numeric path.
    if first.is_ascii_digit() {
        if data.len() < start + 3
            || !data[start + 1].is_ascii_digit()
            || !data[start + 2].is_ascii_digit()
        {
            return Err(ParseError::InvalidCommand(start));
        }
        // After three digits the next byte (if any) must be space.
        let after = start + 3;
        if let Some(&b) = data.get(after) {
            if b != b' ' {
                return Err(ParseError::InvalidCommand(start));
            }
        }
        let code = u16::from(data[start] - b'0') * 100
            + u16::from(data[start + 1] - b'0') * 10
            + u16::from(data[start + 2] - b'0');
        *pos = after;
        return Ok(Verb::Numeric(code));
    }
    // Alphabetic word path.
    if !first.is_ascii_alphabetic() {
        return Err(ParseError::InvalidCommand(start));
    }
    let mut i = start;
    while i < data.len() && data[i].is_ascii_alphabetic() {
        i += 1;
    }
    // After the word, either end-of-input or space.
    if let Some(&b) = data.get(i) {
        if b != b' ' {
            return Err(ParseError::InvalidCommand(start));
        }
    }
    let word = buf.slice(start..i);
    *pos = i;
    Ok(Verb::Word(word))
}

/// Serialize `verb` into `out`.
pub(crate) fn write_verb(verb: &Verb, out: &mut BytesMut) {
    match verb {
        Verb::Word(w) => out.extend_from_slice(w.as_ref()),
        Verb::Numeric(code) => {
            let clamped = code % 1000;
            let h = (clamped / 100) as u8;
            let t = ((clamped / 10) % 10) as u8;
            let u = (clamped % 10) as u8;
            out.put_u8(b'0' + h);
            out.put_u8(b'0' + t);
            out.put_u8(b'0' + u);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Verb, parse_verb, write_verb};
    use bytes::{Bytes, BytesMut};

    fn parse(s: &str) -> Verb {
        let buf = Bytes::copy_from_slice(s.as_bytes());
        let mut pos = 0;
        parse_verb(&buf, &mut pos).expect("parse_verb")
    }

    fn round_trip_verb(v: &Verb) -> String {
        let mut out = BytesMut::new();
        write_verb(v, &mut out);
        String::from_utf8(out.to_vec()).unwrap()
    }

    #[test]
    fn parses_alphabetic_word_at_end_of_input() {
        assert_eq!(parse("PRIVMSG"), Verb::Word(Bytes::from_static(b"PRIVMSG")));
    }

    #[test]
    fn parses_alphabetic_word_followed_by_space() {
        assert_eq!(parse("JOIN "), Verb::Word(Bytes::from_static(b"JOIN")));
    }

    #[test]
    fn parses_numeric_at_end() {
        assert_eq!(parse("001"), Verb::Numeric(1));
    }

    #[test]
    fn parses_numeric_followed_by_space() {
        assert_eq!(parse("353 "), Verb::Numeric(353));
    }

    #[test]
    fn rejects_mixed_numeric() {
        use crate::error::ParseError;
        let buf = Bytes::copy_from_slice(b"12X ");
        let mut pos = 0;
        assert!(matches!(
            super::parse_verb(&buf, &mut pos),
            Err(ParseError::InvalidCommand(0))
        ));
    }

    #[test]
    fn rejects_digit_shorter_than_three() {
        use crate::error::ParseError;
        let buf = Bytes::copy_from_slice(b"12");
        let mut pos = 0;
        assert!(matches!(
            super::parse_verb(&buf, &mut pos),
            Err(ParseError::InvalidCommand(0))
        ));
    }

    #[test]
    fn rejects_alphanumeric_word() {
        use crate::error::ParseError;
        let buf = Bytes::copy_from_slice(b"PRIV1 ");
        let mut pos = 0;
        assert!(matches!(
            super::parse_verb(&buf, &mut pos),
            Err(ParseError::InvalidCommand(0))
        ));
    }

    #[test]
    fn numerics_pad_to_three_digits() {
        assert_eq!(round_trip_verb(&Verb::Numeric(1)), "001");
        assert_eq!(round_trip_verb(&Verb::Numeric(42)), "042");
        assert_eq!(round_trip_verb(&Verb::Numeric(999)), "999");
    }
}
