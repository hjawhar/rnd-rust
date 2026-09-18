//! IRC protocol primitives.
//!
//! Wire-level message parsing and serialization, with zero-copy byte
//! slicing via [`bytes::Bytes`]. Phase 1 ships the wire layer; the typed
//! command enum, numerics, codec, mode parsing, CTCP, and formatting
//! codes land in subsequent commits within this phase.
//!
//! The primary entry points are [`Message::parse`] and [`Message::write`].
//! See `PLAN.md` §3 for the full scope.

#![deny(missing_docs)]

pub mod casemap;

pub mod ident;

pub mod cap;

pub mod codec;

pub mod isupport;

pub mod command;
pub mod ctcp;
pub mod dcc;
pub mod format;

pub mod error;
pub mod limits;
pub mod message;
pub mod mode;
pub mod numeric;
pub mod params;
pub mod prefix;
pub mod tags;
pub mod verb;

mod util;

pub use cap::{CapToken, parse_cap_list};
pub use casemap::Casemap;
pub use codec::{CodecError, IrcCodec};
pub use command::{CapSub, Command, CommandError};
pub use ctcp::CtcpMessage;
pub use dcc::DccRequest;
pub use error::ParseError;
pub use format::{Color, Style, StyledSpan, parse_styled, strip_formatting};
pub use ident::{AccountName, ChannelName, IdentError, Nick, ServerName};
pub use isupport::{Isupport, IsupportToken};
pub use message::Message;
pub use mode::{ModeChange, ModeSpec, parse_channel_modes, parse_user_modes};
pub use numeric::ReplyCode;
pub use params::Params;
pub use prefix::Prefix;
pub use tags::{Tag, TagKey, Tags};
pub use verb::Verb;
