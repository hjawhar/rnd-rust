//! IRCv3 capability-list parsing.
//!
//! Works on the `args` [`Vec<Bytes>`](bytes::Bytes) returned inside a
//! [`crate::Command::Cap`]. The cap payload itself is the **last**
//! argument of a typical CAP response; for a multi-line LS reply the
//! second-to-last argument is `*` to indicate continuation.

use bytes::Bytes;

/// A single capability token: `name` plus optional `value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapToken {
    /// Capability name (e.g. `sasl`, `message-tags`).
    pub name: Bytes,
    /// Optional value (e.g. `PLAIN,EXTERNAL` for `sasl`).
    pub value: Option<Bytes>,
}

/// Split a capability payload (space-separated) into tokens.
#[must_use]
pub fn parse_cap_list(payload: &Bytes) -> Vec<CapToken> {
    let bytes: &[u8] = payload.as_ref();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b' ' {
            if i > start {
                out.push(parse_token(payload.slice(start..i)));
            }
            start = i + 1;
        }
        i += 1;
    }
    if i > start {
        out.push(parse_token(payload.slice(start..i)));
    }
    out
}

fn parse_token(buf: Bytes) -> CapToken {
    match buf.iter().position(|b| *b == b'=') {
        Some(pos) => CapToken {
            name: buf.slice(..pos),
            value: Some(buf.slice(pos + 1..)),
        },
        None => CapToken {
            name: buf,
            value: None,
        },
    }
}

/// Detect the multi-line LS continuation marker (a single `*` arg
/// immediately preceding the payload in a `CAP LS` reply).
#[must_use]
pub fn is_continuation_marker(arg: &Bytes) -> bool {
    arg.as_ref() == b"*"
}

#[cfg(test)]
mod tests {
    use super::{CapToken, is_continuation_marker, parse_cap_list};
    use bytes::Bytes;

    #[test]
    fn parses_name_only_tokens() {
        let payload = Bytes::from_static(b"message-tags server-time");
        let toks = parse_cap_list(&payload);
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].name.as_ref(), b"message-tags");
        assert!(toks[0].value.is_none());
        assert_eq!(toks[1].name.as_ref(), b"server-time");
    }

    #[test]
    fn parses_value_bearing_tokens() {
        let payload = Bytes::from_static(b"sasl=PLAIN,EXTERNAL chathistory=1000");
        let toks = parse_cap_list(&payload);
        assert_eq!(toks.len(), 2);
        assert_eq!(
            toks[0],
            CapToken {
                name: Bytes::from_static(b"sasl"),
                value: Some(Bytes::from_static(b"PLAIN,EXTERNAL")),
            }
        );
        assert_eq!(toks[1].value.as_deref(), Some(&b"1000"[..]));
    }

    #[test]
    fn skips_internal_doubles_and_trailing_space() {
        let payload = Bytes::from_static(b"a  b ");
        let toks = parse_cap_list(&payload);
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].name.as_ref(), b"a");
        assert_eq!(toks[1].name.as_ref(), b"b");
    }

    #[test]
    fn empty_payload_yields_empty_vec() {
        let payload = Bytes::from_static(b"");
        assert!(parse_cap_list(&payload).is_empty());
    }

    #[test]
    fn detects_continuation_marker() {
        assert!(is_continuation_marker(&Bytes::from_static(b"*")));
        assert!(!is_continuation_marker(&Bytes::from_static(b"**")));
    }
}
