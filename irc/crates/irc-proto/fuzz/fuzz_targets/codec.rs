#![no_main]
//! Fuzz target: feed arbitrary byte chunks to the tokio codec.
//!
//! Splits the input at an arbitrary midpoint to exercise partial-line
//! handling (buffered reads). The codec must neither panic nor consume
//! unbounded memory; errors are expected.

use bytes::BytesMut;
use irc_proto::codec::IrcCodec;
use libfuzzer_sys::fuzz_target;
use tokio_util::codec::Decoder;

fuzz_target!(|data: &[u8]| {
    let mid = data.len() / 2;
    let (a, b) = data.split_at(mid);
    let mut codec = IrcCodec::new();
    let mut buf = BytesMut::new();

    buf.extend_from_slice(a);
    loop {
        match codec.decode(&mut buf) {
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
    }

    buf.extend_from_slice(b);
    loop {
        match codec.decode(&mut buf) {
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
    }
});
