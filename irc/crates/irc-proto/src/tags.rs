//! IRCv3 message tags.
//!
//! Wire grammar (IRCv3 `message-tags` spec):
//!
//! ```text
//! tags     = tag *(";" tag)
//! tag      = key ["=" escaped-value]
//! key      = [client-prefix] [vendor "/"] key-name
//! ```
//!
//! Values are escape-decoded on parse: `\:` → `;`, `\s` → space, `\\` →
//! `\`, `\r` → CR, `\n` → LF. An unrecognised escape is treated
//! liberally: the backslash is dropped and the following byte (if any)
//! is taken verbatim — matching the spec's MUST-accept rule for unknown
//! escapes. Values are escape-encoded on serialize using the inverse
//! table.

use bytes::{BufMut, Bytes, BytesMut};
use smallvec::SmallVec;

use crate::error::ParseError;
use crate::util::{is_forbidden, is_tag_key_byte, is_tag_value_byte};

/// Ordered set of [`Tag`]s attached to a [`crate::Message`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tags(pub(crate) SmallVec<[Tag; 4]>);

impl Tags {
    /// Construct an empty tag set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when the set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of tags present.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Iterate over the tags in insertion order.
    pub fn iter(&self) -> std::slice::Iter<'_, Tag> {
        self.0.iter()
    }

    /// Find a tag whose [`TagKey::name`] matches `name` byte-for-byte.
    #[must_use]
    pub fn get(&self, name: &[u8]) -> Option<&Tag> {
        self.0.iter().find(|t| t.key.name.as_ref() == name)
    }

    /// Append a tag.
    pub fn push(&mut self, tag: Tag) {
        self.0.push(tag);
    }
}

impl<'a> IntoIterator for &'a Tags {
    type Item = &'a Tag;
    type IntoIter = std::slice::Iter<'a, Tag>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// A single message tag: key plus optional value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// Key portion (may include a vendor prefix and client-only flag).
    pub key: TagKey,
    /// Value portion, already un-escaped. `None` indicates the wire form
    /// had no `=`; an empty `Some(Bytes::new())` indicates `key=` with no
    /// value bytes — different semantics, both legal.
    pub value: Option<Bytes>,
}

/// Tag key: optional client-only `+` flag plus a name (which may itself
/// contain a `vendor/` prefix).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagKey {
    /// Whether the wire form started with a `+` (client-only tag).
    pub client_only: bool,
    /// Name bytes: either `key-name` or `vendor/key-name`.
    pub name: Bytes,
}

impl TagKey {
    /// Return the vendor portion (everything before the last `/`), if any.
    #[must_use]
    pub fn vendor(&self) -> Option<&[u8]> {
        let bytes: &[u8] = self.name.as_ref();
        bytes.iter().rposition(|b| *b == b'/').map(|i| &bytes[..i])
    }

    /// Return the short key-name (everything after the last `/`, or the
    /// whole name when there is no vendor).
    #[must_use]
    pub fn short_name(&self) -> &[u8] {
        let bytes: &[u8] = self.name.as_ref();
        bytes
            .iter()
            .rposition(|b| *b == b'/')
            .map_or(bytes, |i| &bytes[i + 1..])
    }
}

/// Parse the tags region starting at `*pos`, which must point at the
/// first byte after the leading `@`. On success, `*pos` is left pointing
/// at the mandatory space separating tags from the rest of the message.
pub(crate) fn parse_tags(buf: &Bytes, pos: &mut usize) -> Result<Tags, ParseError> {
    let data = buf.as_ref();
    let mut out = Tags::new();
    loop {
        let (client_only, name_start, name_end) = parse_key(data, *pos)?;
        *pos = name_end;
        let name = buf.slice(name_start..name_end);
        let value = if data.get(*pos) == Some(&b'=') {
            *pos += 1;
            let (v, after) = parse_value(buf, *pos)?;
            *pos = after;
            Some(v)
        } else {
            None
        };
        out.push(Tag {
            key: TagKey { client_only, name },
            value,
        });
        match data.get(*pos) {
            Some(&b';') => {
                *pos += 1;
            }
            Some(&b' ') => return Ok(out),
            Some(_) | None => return Err(ParseError::InvalidTagKey(*pos)),
        }
    }
}

/// Parse a single key. Returns `(client_only, name_start, name_end)`
/// where the slice `data[name_start..name_end]` is the key name.
fn parse_key(data: &[u8], start: usize) -> Result<(bool, usize, usize), ParseError> {
    let mut i = start;
    let client_only = data.get(i) == Some(&b'+');
    if client_only {
        i += 1;
    }
    let name_start = i;
    while i < data.len() && is_tag_key_byte(data[i]) {
        i += 1;
    }
    if i == name_start {
        return Err(ParseError::InvalidTagKey(start));
    }
    Ok((client_only, name_start, i))
}

fn parse_value(buf: &Bytes, start: usize) -> Result<(Bytes, usize), ParseError> {
    let data = buf.as_ref();
    let mut out = BytesMut::new();
    let mut i = start;
    while i < data.len() {
        let b = data[i];
        if b == b';' || b == b' ' {
            break;
        }
        if b == b'\\' {
            match data.get(i + 1) {
                Some(&b':') => {
                    out.put_u8(b';');
                    i += 2;
                }
                Some(&b's') => {
                    out.put_u8(b' ');
                    i += 2;
                }
                Some(&b'\\') => {
                    out.put_u8(b'\\');
                    i += 2;
                }
                Some(&b'r') => {
                    out.put_u8(b'\r');
                    i += 2;
                }
                Some(&b'n') => {
                    out.put_u8(b'\n');
                    i += 2;
                }
                Some(&c) => {
                    // Unknown escape: accept the following byte verbatim.
                    out.put_u8(c);
                    i += 2;
                }
                None => {
                    // Trailing backslash: drop silently per spec.
                    i += 1;
                }
            }
            continue;
        }
        if is_forbidden(b) || !is_tag_value_byte(b) {
            return Err(ParseError::InvalidTagEscape(i));
        }
        out.put_u8(b);
        i += 1;
    }
    Ok((out.freeze(), i))
}

/// Serialize `tags` into `out`, prefixed by `@` and followed by a single
/// space iff the set is non-empty. Values are escape-encoded per spec.
pub(crate) fn write_tags(tags: &Tags, out: &mut BytesMut) {
    if tags.is_empty() {
        return;
    }
    out.put_u8(b'@');
    for (i, tag) in tags.iter().enumerate() {
        if i > 0 {
            out.put_u8(b';');
        }
        if tag.key.client_only {
            out.put_u8(b'+');
        }
        out.extend_from_slice(tag.key.name.as_ref());
        if let Some(v) = &tag.value {
            out.put_u8(b'=');
            write_escaped_value(v.as_ref(), out);
        }
    }
    out.put_u8(b' ');
}

fn write_escaped_value(v: &[u8], out: &mut BytesMut) {
    for &b in v {
        match b {
            b';' => out.extend_from_slice(b"\\:"),
            b' ' => out.extend_from_slice(b"\\s"),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\n' => out.extend_from_slice(b"\\n"),
            other => out.put_u8(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Tag, TagKey, Tags, parse_tags, write_tags};
    use bytes::{Bytes, BytesMut};

    fn parse(s: &str) -> Tags {
        let buf = Bytes::copy_from_slice(s.as_bytes());
        assert_eq!(buf[0], b'@', "test input must start with @");
        let mut pos = 1;
        parse_tags(&buf, &mut pos).expect("parse_tags")
    }

    fn round_trip(s: &str) -> String {
        let t = parse(s);
        let mut out = BytesMut::new();
        write_tags(&t, &mut out);
        String::from_utf8(out.to_vec()).expect("utf-8")
    }

    #[test]
    fn single_tag_without_value() {
        let t = parse("@tag ");
        assert_eq!(t.len(), 1);
        let only = &t.0[0];
        assert!(!only.key.client_only);
        assert_eq!(only.key.name.as_ref(), b"tag");
        assert!(only.value.is_none());
    }

    #[test]
    fn tag_with_empty_value() {
        let t = parse("@tag= ");
        let only = &t.0[0];
        assert_eq!(only.value.as_deref(), Some(&[][..]));
    }

    #[test]
    fn tag_value_unescapes_all_specials() {
        let t = parse(r"@k=a\:b\sc\\d\re\nf ");
        let only = &t.0[0];
        assert_eq!(only.value.as_deref(), Some(&b"a;b c\\d\re\nf"[..]));
    }

    #[test]
    fn client_only_and_vendor_key() {
        let t = parse("@+example.com/my-tag=v ");
        let only = &t.0[0];
        assert!(only.key.client_only);
        assert_eq!(only.key.name.as_ref(), b"example.com/my-tag");
        assert_eq!(only.key.vendor(), Some(&b"example.com"[..]));
        assert_eq!(only.key.short_name(), b"my-tag");
    }

    #[test]
    fn multiple_tags_semicolon_separated() {
        let t = parse("@a=1;b;c=3 ");
        assert_eq!(t.len(), 3);
        assert_eq!(t.0[0].key.name.as_ref(), b"a");
        assert_eq!(t.0[0].value.as_deref(), Some(&b"1"[..]));
        assert_eq!(t.0[1].key.name.as_ref(), b"b");
        assert!(t.0[1].value.is_none());
        assert_eq!(t.0[2].key.name.as_ref(), b"c");
    }

    #[test]
    fn round_trip_escape_identity() {
        let s = r"@k=a\:b\sc\\d\re\nf ";
        assert_eq!(round_trip(s), s);
    }

    #[test]
    fn round_trip_multiple_tags() {
        let s = "@x=1;y;+vend.or/z=value ";
        assert_eq!(round_trip(s), s);
    }

    #[test]
    fn tag_lookup_by_name() {
        let t = Tags(
            vec![
                Tag {
                    key: TagKey {
                        client_only: false,
                        name: Bytes::from_static(b"foo"),
                    },
                    value: Some(Bytes::from_static(b"1")),
                },
                Tag {
                    key: TagKey {
                        client_only: false,
                        name: Bytes::from_static(b"bar"),
                    },
                    value: None,
                },
            ]
            .into(),
        );
        assert!(t.get(b"foo").is_some());
        assert!(t.get(b"bar").is_some());
        assert!(t.get(b"baz").is_none());
    }
}
