#![no_main]
//! Fuzz target: folding arbitrary byte slices under every casemap.
//!
//! Verifies the fold tables never panic and that equality reflexivity
//! holds (any slice is equal to itself under every casemap).

use irc_proto::Casemap;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    for cm in [Casemap::Ascii, Casemap::Rfc1459, Casemap::Rfc1459Strict] {
        let _ = cm.fold(data);
        assert!(cm.eq_bytes(data, data));
    }
});
