//! Helpers for assembling numeric reply messages.

use bytes::Bytes;
use irc_proto::{Message, Params, Prefix, ReplyCode, Tags, Verb};

use crate::state::ServerState;

/// A reply target — either the nick of a registered user or `*` for a
/// pre-registration connection.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a>(pub &'a [u8]);

impl Target<'_> {
    /// Literal `*` target used before the client has a nick.
    pub const UNREGISTERED: Self = Self(b"*");
}

/// Build a numeric reply with any number of middle params and an
/// optional trailing (human-readable) tail.
pub fn numeric<I>(
    state: &ServerState,
    target: Target<'_>,
    code: ReplyCode,
    middles: I,
    trailing: Option<Bytes>,
) -> Message
where
    I: IntoIterator<Item = Bytes>,
{
    let mut params = Params::new();
    params.push(Bytes::copy_from_slice(target.0));
    for m in middles {
        params.push(m);
    }
    if let Some(t) = trailing {
        params.push_trailing(t);
    }
    Message {
        tags: Tags::new(),
        prefix: Some(Prefix::Server(server_name_bytes(state))),
        verb: Verb::Numeric(code.code()),
        params,
    }
}

/// Shortcut for the common shape `<code> <target> :<text>`.
pub fn numeric_text(
    state: &ServerState,
    target: Target<'_>,
    code: ReplyCode,
    text: impl Into<Bytes>,
) -> Message {
    numeric(state, target, code, std::iter::empty(), Some(text.into()))
}

/// Shortcut for numerics with a single middle param and a trailing.
pub fn numeric_one(
    state: &ServerState,
    target: Target<'_>,
    code: ReplyCode,
    middle: impl Into<Bytes>,
    text: impl Into<Bytes>,
) -> Message {
    numeric(
        state,
        target,
        code,
        std::iter::once(middle.into()),
        Some(text.into()),
    )
}

/// Convenience: this server's name as a [`Bytes`].
pub fn server_name_bytes(state: &ServerState) -> Bytes {
    Bytes::copy_from_slice(state.config().server_name.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::{Target, numeric, numeric_text};
    use crate::{Config, ServerState};
    use irc_proto::ReplyCode;
    use std::sync::Arc;

    fn state() -> ServerState {
        ServerState::new(
            Arc::new(Config::builder().build().unwrap()),
            Arc::new(crate::store::AnyStore::InMemory(
                crate::store::InMemoryStore::new(),
            )),
        )
    }

    #[test]
    fn numeric_text_includes_target_and_trailing() {
        let s = state();
        let msg = numeric_text(&s, Target(b"alice"), ReplyCode::RPL_WELCOME, "Welcome!");
        let bytes = msg.to_bytes();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains(":irc.local 001 alice :Welcome!"));
    }

    #[test]
    fn numeric_supports_middle_params() {
        let s = state();
        let msg = numeric(
            &s,
            Target(b"alice"),
            ReplyCode::RPL_ISUPPORT,
            [bytes::Bytes::from_static(b"CASEMAPPING=rfc1459")],
            Some("are supported by this server".into()),
        );
        let bytes = msg.to_bytes();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains("005 alice CASEMAPPING=rfc1459 :are supported"));
    }
}
