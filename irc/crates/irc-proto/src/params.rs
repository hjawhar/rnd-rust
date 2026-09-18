//! Params: the list of arguments following the command verb.
//!
//! Grammar per RFC 2812:
//!
//! ```text
//! params   = *14( SPACE middle ) [ SPACE ":" trailing ]
//!          / 14( SPACE middle ) [ SPACE [ ":" ] trailing ]
//!
//! middle   = nospcrlfcl *( ":" / nospcrlfcl )
//! trailing = *( ":" / " " / nospcrlfcl )
//! ```
//!
//! `Params` stores the values flat; a boolean flag records whether the
//! last value was carried as `trailing` on the wire so [`Params::write`]
//! can reproduce the exact byte sequence that was parsed.

use bytes::{BufMut, Bytes, BytesMut};
use smallvec::SmallVec;

use crate::error::ParseError;
use crate::limits::MAX_PARAMS;
use crate::util::{is_forbidden, is_middle_first, is_middle_rest};

/// List of params attached to a [`crate::Message`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Params {
    values: SmallVec<[Bytes; 8]>,
    /// Whether the last value was received in `trailing` form (prefixed
    /// with `:` on the wire, or — when 15 params are present — as the
    /// implicit trailing after 14 middles).
    trailing: bool,
}

impl Params {
    /// Construct an empty `Params`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct `Params` from an iterator of middle-form values.
    ///
    /// No `trailing` flag is set; call [`Params::with_trailing`] to mark
    /// the last value as explicit trailing.
    pub fn from_iter_middle<I, B>(iter: I) -> Self
    where
        I: IntoIterator<Item = B>,
        B: Into<Bytes>,
    {
        Self {
            values: iter.into_iter().map(Into::into).collect(),
            trailing: false,
        }
    }

    /// Mark the last value as explicit trailing.
    #[must_use]
    pub const fn with_trailing(mut self, trailing: bool) -> Self {
        self.trailing = trailing;
        self
    }

    /// Returns `true` when no values are present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Number of values.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Return the value at `index`, if present.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Bytes> {
        self.values.get(index)
    }

    /// Iterate over every value in order.
    pub fn iter(&self) -> std::slice::Iter<'_, Bytes> {
        self.values.iter()
    }

    /// Whether the last value is carried in trailing form.
    #[must_use]
    pub const fn is_trailing(&self) -> bool {
        self.trailing
    }

    /// Append a middle-form value.
    pub fn push(&mut self, value: impl Into<Bytes>) {
        self.values.push(value.into());
    }

    /// Append and mark the supplied value as trailing. Clears the
    /// `trailing` flag first if needed so only the pushed value is
    /// trailing.
    pub fn push_trailing(&mut self, value: impl Into<Bytes>) {
        self.values.push(value.into());
        self.trailing = true;
    }

    /// Last value, if any.
    #[must_use]
    pub fn last(&self) -> Option<&Bytes> {
        self.values.last()
    }
}

impl std::ops::Index<usize> for Params {
    type Output = Bytes;
    fn index(&self, index: usize) -> &Self::Output {
        &self.values[index]
    }
}

impl<'a> IntoIterator for &'a Params {
    type Item = &'a Bytes;
    type IntoIter = std::slice::Iter<'a, Bytes>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

/// Parse the params region starting at `*pos`.
///
/// `*pos` must point either at the space that precedes the first param
/// or at the end of input (empty params). On success, `*pos` is left at
/// end of input.
pub(crate) fn parse_params(buf: &Bytes, pos: &mut usize) -> Result<Params, ParseError> {
    let data = buf.as_ref();
    let mut out = Params::new();
    loop {
        if *pos >= data.len() {
            return Ok(out);
        }
        if data[*pos] != b' ' {
            // Caller didn't honour the "space before next param" rule.
            return Err(ParseError::MissingSpace(*pos));
        }
        // Skip the single mandatory space. The spec is strict about a
        // single space here; accepting runs would subtly change
        // serialization invariants, so we reject.
        *pos += 1;
        if *pos >= data.len() {
            // Trailing space with no param is a malformed message.
            return Err(ParseError::UnexpectedEnd);
        }
        if out.values.len() > MAX_PARAMS {
            return Err(ParseError::TooManyParams(*pos));
        }
        let first = data[*pos];
        if first == b':' {
            // Explicit trailing.
            *pos += 1;
            let value_start = *pos;
            while *pos < data.len() {
                if is_forbidden(data[*pos]) {
                    return Err(ParseError::Forbidden(*pos));
                }
                *pos += 1;
            }
            out.values.push(buf.slice(value_start..*pos));
            out.trailing = true;
            return Ok(out);
        }
        if out.values.len() == 14 {
            // 15th param without leading `:` — still trailing per spec.
            let value_start = *pos;
            while *pos < data.len() {
                if is_forbidden(data[*pos]) {
                    return Err(ParseError::Forbidden(*pos));
                }
                *pos += 1;
            }
            out.values.push(buf.slice(value_start..*pos));
            out.trailing = true;
            return Ok(out);
        }
        if !is_middle_first(first) {
            return Err(ParseError::Forbidden(*pos));
        }
        let value_start = *pos;
        while *pos < data.len() && is_middle_rest(data[*pos]) {
            *pos += 1;
        }
        if *pos < data.len() && is_forbidden(data[*pos]) {
            return Err(ParseError::Forbidden(*pos));
        }
        out.values.push(buf.slice(value_start..*pos));
    }
}

/// Serialize `params` into `out`. Each value is preceded by a single
/// space. The last value is written in trailing form (leading `:`) when
/// the [`Params::trailing`] flag is set, or automatically when the value
/// is empty, starts with `:`, or contains a space.
pub(crate) fn write_params(params: &Params, out: &mut BytesMut) {
    let last_idx = params.values.len().saturating_sub(1);
    for (i, v) in params.values.iter().enumerate() {
        out.put_u8(b' ');
        let bytes: &[u8] = v.as_ref();
        let is_last = i == last_idx;
        let needs_trailing = is_last
            && (params.trailing
                || bytes.is_empty()
                || bytes.first() == Some(&b':')
                || bytes.contains(&b' '));
        if needs_trailing {
            out.put_u8(b':');
        }
        out.extend_from_slice(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::{Params, parse_params, write_params};
    use bytes::{Bytes, BytesMut};

    fn parse(s: &str) -> Params {
        let buf = Bytes::copy_from_slice(s.as_bytes());
        let mut pos = 0;
        parse_params(&buf, &mut pos).expect("parse_params")
    }

    fn round_trip(s: &str) -> String {
        let p = parse(s);
        let mut out = BytesMut::new();
        write_params(&p, &mut out);
        String::from_utf8(out.to_vec()).unwrap()
    }

    #[test]
    fn no_params() {
        let p = parse("");
        assert!(p.is_empty());
    }

    #[test]
    fn single_middle() {
        let p = parse(" foo");
        assert_eq!(p.len(), 1);
        assert_eq!(&*p[0], b"foo");
        assert!(!p.is_trailing());
    }

    #[test]
    fn two_middles() {
        let p = parse(" a b");
        assert_eq!(p.len(), 2);
        assert_eq!(&*p[0], b"a");
        assert_eq!(&*p[1], b"b");
        assert!(!p.is_trailing());
    }

    #[test]
    fn trailing_only() {
        let p = parse(" :hello world");
        assert_eq!(p.len(), 1);
        assert_eq!(&*p[0], b"hello world");
        assert!(p.is_trailing());
    }

    #[test]
    fn empty_trailing() {
        let p = parse(" :");
        assert_eq!(p.len(), 1);
        assert_eq!(&*p[0], b"");
        assert!(p.is_trailing());
    }

    #[test]
    fn middle_then_trailing() {
        let p = parse(" #chan :hello world");
        assert_eq!(p.len(), 2);
        assert_eq!(&*p[0], b"#chan");
        assert_eq!(&*p[1], b"hello world");
        assert!(p.is_trailing());
    }

    #[test]
    fn trailing_may_contain_colon() {
        let p = parse(" :foo:bar:baz");
        assert_eq!(&*p[0], b"foo:bar:baz");
    }

    #[test]
    fn middle_may_contain_colon_after_first() {
        let p = parse(" a:b");
        assert_eq!(&*p[0], b"a:b");
    }

    #[test]
    fn middle_starting_with_colon_is_always_trailing() {
        // `:foo` at a middle position is by definition trailing.
        let p = parse(" :foo");
        assert!(p.is_trailing());
        assert_eq!(&*p[0], b"foo");
    }

    #[test]
    fn fifteenth_param_is_implicit_trailing() {
        // 14 middles + 15th without leading colon, per RFC 2812.
        let middles: Vec<String> = (0..14).map(|i| format!("a{i}")).collect();
        let mut line = String::new();
        for m in &middles {
            line.push(' ');
            line.push_str(m);
        }
        line.push(' ');
        line.push_str("hello world"); // no colon, has space — implicit trailing
        let p = parse(&line);
        assert_eq!(p.len(), 15);
        assert!(p.is_trailing());
        assert_eq!(&*p[14], b"hello world");
    }

    #[test]
    fn round_trip_explicit_trailing() {
        assert_eq!(round_trip(" :hello world"), " :hello world");
        assert_eq!(round_trip(" #chan :text"), " #chan :text");
    }

    #[test]
    fn round_trip_implicit_trailing_for_space_values() {
        // Values that contain spaces always serialize as trailing even
        // without the flag being explicitly set (we auto-promote).
        let mut p = Params::new();
        p.push("hello world");
        let mut out = BytesMut::new();
        write_params(&p, &mut out);
        assert_eq!(&out[..], b" :hello world");
    }

    #[test]
    fn round_trip_empty_value_forces_colon() {
        let mut p = Params::new();
        p.push("");
        let mut out = BytesMut::new();
        write_params(&p, &mut out);
        assert_eq!(&out[..], b" :");
    }

    #[test]
    fn rejects_nul() {
        use crate::error::ParseError;
        let buf = Bytes::copy_from_slice(b" a\0b");
        let mut pos = 0;
        assert!(matches!(
            parse_params(&buf, &mut pos),
            Err(ParseError::Forbidden(_))
        ));
    }
}
