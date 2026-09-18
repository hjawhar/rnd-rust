//! Typed identifier newtypes.
//!
//! Validates inputs at construction so downstream code can pass a
//! `Nick`, `ChannelName`, `ServerName`, or `AccountName` without
//! re-checking the byte rules at every call site.
//!
//! Equality on these types is **byte-wise**. Casemap-aware comparison
//! is intentionally a separate operation owned by [`crate::Casemap`] —
//! the identifier holds the original-case bytes (for display) and the
//! folded form is computed on demand via [`Nick::fold`] /
//! [`ChannelName::fold`].

use bytes::Bytes;
use thiserror::Error;

use crate::casemap::Casemap;

/// Validation errors returned when constructing an identifier.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IdentError {
    /// Identifier was empty.
    #[error("identifier is empty")]
    Empty,
    /// Identifier exceeded the maximum length for its kind.
    #[error("identifier too long: {0} bytes (max {1})")]
    TooLong(usize, usize),
    /// A byte at the given offset was not permitted.
    #[error("invalid byte 0x{1:02x} at offset {0}")]
    InvalidByte(usize, u8),
    /// First byte was not permitted at that position.
    #[error("invalid leading byte 0x{0:02x}")]
    InvalidLeadingByte(u8),
    /// Channel name did not start with a recognised prefix.
    #[error("channel name must start with #, &, +, or !")]
    InvalidChannelPrefix,
}

/// Length cap for nicknames before any ISUPPORT `NICKLEN` override.
///
/// RFC 2812 specifies 9; modern servers commonly advertise 16-32. We
/// pick a generous baseline that accommodates the network without
/// hard-coding network-specific quirks.
pub const DEFAULT_NICK_MAX: usize = 32;

/// Length cap for channel names before any ISUPPORT `CHANNELLEN`
/// override.
pub const DEFAULT_CHANNEL_MAX: usize = 50;

/// Length cap for server names.
pub const DEFAULT_SERVER_MAX: usize = 63;

/// Length cap for account names before any service-specific override.
pub const DEFAULT_ACCOUNT_MAX: usize = 32;

/// Nickname identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Nick(Bytes);

impl Nick {
    /// Validate `bytes` as a nickname and wrap.
    pub fn parse(bytes: impl Into<Bytes>) -> Result<Self, IdentError> {
        let b = bytes.into();
        validate_nick(&b)?;
        Ok(Self(b))
    }

    /// Construct without validation. Caller asserts the bytes already
    /// satisfy the nick grammar; useful when the bytes came from a
    /// trusted source (a parsed wire prefix from our own server).
    #[must_use]
    pub fn from_bytes_unchecked(bytes: impl Into<Bytes>) -> Self {
        Self(bytes.into())
    }

    /// Underlying bytes in original case.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the nick is the empty string. Always `false` for a value
    /// constructed via [`Nick::parse`] — provided to satisfy clippy.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Return the case-folded form for the supplied casemap.
    #[must_use]
    pub fn fold(&self, casemap: Casemap) -> Bytes {
        casemap.fold(&self.0)
    }

    /// Casemap-aware equality.
    #[must_use]
    pub fn eq_under(&self, other: &Self, casemap: Casemap) -> bool {
        casemap.eq_bytes(&self.0, &other.0)
    }
}

/// Channel name identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChannelName(Bytes);

impl ChannelName {
    /// Validate `bytes` as a channel name and wrap.
    pub fn parse(bytes: impl Into<Bytes>) -> Result<Self, IdentError> {
        let b = bytes.into();
        validate_channel(&b)?;
        Ok(Self(b))
    }

    /// Construct without validation.
    #[must_use]
    pub fn from_bytes_unchecked(bytes: impl Into<Bytes>) -> Self {
        Self(bytes.into())
    }

    /// Underlying bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the name is empty. Always `false` for parsed values.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Return the prefix byte (`#`, `&`, `+`, or `!`).
    #[must_use]
    pub fn prefix(&self) -> u8 {
        self.0[0]
    }

    /// Return the case-folded form for the supplied casemap.
    #[must_use]
    pub fn fold(&self, casemap: Casemap) -> Bytes {
        casemap.fold(&self.0)
    }

    /// Casemap-aware equality.
    #[must_use]
    pub fn eq_under(&self, other: &Self, casemap: Casemap) -> bool {
        casemap.eq_bytes(&self.0, &other.0)
    }
}

/// Server-name identifier (e.g. `irc.example.net`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerName(Bytes);

impl ServerName {
    /// Validate `bytes` as a server name and wrap.
    pub fn parse(bytes: impl Into<Bytes>) -> Result<Self, IdentError> {
        let b = bytes.into();
        validate_server(&b)?;
        Ok(Self(b))
    }

    /// Construct without validation.
    #[must_use]
    pub fn from_bytes_unchecked(bytes: impl Into<Bytes>) -> Self {
        Self(bytes.into())
    }

    /// Underlying bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Account-name identifier (services / SASL account).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountName(Bytes);

impl AccountName {
    /// Validate `bytes` as an account name and wrap.
    pub fn parse(bytes: impl Into<Bytes>) -> Result<Self, IdentError> {
        let b = bytes.into();
        validate_account(&b)?;
        Ok(Self(b))
    }

    /// Construct without validation.
    #[must_use]
    pub fn from_bytes_unchecked(bytes: impl Into<Bytes>) -> Self {
        Self(bytes.into())
    }

    /// Underlying bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

// --- validators -------------------------------------------------------

fn validate_nick(b: &[u8]) -> Result<(), IdentError> {
    if b.is_empty() {
        return Err(IdentError::Empty);
    }
    if b.len() > DEFAULT_NICK_MAX {
        return Err(IdentError::TooLong(b.len(), DEFAULT_NICK_MAX));
    }
    let first = b[0];
    if !is_nick_first_byte(first) {
        return Err(IdentError::InvalidLeadingByte(first));
    }
    for (i, &byte) in b.iter().enumerate().skip(1) {
        if !is_nick_rest_byte(byte) {
            return Err(IdentError::InvalidByte(i, byte));
        }
    }
    Ok(())
}

fn validate_channel(b: &[u8]) -> Result<(), IdentError> {
    if b.is_empty() {
        return Err(IdentError::Empty);
    }
    if b.len() > DEFAULT_CHANNEL_MAX {
        return Err(IdentError::TooLong(b.len(), DEFAULT_CHANNEL_MAX));
    }
    if !matches!(b[0], b'#' | b'&' | b'+' | b'!') {
        return Err(IdentError::InvalidChannelPrefix);
    }
    for (i, &byte) in b.iter().enumerate().skip(1) {
        if !is_channel_rest_byte(byte) {
            return Err(IdentError::InvalidByte(i, byte));
        }
    }
    Ok(())
}

fn validate_server(b: &[u8]) -> Result<(), IdentError> {
    if b.is_empty() {
        return Err(IdentError::Empty);
    }
    if b.len() > DEFAULT_SERVER_MAX {
        return Err(IdentError::TooLong(b.len(), DEFAULT_SERVER_MAX));
    }
    for (i, &byte) in b.iter().enumerate() {
        if !is_server_byte(byte) {
            return Err(IdentError::InvalidByte(i, byte));
        }
    }
    Ok(())
}

fn validate_account(b: &[u8]) -> Result<(), IdentError> {
    if b.is_empty() {
        return Err(IdentError::Empty);
    }
    if b.len() > DEFAULT_ACCOUNT_MAX {
        return Err(IdentError::TooLong(b.len(), DEFAULT_ACCOUNT_MAX));
    }
    for (i, &byte) in b.iter().enumerate() {
        if !is_account_byte(byte) {
            return Err(IdentError::InvalidByte(i, byte));
        }
    }
    Ok(())
}

#[inline]
const fn is_nick_first_byte(b: u8) -> bool {
    matches!(
        b,
        b'a'..=b'z' | b'A'..=b'Z'
            | b'[' | b']' | b'\\' | b'`' | b'_' | b'^' | b'{' | b'|' | b'}'
    )
}

#[inline]
const fn is_nick_rest_byte(b: u8) -> bool {
    matches!(
        b,
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9'
            | b'[' | b']' | b'\\' | b'`' | b'_' | b'^' | b'{' | b'|' | b'}' | b'-'
    )
}

#[inline]
const fn is_channel_rest_byte(b: u8) -> bool {
    // Forbidden: NUL, BEL (\x07), CR, LF, SPACE, comma, colon.
    !matches!(b, 0 | 0x07 | b'\r' | b'\n' | b' ' | b',' | b':')
}

#[inline]
const fn is_server_byte(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-')
}

#[inline]
const fn is_account_byte(b: u8) -> bool {
    // Permissive baseline: ASCII printable except space and the hostmask
    // delimiters `!`, `@`, `,`, `:`. Phase 3 may tighten further.
    matches!(b, 0x21..=0x7E) && !matches!(b, b' ' | b'!' | b'@' | b',' | b':')
}

#[cfg(test)]
mod tests {
    use super::{AccountName, ChannelName, IdentError, Nick, ServerName};
    use crate::casemap::Casemap;

    #[test]
    fn nick_accepts_letters_brackets_and_underscores() {
        Nick::parse("alice").unwrap();
        Nick::parse("Alice_42").unwrap();
        Nick::parse("[bot]").unwrap();
        Nick::parse("nick-with-dash").unwrap();
    }

    #[test]
    fn nick_rejects_empty_and_too_long() {
        assert_eq!(Nick::parse(""), Err(IdentError::Empty));
        let huge = "a".repeat(64);
        assert!(matches!(Nick::parse(huge), Err(IdentError::TooLong(64, _))));
    }

    #[test]
    fn nick_rejects_leading_digit() {
        assert!(matches!(
            Nick::parse("9bob"),
            Err(IdentError::InvalidLeadingByte(b'9'))
        ));
    }

    #[test]
    fn nick_rejects_space_or_punctuation() {
        assert!(Nick::parse("a b").is_err());
        assert!(Nick::parse("a:b").is_err());
        assert!(Nick::parse("a!b").is_err());
    }

    #[test]
    fn channel_requires_prefix_byte() {
        ChannelName::parse("#rust").unwrap();
        ChannelName::parse("&local").unwrap();
        ChannelName::parse("+modeless").unwrap();
        ChannelName::parse("!safe").unwrap();
        assert_eq!(
            ChannelName::parse("rust"),
            Err(IdentError::InvalidChannelPrefix)
        );
    }

    #[test]
    fn channel_forbids_control_chars_and_separators() {
        assert!(ChannelName::parse("#a,b").is_err());
        assert!(ChannelName::parse("#a b").is_err());
        assert!(ChannelName::parse("#a:b").is_err());
        assert!(ChannelName::parse("#a\x07b").is_err());
    }

    #[test]
    fn server_name_accepts_dotted_dns_form() {
        ServerName::parse("irc.example.net").unwrap();
        ServerName::parse("us-east-1.example.com").unwrap();
        assert!(ServerName::parse("bad space").is_err());
    }

    #[test]
    fn account_name_accepts_printable_ascii() {
        AccountName::parse("alice").unwrap();
        AccountName::parse("alice123").unwrap();
        assert!(AccountName::parse("alice@host").is_err());
        assert!(AccountName::parse("alice spaced").is_err());
    }

    #[test]
    fn casemap_aware_equality_respects_rfc1459() {
        let a = Nick::parse("Alice[home]").unwrap();
        let b = Nick::parse("alice{home}").unwrap();
        assert!(!a.eq_under(&b, Casemap::Ascii));
        assert!(a.eq_under(&b, Casemap::Rfc1459));
        assert_ne!(a, b); // byte equality is strict
    }

    #[test]
    fn folded_form_is_zero_copy_per_byte() {
        let n = Nick::parse("Alice[home]").unwrap();
        assert_eq!(n.fold(Casemap::Rfc1459).as_ref(), b"alice{home}");
    }
}
