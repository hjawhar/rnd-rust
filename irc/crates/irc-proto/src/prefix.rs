//! Prefix: the optional origin token before the command word.
//!
//! Grammar per RFC 2812:
//!
//! ```text
//! prefix = servername / ( nickname [ [ "!" user ] "@" host ] )
//! ```
//!
//! Server names and bare nicks are not syntactically distinguishable in
//! every case. We classify based on the presence of `!`, `@`, and `.`:
//!
//! - Contains `!` or `@` → [`Prefix::User`].
//! - Otherwise, contains `.` → [`Prefix::Server`].
//! - Otherwise → [`Prefix::User`] with `user` and `host` both `None`.

use bytes::{BufMut, Bytes, BytesMut};

use crate::error::ParseError;
use crate::util::is_forbidden;

/// Origin of a message as carried in the wire prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prefix {
    /// Server origin (e.g. `irc.example.net`).
    Server(Bytes),
    /// User origin. `nick` is always present; `user` and `host` appear
    /// when the wire form included `!user` or `@host`.
    User {
        /// Nickname portion.
        nick: Bytes,
        /// `user` portion (after `!`), if present.
        user: Option<Bytes>,
        /// Host portion (after `@`), if present.
        host: Option<Bytes>,
    },
}

impl Prefix {
    /// Build a server-origin prefix.
    #[must_use]
    pub fn server(name: impl Into<Bytes>) -> Self {
        Self::Server(name.into())
    }

    /// Build a user-origin prefix with optional user + host.
    #[must_use]
    pub fn user(nick: impl Into<Bytes>, user: Option<Bytes>, host: Option<Bytes>) -> Self {
        Self::User {
            nick: nick.into(),
            user,
            host,
        }
    }
}

/// Parse a prefix starting at `*pos`, which must point at the first byte
/// after the leading `:`. On success, `*pos` is left pointing at the
/// mandatory space separating the prefix from the command.
pub(crate) fn parse_prefix(buf: &Bytes, pos: &mut usize) -> Result<Prefix, ParseError> {
    let start = *pos;
    let data = buf.as_ref();
    let mut i = start;
    while i < data.len() && data[i] != b' ' {
        let b = data[i];
        if is_forbidden(b) {
            return Err(ParseError::Forbidden(i));
        }
        i += 1;
    }
    if i == start {
        return Err(ParseError::InvalidPrefix(start));
    }
    let raw = buf.slice(start..i);
    *pos = i;
    Ok(classify(raw))
}

fn classify(raw: Bytes) -> Prefix {
    let bang = raw.iter().position(|b| *b == b'!');
    let at = raw.iter().position(|b| *b == b'@');
    match (bang, at) {
        (Some(bi), Some(ai)) if bi < ai => {
            let nick = raw.slice(0..bi);
            let user = raw.slice(bi + 1..ai);
            let host = raw.slice(ai + 1..raw.len());
            Prefix::User {
                nick,
                user: Some(user),
                host: Some(host),
            }
        }
        (Some(bi), None) => {
            let nick = raw.slice(0..bi);
            let user = raw.slice(bi + 1..raw.len());
            Prefix::User {
                nick,
                user: Some(user),
                host: None,
            }
        }
        (None, Some(ai)) => {
            let nick = raw.slice(0..ai);
            let host = raw.slice(ai + 1..raw.len());
            Prefix::User {
                nick,
                user: None,
                host: Some(host),
            }
        }
        _ => {
            if raw.contains(&b'.') {
                Prefix::Server(raw)
            } else {
                Prefix::User {
                    nick: raw,
                    user: None,
                    host: None,
                }
            }
        }
    }
}

/// Serialize `prefix` into `out`, prefixed by `:` and followed by a
/// single trailing space.
pub(crate) fn write_prefix(prefix: &Prefix, out: &mut BytesMut) {
    out.put_u8(b':');
    match prefix {
        Prefix::Server(name) => out.extend_from_slice(name.as_ref()),
        Prefix::User { nick, user, host } => {
            out.extend_from_slice(nick.as_ref());
            if let Some(u) = user {
                out.put_u8(b'!');
                out.extend_from_slice(u.as_ref());
            }
            if let Some(h) = host {
                out.put_u8(b'@');
                out.extend_from_slice(h.as_ref());
            }
        }
    }
    out.put_u8(b' ');
}

#[cfg(test)]
mod tests {
    use super::{Prefix, parse_prefix, write_prefix};
    use bytes::{Bytes, BytesMut};

    fn parse(s: &str) -> Prefix {
        let buf = Bytes::copy_from_slice(s.as_bytes());
        assert_eq!(buf[0], b':');
        let mut pos = 1;
        parse_prefix(&buf, &mut pos).expect("parse_prefix")
    }

    fn round_trip(s: &str) -> String {
        let p = parse(s);
        let mut out = BytesMut::new();
        write_prefix(&p, &mut out);
        String::from_utf8(out.to_vec()).expect("utf-8")
    }

    #[test]
    fn server_with_dots() {
        let p = parse(":irc.example.net ");
        assert_eq!(p, Prefix::Server(Bytes::from_static(b"irc.example.net")));
    }

    #[test]
    fn user_with_bang_and_at() {
        let p = parse(":alice!~al@host.example.com ");
        assert_eq!(
            p,
            Prefix::User {
                nick: Bytes::from_static(b"alice"),
                user: Some(Bytes::from_static(b"~al")),
                host: Some(Bytes::from_static(b"host.example.com")),
            }
        );
    }

    #[test]
    fn user_at_only() {
        let p = parse(":alice@host ");
        assert_eq!(
            p,
            Prefix::User {
                nick: Bytes::from_static(b"alice"),
                user: None,
                host: Some(Bytes::from_static(b"host")),
            }
        );
    }

    #[test]
    fn bare_nick_without_dot() {
        let p = parse(":alice ");
        assert_eq!(
            p,
            Prefix::User {
                nick: Bytes::from_static(b"alice"),
                user: None,
                host: None,
            }
        );
    }

    #[test]
    fn round_trip_server() {
        assert_eq!(round_trip(":irc.example.net "), ":irc.example.net ");
    }

    #[test]
    fn round_trip_full_user() {
        assert_eq!(
            round_trip(":alice!~al@host.example.com "),
            ":alice!~al@host.example.com "
        );
    }
}
