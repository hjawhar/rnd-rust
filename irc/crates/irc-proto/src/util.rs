//! Shared byte predicates used by the parser.
//!
//! Kept in a single module so the rules stay consistent across tags,
//! prefix, verb, and params parsing.

/// Bytes forbidden anywhere in a wire message: NUL, CR, LF.
#[inline]
pub(crate) const fn is_forbidden(b: u8) -> bool {
    matches!(b, 0 | b'\r' | b'\n')
}

/// Valid characters in an IRCv3 tag key (including vendor portion separators).
///
/// `key-name = 1*( letter / digit / "-" / "_" )`; vendors are
/// dot-separated DNS names, so we also allow `.` and the `/` separator
/// between vendor and key-name here for a single-pass lexer.
#[inline]
pub(crate) const fn is_tag_key_byte(b: u8) -> bool {
    matches!(
        b,
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/'
    )
}

/// Valid bytes inside an IRCv3 tag value before escape decoding.
///
/// Everything is allowed except NUL, CR, LF, space, semicolon, and the
/// backslash itself (which begins an escape sequence).
#[inline]
pub(crate) const fn is_tag_value_byte(b: u8) -> bool {
    !matches!(b, 0 | b'\r' | b'\n' | b' ' | b';' | b'\\')
}

/// First-byte rule for a `middle` param: any octet except NUL, CR, LF,
/// space, or `:`.
#[inline]
pub(crate) const fn is_middle_first(b: u8) -> bool {
    !matches!(b, 0 | b'\r' | b'\n' | b' ' | b':')
}

/// Continuation-byte rule for a `middle` param: any octet except NUL,
/// CR, LF, or space.
#[inline]
pub(crate) const fn is_middle_rest(b: u8) -> bool {
    !matches!(b, 0 | b'\r' | b'\n' | b' ')
}
