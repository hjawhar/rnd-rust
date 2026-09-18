#![no_main]
//! Fuzz target: feed arbitrary bytes to [`irc_proto::Message::parse`].
//!
//! The parser is required to never panic on any input. Errors are
//! expected (and fine); crashes are bugs.

use bytes::Bytes;
use irc_proto::Message;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = Message::parse(&Bytes::copy_from_slice(data));
});
