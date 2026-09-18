//! Byte-length and count limits defined by the protocol specs.

/// Maximum byte length of the tags portion (including the leading `@`),
/// per the IRCv3 `message-tags` specification.
pub const MAX_TAG_BYTES: usize = 8191;

/// Maximum byte length of a non-tagged message (prefix + verb + params +
/// CRLF), per RFC 1459 / RFC 2812.
pub const MAX_REST_BYTES: usize = 512;

/// Maximum total byte length of a message on the wire.
pub const MAX_LINE_BYTES: usize = MAX_TAG_BYTES + MAX_REST_BYTES;

/// Maximum number of params a wire message may carry.
///
/// RFC 2812 permits up to 14 `middle` params plus one `trailing`, giving
/// 15 total.
pub const MAX_PARAMS: usize = 15;
