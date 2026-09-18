//! ISUPPORT (`005`) numeric-reply parsing.
//!
//! A single 005 line looks like:
//!
//! ```text
//! :server 005 alice CHANTYPES=#& PREFIX=(ov)@+ CASEMAPPING=rfc1459 :are supported by this server
//! ```
//!
//! Every middle param (between the client nickname and the trailing
//! human-readable suffix) is a `KEY[=VALUE]` token. Tokens whose key
//! starts with `-` remove a previously-advertised key, per spec.

use bytes::Bytes;

use crate::casemap::Casemap;

/// A single ISUPPORT token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsupportToken {
    /// Token name.
    pub key: Bytes,
    /// Optional value.
    pub value: Option<Bytes>,
    /// Whether the token is a removal (leading `-`).
    pub is_removal: bool,
}

impl IsupportToken {
    /// Parse a single ISUPPORT param.
    #[must_use]
    pub fn parse(raw: &Bytes) -> Self {
        let bytes: &[u8] = raw.as_ref();
        let (is_removal, body) = if bytes.first() == Some(&b'-') {
            (true, raw.slice(1..))
        } else {
            (false, raw.clone())
        };
        let body_bytes: &[u8] = body.as_ref();
        match body_bytes.iter().position(|b| *b == b'=') {
            Some(pos) => Self {
                key: body.slice(..pos),
                value: Some(body.slice(pos + 1..)),
                is_removal,
            },
            None => Self {
                key: body,
                value: None,
                is_removal,
            },
        }
    }
}

/// Aggregate view of ISUPPORT tokens the server has advertised so far.
///
/// Callers typically keep a single [`Isupport`] per server connection
/// and feed it every 005 line via [`Isupport::merge`].
#[derive(Debug, Default, Clone)]
pub struct Isupport {
    tokens: Vec<IsupportToken>,
}

impl Isupport {
    /// Construct an empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge every parsed token from a 005 reply.
    pub fn merge<I>(&mut self, tokens: I)
    where
        I: IntoIterator<Item = IsupportToken>,
    {
        for tok in tokens {
            if tok.is_removal {
                self.tokens.retain(|t| t.key != tok.key);
            } else if let Some(existing) = self.tokens.iter_mut().find(|t| t.key == tok.key) {
                existing.value = tok.value;
            } else {
                self.tokens.push(tok);
            }
        }
    }

    /// Look up a token by key (byte-exact match).
    #[must_use]
    pub fn get(&self, key: &[u8]) -> Option<&IsupportToken> {
        self.tokens.iter().find(|t| t.key.as_ref() == key)
    }

    /// Iterate over the currently-active tokens.
    pub fn iter(&self) -> std::slice::Iter<'_, IsupportToken> {
        self.tokens.iter()
    }

    /// Resolve the `CASEMAPPING=` token to a [`Casemap`]. Returns the
    /// default ([`Casemap::Rfc1459`]) when the token is absent or
    /// unrecognised.
    #[must_use]
    pub fn casemap(&self) -> Casemap {
        self.get(b"CASEMAPPING")
            .and_then(|t| t.value.as_ref())
            .and_then(|v| Casemap::from_token(v.as_ref()))
            .unwrap_or_default()
    }

    /// Resolve the `NICKLEN=` token to a `usize`. Returns `None` when
    /// absent or unparsable.
    #[must_use]
    pub fn nicklen(&self) -> Option<usize> {
        self.get(b"NICKLEN")
            .and_then(|t| t.value.as_ref())
            .and_then(|v| std::str::from_utf8(v.as_ref()).ok())
            .and_then(|s| s.parse().ok())
    }

    /// Resolve the `CHANNELLEN=` token to a `usize`.
    #[must_use]
    pub fn channellen(&self) -> Option<usize> {
        self.get(b"CHANNELLEN")
            .and_then(|t| t.value.as_ref())
            .and_then(|v| std::str::from_utf8(v.as_ref()).ok())
            .and_then(|s| s.parse().ok())
    }
}

impl<'a> IntoIterator for &'a Isupport {
    type Item = &'a IsupportToken;
    type IntoIter = std::slice::Iter<'a, IsupportToken>;

    fn into_iter(self) -> Self::IntoIter {
        self.tokens.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::{Isupport, IsupportToken};
    use crate::casemap::Casemap;
    use bytes::Bytes;

    fn tok(raw: &str) -> IsupportToken {
        IsupportToken::parse(&Bytes::copy_from_slice(raw.as_bytes()))
    }

    #[test]
    fn parses_name_only() {
        let t = tok("NETWORK");
        assert_eq!(t.key.as_ref(), b"NETWORK");
        assert!(t.value.is_none());
        assert!(!t.is_removal);
    }

    #[test]
    fn parses_value_bearing_token() {
        let t = tok("CASEMAPPING=rfc1459");
        assert_eq!(t.key.as_ref(), b"CASEMAPPING");
        assert_eq!(t.value.as_deref(), Some(&b"rfc1459"[..]));
    }

    #[test]
    fn parses_removal() {
        let t = tok("-AWAYLEN");
        assert!(t.is_removal);
        assert_eq!(t.key.as_ref(), b"AWAYLEN");
    }

    #[test]
    fn isupport_merge_overrides_duplicates() {
        let mut s = Isupport::new();
        s.merge([tok("NICKLEN=16")]);
        s.merge([tok("NICKLEN=32")]);
        assert_eq!(s.nicklen(), Some(32));
    }

    #[test]
    fn isupport_removal_drops_token() {
        let mut s = Isupport::new();
        s.merge([tok("CHANNELLEN=50")]);
        assert_eq!(s.channellen(), Some(50));
        s.merge([tok("-CHANNELLEN")]);
        assert_eq!(s.channellen(), None);
    }

    #[test]
    fn casemap_accessor_falls_back_to_rfc1459() {
        let mut s = Isupport::new();
        assert_eq!(s.casemap(), Casemap::Rfc1459);
        s.merge([tok("CASEMAPPING=ascii")]);
        assert_eq!(s.casemap(), Casemap::Ascii);
        s.merge([tok("CASEMAPPING=rfc1459-strict")]);
        assert_eq!(s.casemap(), Casemap::Rfc1459Strict);
    }
}
