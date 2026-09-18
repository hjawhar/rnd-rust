//! Casemap-aware byte folding.
//!
//! IRC has three historical casemaps for nicknames and channel names:
//!
//! | Variant            | Folds                                                      |
//! |--------------------|------------------------------------------------------------|
//! | [`Casemap::Ascii`]         | `A`-`Z` → `a`-`z` only                              |
//! | [`Casemap::Rfc1459`]       | ASCII + `[]\\` → `{}|`                              |
//! | [`Casemap::Rfc1459Strict`] | RFC 1459 + `~` → `^`                                |
//!
//! The Scandinavian-origin `[]\\` ↔ `{}|` mapping comes from the
//! original RFC 1459. Most servers advertise `rfc1459` (without the `^`
//! pair); a handful advertise `rfc1459-strict`. Modern IRCv3 servers
//! often advertise `ascii`.

use bytes::{BufMut, Bytes, BytesMut};

/// Case-folding rules used by a server for nicknames and channel names.
///
/// Construct via [`Casemap::Ascii`], [`Casemap::Rfc1459`], or
/// [`Casemap::Rfc1459Strict`]; parse from an ISUPPORT token via
/// [`Casemap::from_token`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Casemap {
    /// Plain ASCII case folding.
    Ascii,
    /// RFC 1459 with the `[]\\` ↔ `{}|` Scandinavian extension.
    Rfc1459,
    /// RFC 1459 strict — adds `~` ↔ `^` on top of [`Casemap::Rfc1459`].
    Rfc1459Strict,
}

impl Default for Casemap {
    /// Default to [`Casemap::Rfc1459`] — what the vast majority of
    /// servers advertise.
    fn default() -> Self {
        Self::Rfc1459
    }
}

impl Casemap {
    /// Parse the value of an `ISUPPORT CASEMAPPING=` token.
    ///
    /// Returns `None` for an unrecognised token; callers can fall back
    /// to [`Casemap::default`].
    #[must_use]
    pub const fn from_token(token: &[u8]) -> Option<Self> {
        match token {
            b"ascii" => Some(Self::Ascii),
            b"rfc1459" => Some(Self::Rfc1459),
            b"rfc1459-strict" => Some(Self::Rfc1459Strict),
            _ => None,
        }
    }

    /// Render this casemap as the byte string used in ISUPPORT.
    #[must_use]
    pub const fn token(self) -> &'static [u8] {
        match self {
            Self::Ascii => b"ascii",
            Self::Rfc1459 => b"rfc1459",
            Self::Rfc1459Strict => b"rfc1459-strict",
        }
    }

    /// Fold a single byte under this casemap.
    #[must_use]
    pub const fn fold_byte(self, b: u8) -> u8 {
        let lowered = b.to_ascii_lowercase();
        match self {
            Self::Ascii => lowered,
            Self::Rfc1459 => match lowered {
                b'[' => b'{',
                b']' => b'}',
                b'\\' => b'|',
                other => other,
            },
            Self::Rfc1459Strict => match lowered {
                b'[' => b'{',
                b']' => b'}',
                b'\\' => b'|',
                b'~' => b'^',
                other => other,
            },
        }
    }

    /// Fold every byte of `src` into a fresh [`Bytes`].
    #[must_use]
    pub fn fold(self, src: &[u8]) -> Bytes {
        let mut out = BytesMut::with_capacity(src.len());
        for &b in src {
            out.put_u8(self.fold_byte(b));
        }
        out.freeze()
    }

    /// Casemap-aware byte equality without allocation.
    #[must_use]
    pub fn eq_bytes(self, a: &[u8], b: &[u8]) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(x, y)| self.fold_byte(*x) == self.fold_byte(*y))
    }
}

#[cfg(test)]
mod tests {
    use super::Casemap;

    #[test]
    fn ascii_folds_letters_only() {
        let cm = Casemap::Ascii;
        assert_eq!(cm.fold_byte(b'A'), b'a');
        assert_eq!(cm.fold_byte(b'Z'), b'z');
        assert_eq!(cm.fold_byte(b'['), b'[');
        assert_eq!(cm.fold_byte(b'~'), b'~');
    }

    #[test]
    fn rfc1459_folds_brackets_and_pipe() {
        let cm = Casemap::Rfc1459;
        assert_eq!(cm.fold_byte(b'['), b'{');
        assert_eq!(cm.fold_byte(b']'), b'}');
        assert_eq!(cm.fold_byte(b'\\'), b'|');
        assert_eq!(cm.fold_byte(b'~'), b'~'); // not folded under loose
    }

    #[test]
    fn rfc1459_strict_also_folds_tilde() {
        let cm = Casemap::Rfc1459Strict;
        assert_eq!(cm.fold_byte(b'~'), b'^');
        assert_eq!(cm.fold_byte(b'['), b'{');
    }

    #[test]
    fn eq_bytes_is_casemap_aware() {
        assert!(Casemap::Ascii.eq_bytes(b"Alice", b"alice"));
        assert!(!Casemap::Ascii.eq_bytes(b"a[b", b"a{b"));
        assert!(Casemap::Rfc1459.eq_bytes(b"a[b", b"a{b"));
        assert!(Casemap::Rfc1459.eq_bytes(b"x\\y", b"X|Y"));
    }

    #[test]
    fn fold_returns_byte_per_byte() {
        assert_eq!(Casemap::Rfc1459.fold(b"Foo[Bar]").as_ref(), b"foo{bar}");
        assert_eq!(Casemap::Ascii.fold(b"Foo[Bar]").as_ref(), b"foo[bar]");
    }

    #[test]
    fn from_token_round_trip() {
        for cm in [Casemap::Ascii, Casemap::Rfc1459, Casemap::Rfc1459Strict] {
            assert_eq!(Casemap::from_token(cm.token()), Some(cm));
        }
        assert!(Casemap::from_token(b"unknown").is_none());
    }
}
