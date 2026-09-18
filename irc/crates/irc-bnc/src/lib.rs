//! Multi-user IRC bouncer with persistent upstreams and IRCv3 server-time replay.

pub mod admin;
pub mod buffer;
pub mod config;
pub mod downstream;
pub mod upstream;

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_util::codec::Framed;
use tracing::{error, info};

use irc_proto::params::Params;
use irc_proto::verb::Verb;
use irc_proto::{Command, IrcCodec, Message, Tags};

use crate::config::BncConfig;
use crate::upstream::Upstream;

/// The core bouncer, managing upstream connections and accepting downstream clients.
pub struct Bouncer {
    config: BncConfig,
    /// Per-user, per-network upstream connections.
    upstreams: HashMap<(String, String), Arc<Upstream>>,
}

impl Bouncer {
    /// Create a new bouncer from the given configuration.
    pub fn new(config: BncConfig) -> Self {
        let mut upstreams = HashMap::new();
        for user in &config.users {
            for network in &user.networks {
                let key = (user.name.clone(), network.name.clone());
                upstreams.insert(key, Arc::new(Upstream::new(network.clone())));
            }
        }
        Self { config, upstreams }
    }

    /// Run the bouncer: connect upstreams and listen for downstream clients.
    pub async fn run(self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(self.config.listen).await?;
        info!(addr = %self.config.listen, "bouncer listening");

        let users = Arc::new(self.config.users.clone());

        // Spawn upstream connection tasks + writer channels
        let mut upstream_senders: HashMap<(String, String), mpsc::Sender<Message>> = HashMap::new();
        for (key, upstream) in &self.upstreams {
            let (tx, mut rx) = mpsc::channel::<Message>(128);
            upstream_senders.insert(key.clone(), tx);

            let upstream_for_read = Arc::clone(upstream);
            tokio::spawn(async move {
                loop {
                    if let Err(e) = upstream_for_read.run().await {
                        error!(error = %e, "upstream connection failed, retrying in 30s");
                    }
                    tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                }
            });

            // Writer task — currently logs; full implementation shares the Framed writer.
            tokio::spawn(async move {
                while let Some(_msg) = rx.recv().await {
                    // Placeholder: in a full implementation this would forward to
                    // the upstream Framed sink.
                }
            });
        }

        // Accept downstream connections
        loop {
            let (stream, addr) = listener.accept().await?;
            info!(addr = %addr, "accepted downstream connection");

            let users = Arc::clone(&users);
            let upstreams = self.upstreams.clone();
            let senders = upstream_senders.clone();

            tokio::spawn(async move {
                handle_new_downstream(stream, users, upstreams, senders).await;
            });
        }
    }
}

/// Pre-authenticate a new downstream connection, then hand off to the relay.
async fn handle_new_downstream(
    stream: tokio::net::TcpStream,
    users: Arc<Vec<crate::config::BncUser>>,
    upstreams: HashMap<(String, String), Arc<Upstream>>,
    senders: HashMap<(String, String), mpsc::Sender<Message>>,
) {
    let mut framed = Framed::new(stream, IrcCodec::new());

    let mut pass_bytes: Option<Bytes> = None;
    let mut got_nick = false;
    let mut got_user = false;

    for _ in 0..10 {
        let Some(Ok(msg)) = framed.next().await else {
            return;
        };
        match Command::parse(&msg) {
            Ok(Command::Pass { password }) => pass_bytes = Some(password),
            Ok(Command::Nick { .. }) => got_nick = true,
            Ok(Command::User { .. }) => got_user = true,
            _ => {}
        }
        if pass_bytes.is_some() && got_nick && got_user {
            break;
        }
    }

    let Some(pass_bytes) = pass_bytes else {
        let _ = send_error_raw(&mut framed, "No PASS provided").await;
        return;
    };

    let Some((auth_info, _user_idx)) = downstream::parse_pass(&pass_bytes, &users) else {
        let _ = send_error_raw(&mut framed, "Authentication failed").await;
        return;
    };

    let key = (auth_info.user_name.clone(), auth_info.network_name.clone());

    let Some(upstream) = upstreams.get(&key).map(Arc::clone) else {
        let _ = send_error_raw(&mut framed, "Unknown network").await;
        return;
    };

    let Some(sender) = senders.get(&key).cloned() else {
        return;
    };

    // Reconstruct the TcpStream from the Framed to hand to handle_downstream
    let stream = framed.into_inner();
    downstream::handle_downstream(stream, upstream, users, sender).await;
}

async fn send_error_raw(
    framed: &mut Framed<tokio::net::TcpStream, IrcCodec>,
    reason: &str,
) -> anyhow::Result<()> {
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
