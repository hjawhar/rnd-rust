//! Property tests: serialize an arbitrary [`Message`], parse the
//! result, and assert byte-level equality.
//!
//! Generators are constrained to the wire-level shape the serializer
//! emits (e.g. middle values never contain space), so every generated
//! message has exactly one canonical serialization.

use bytes::Bytes;
use irc_proto::{Message, Params, Prefix, Tag, TagKey, Tags, Verb};
use proptest::prelude::*;

fn arb_ascii_alpha(len: std::ops::Range<usize>) -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(
        any::<u8>().prop_filter("ascii-alpha", u8::is_ascii_alphabetic),
        len,
    )
}

fn arb_tag_key_name() -> impl Strategy<Value = Bytes> {
    proptest::collection::vec(
        any::<u8>().prop_filter("tag-key-byte", |b| {
            b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_'
        }),
        1..15,
    )
    .prop_map(Bytes::from)
}

fn arb_tag_value() -> impl Strategy<Value = Bytes> {
    // Any bytes except NUL/CR/LF. Backslash and semicolon are fine —
    // they round-trip via the escape codec.
    proptest::collection::vec(
        any::<u8>().prop_filter("no-nul-cr-lf", |b| *b != 0 && *b != b'\r' && *b != b'\n'),
        0..50,
    )
    .prop_map(Bytes::from)
}

fn arb_tag() -> impl Strategy<Value = Tag> {
    (
        any::<bool>(),
        arb_tag_key_name(),
        proptest::option::of(arb_tag_value()),
    )
        .prop_map(|(client_only, name, value)| Tag {
            key: TagKey { client_only, name },
            value,
        })
}

fn arb_tags() -> impl Strategy<Value = Tags> {
    proptest::collection::vec(arb_tag(), 0..4).prop_map(|v| {
        let mut t = Tags::new();
        for tag in v {
            t.push(tag);
        }
        t
    })
}

fn arb_prefix() -> impl Strategy<Value = Option<Prefix>> {
    // Server names contain at least one dot so classification is
    // unambiguous; user-origin prefixes use pure-alpha nicks to avoid
    // accidentally looking like server names.
    let server = arb_ascii_alpha(3..10).prop_map(|mut v| {
        v.push(b'.');
        v.extend_from_slice(b"net");
        Prefix::Server(Bytes::from(v))
    });
    let user = (
        arb_ascii_alpha(1..10),
        proptest::option::of(arb_ascii_alpha(1..10)),
        proptest::option::of(arb_ascii_alpha(1..15)),
    )
        .prop_map(|(nick, user, host)| Prefix::User {
            nick: Bytes::from(nick),
            user: user.map(Bytes::from),
            host: host.map(Bytes::from),
        });
    proptest::option::of(prop_oneof![server, user])
}

fn arb_verb() -> impl Strategy<Value = Verb> {
    let word = arb_ascii_alpha(1..8).prop_map(|v| Verb::Word(Bytes::from(v)));
    let numeric = (0u16..1000).prop_map(Verb::Numeric);
    prop_oneof![word, numeric]
}

fn arb_middle_value() -> impl Strategy<Value = Bytes> {
    proptest::collection::vec(
        any::<u8>().prop_filter("middle-rest", |b| !matches!(*b, 0 | b'\r' | b'\n' | b' ')),
        1..15,
    )
    .prop_filter("middle-first-not-colon", |v| v.first() != Some(&b':'))
    .prop_map(Bytes::from)
}

fn arb_trailing_value() -> impl Strategy<Value = Bytes> {
    proptest::collection::vec(
        any::<u8>().prop_filter("trailing-byte", |b| !matches!(*b, 0 | b'\r' | b'\n')),
        0..50,
    )
    .prop_map(Bytes::from)
}

fn arb_params() -> impl Strategy<Value = Params> {
    (
        proptest::collection::vec(arb_middle_value(), 0..14),
        proptest::option::of(arb_trailing_value()),
    )
        .prop_map(|(middles, trailing)| {
            let mut p = Params::new();
            for m in middles {
                p.push(m);
            }
            if let Some(t) = trailing {
                p.push_trailing(t);
            }
            p
        })
}

fn arb_message() -> impl Strategy<Value = Message> {
    (arb_tags(), arb_prefix(), arb_verb(), arb_params()).prop_map(|(tags, prefix, verb, params)| {
        Message {
            tags,
            prefix,
            verb,
            params,
        }
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn parse_write_roundtrip(msg in arb_message()) {
        let bytes = msg.to_bytes();
        let parsed = Message::parse(&bytes).expect("parse must succeed for generated message");
        prop_assert_eq!(parsed, msg);
    }

    #[test]
    fn parse_never_panics_on_random_bytes(input in proptest::collection::vec(any::<u8>(), 0..600)) {
        // The parser may error — that's fine. It must not panic.
        let bytes = Bytes::from(input);
        let _ = Message::parse(&bytes);
    }
}
