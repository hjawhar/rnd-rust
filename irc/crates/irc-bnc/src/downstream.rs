use std::sync::Arc;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio_util::codec::Framed;
use tracing::{debug, error, info, warn};

use irc_proto::params::Params;
use irc_proto::verb::Verb;
use irc_proto::{Command, IrcCodec, Message, Prefix, Tags};

use crate::admin;
use crate::config::BncUser;
use crate::upstream::Upstream;

/// Result of parsing a PASS credential.
pub struct AuthInfo {
    /// Matched user name.
    pub user_name: String,
    /// Matched network name.
    pub network_name: String,
}

/// Parse PASS credentials in the format `user/network:password`.
pub fn parse_pass(pass: &[u8], users: &[BncUser]) -> Option<(AuthInfo, usize)> {
    let s = std::str::from_utf8(pass).ok()?;
    let (user_network, password) = s.split_once(':')?;
    let (user_name, network_name) = user_network.split_once('/')?;

    let user_idx = users.iter().position(|u| {
        u.name == user_name
            && u.password == password
            && u.networks.iter().any(|n| n.name == network_name)
    })?;

    Some((
        AuthInfo {
            user_name: user_name.to_owned(),
            network_name: network_name.to_owned(),
        },
        user_idx,
    ))
}

/// Handle a single downstream client connection.
#[allow(clippy::cognitive_complexity)] // IRC downstream auth + replay is inherently branchy
pub async fn handle_downstream(
    stream: TcpStream,
    upstream: Arc<Upstream>,
    users: Arc<Vec<BncUser>>,
    upstream_tx: tokio::sync::mpsc::Sender<Message>,
) {
    let addr = stream
        .peer_addr()
        .map_or_else(|_| "unknown".into(), |a| a.to_string());
    info!(addr = %addr, "downstream client connected");

    let mut framed = Framed::new(stream, IrcCodec::new());

    let Some(auth) = authenticate(&mut framed, &users).await else {
        let _ = send_error(&mut framed, "Authentication failed").await;
        return;
    };

    info!(user = %auth.user_name, network = %auth.network_name, "downstream authenticated");

    send_welcome(&mut framed, &auth).await;
    send_join_and_replay(&mut framed, &upstream).await;
    relay_loop(&mut framed, &upstream, &upstream_tx).await;

    info!(addr = %addr, "downstream client disconnected");
}

/// Collect JOINs and buffered messages from upstream state, then send to client.
async fn send_join_and_replay(framed: &mut Framed<TcpStream, IrcCodec>, upstream: &Upstream) {
    let (join_msgs, replay_msgs) = {
        let state = upstream.state();
        let joins: Vec<Message> = state
            .joined_channels
            .iter()
            .map(|ch| Message {
                tags: Tags::new(),
                prefix: Some(Prefix::user(state.nick.clone(), None, None)),
                verb: Verb::Word(Bytes::from_static(b"JOIN")),
                params: Params::from_iter_middle([Bytes::from(ch.clone())]),
            })
            .collect();

        let mut replays = Vec::new();
        for buf in state.buffers.values() {
            for (ts, msg) in buf.last_n(500) {
                let mut replay = msg.clone();
                replay.tags.push(irc_proto::Tag {
                    key: irc_proto::TagKey {
                        client_only: false,
                        name: Bytes::from_static(b"time"),
                    },
                    value: Some(ts),
                });
                replays.push(replay);
            }
        }
        drop(state);
        (joins, replays)
    };

    for msg in join_msgs {
        let _ = framed.send(msg).await;
    }
    for msg in replay_msgs {
        let _ = framed.send(msg).await;
    }
}

/// Bidirectional relay between upstream broadcast and downstream client.
#[allow(clippy::cognitive_complexity)]
async fn relay_loop(
    framed: &mut Framed<TcpStream, IrcCodec>,
    upstream: &Upstream,
    upstream_tx: &tokio::sync::mpsc::Sender<Message>,
) {
    let mut rx = upstream.subscribe();
    loop {
        tokio::select! {
            upstream_msg = rx.recv() => {
                match upstream_msg {
                    Ok(msg) => {
                        if framed.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(lagged = n, "downstream lagged, messages dropped");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
            client_msg = framed.next() => {
                match client_msg {
                    Some(Ok(msg)) => {
                        let admin_replies = {
                            let state = upstream.state();
                            admin::handle_admin_command(&msg, &state)
                        };
                        if let Some(replies) = admin_replies {
                            for reply in replies {
                                if framed.send(reply).await.is_err() {
                                    return;
                                }
                            }
                            continue;
                        }
                        if upstream_tx.send(msg).await.is_err() {
                            error!("upstream send channel closed");
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        debug!(error = %e, "downstream codec error");
                        break;
                    }
                    None => break,
                }
            }
        }
    }
}

async fn authenticate(
    framed: &mut Framed<TcpStream, IrcCodec>,
    users: &[BncUser],
) -> Option<AuthInfo> {
    let mut pass: Option<Bytes> = None;
    let mut got_nick = false;
    let mut got_user = false;

    for _ in 0..10 {
        let Some(Ok(msg)) = framed.next().await else {
            return None;
        };

        match Command::parse(&msg) {
            Ok(Command::Pass { password }) => pass = Some(password),
            Ok(Command::Nick { .. }) => got_nick = true,
            Ok(Command::User { .. }) => got_user = true,
            _ => {}
        }

        if pass.is_some() && got_nick && got_user {
            break;
        }
    }

    let pass_bytes = pass?;
    let (info, _) = parse_pass(&pass_bytes, users)?;
    Some(info)
}

async fn send_welcome(framed: &mut Framed<TcpStream, IrcCodec>, auth: &AuthInfo) {
    let server = Bytes::from_static(b"irc-bnc");
    let nick = Bytes::from(auth.user_name.clone());

    let numerics: &[(u16, &str)] = &[
        (1, "Welcome to the IRC bouncer"),
        (2, "Your host is irc-bnc, running irc-bnc"),
        (3, "This server was created now"),
        (4, "irc-bnc 0.0.0 o o"),
    ];

    for &(code, text) in numerics {
        let mut params = Params::from_iter_middle([nick.clone()]);
        params.push_trailing(Bytes::from(text));
        let msg = Message {
            tags: Tags::new(),
            prefix: Some(Prefix::server(server.clone())),
            verb: Verb::Numeric(code),
            params,
        };
        let _ = framed.send(msg).await;
    }
}

async fn send_error(framed: &mut Framed<TcpStream, IrcCodec>, reason: &str) -> anyhow::Result<()> {
    let mut params = Params::new();
    params.push_trailing(Bytes::from(reason.to_owned()));
    let msg = Message {
        tags: Tags::new(),
        prefix: None,
        verb: Verb::Word(Bytes::from_static(b"ERROR")),
        params,
    };
    framed.send(msg).await.map_err(|e| anyhow::anyhow!("{e}"))
}
