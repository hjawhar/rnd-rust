//! Keep-alive and lifecycle handlers: PING, PONG, QUIT.

use std::sync::Arc;

use bytes::Bytes;
use irc_proto::{Message, Params, Prefix, Tags, Verb};

use crate::handler::Outcome;
use crate::state::{ServerState, User};

/// Respond to `PING <token>` with `PONG <server> <token>`.
pub fn handle_ping(state: &Arc<ServerState>, user: &Arc<User>, token: Bytes) -> Outcome {
    let sv = crate::numeric::server_name_bytes(state);
    let mut params = Params::new();
    params.push(sv.clone());
    params.push_trailing(token);
    let msg = Message {
        tags: Tags::new(),
        prefix: Some(Prefix::Server(sv)),
        verb: Verb::word(Bytes::from_static(b"PONG")),
        params,
    };
    user.send(msg);
    Outcome::Continue
}

/// Handle `QUIT [:reason]`.
///
/// The connection driver is responsible for emitting the goodbye to
/// any peers the user shared channels with — we only signal
/// disconnect here so the caller tears the connection down cleanly.
pub fn handle_quit(
    _state: &Arc<ServerState>,
    _user: &Arc<User>,
    _reason: Option<Bytes>,
) -> Outcome {
    Outcome::Disconnect
}

#[cfg(test)]
mod tests {
    use super::handle_ping;
    use crate::Config;
    use crate::state::{ServerState, User};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn ping_replies_pong_with_same_token() {
        let cfg = Arc::new(Config::builder().build().unwrap());
        let store = Arc::new(crate::store::AnyStore::InMemory(
            crate::store::InMemoryStore::new(),
        ));
        let state = Arc::new(ServerState::new(cfg, store));
        let (tx, mut rx) = mpsc::channel(8);
        let user = Arc::new(User::new(
            state.next_user_id(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            tx,
        ));
        let _ = handle_ping(&state, &user, bytes::Bytes::from_static(b"cookie"));
        let reply = rx.recv().await.unwrap();
        let wire = String::from_utf8(reply.to_bytes().to_vec()).unwrap();
        assert!(wire.contains("PONG irc.local :cookie"), "got {wire:?}");
    }
}
