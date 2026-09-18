//! Error types for wire parsing.

use thiserror::Error;

/// Error returned when parsing a wire message fails.
///
/// Every variant carries the byte offset into the input where the failure
/// was detected so callers can pinpoint malformed data when diagnosing
/// peer bugs.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The supplied input was empty.
    #[error("empty input")]
    Empty,
    /// The supplied input exceeded the byte limit for its class.
    #[error("line too long: {0} bytes")]
    LineTooLong(usize),
    /// A forbidden byte (NUL, CR, or LF) appeared at the given offset in
    /// a context where it is not permitted by the grammar.
    #[error("forbidden NUL/CR/LF byte at offset {0}")]
    Forbidden(usize),
    /// A mandatory space separator was missing at the given offset.
    #[error("missing space separator at offset {0}")]
    MissingSpace(usize),
    /// Input ended while more bytes were required.
    #[error("unexpected end of input")]
    UnexpectedEnd,
    /// A tag key was malformed at the given offset.
    #[error("invalid tag key at offset {0}")]
    InvalidTagKey(usize),
    /// A tag value contained an invalid or incomplete escape.
    #[error("invalid tag escape at offset {0}")]
    InvalidTagEscape(usize),
    /// The command word was neither an alphabetic verb nor a three-digit
    /// numeric.
    #[error("invalid command at offset {0}")]
    InvalidCommand(usize),
    /// The prefix was malformed.
    #[error("invalid prefix at offset {0}")]
    InvalidPrefix(usize),
    /// Too many params supplied on the wire (more than [`crate::limits::MAX_PARAMS`]).
    #[error("too many params at offset {0}")]
    TooManyParams(usize),
}
