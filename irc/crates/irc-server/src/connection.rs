//! Per-connection driver.
//!
//! Each accepted socket is split into a read half and a write half.
//! The write task drains a bounded `mpsc::Receiver<Message>` fed by
//! the rest of the server; the read task decodes incoming messages
//! and hands them to the dispatcher. When the dispatcher says
//! `Outcome::Disconnect`, the connection tears down cleanly,
//! broadcasting a QUIT to every peer that shares a channel.

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use irc_proto::{IrcCodec, Message, Params, Prefix, Tags, Verb};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, FramedWrite};
use tracing::{Instrument, debug, info, info_span, warn};

use crate::flood::FloodBucket;
use crate::handler::{Outcome, dispatch};
use crate::limiter::ConnectionLimiter;
use crate::state::{ServerState, User, UserId};

/// Bounded capacity for the per-connection outbound queue.
/// Overflow drops the message — Phase 5 tightens this with a policy.
const OUTBOUND_QUEUE: usize = 256;

/// A stream that is either plain TCP or TLS-wrapped TCP.
pub enum MaybeTls {
    /// Unencrypted TCP connection.
    Plain(TcpStream),
    /// TLS-encrypted TCP connection.
    Tls(Box<tokio_rustls::server::TlsStream<TcpStream>>),
}

impl AsyncRead for MaybeTls {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MaybeTls {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_flush(cx),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Self::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// Drive a single accepted connection to completion.
pub async fn handle_connection(
    state: Arc<ServerState>,
    limiter: Arc<ConnectionLimiter>,
    stream: MaybeTls,
    peer: SocketAddr,
) {
    let ip = peer.ip();
    if !limiter.try_acquire(ip, state.config().limits.per_ip_max_connections) {
        warn!(%peer, "per-IP connection limit reached, dropping");
        return;
    }

    let user_id = state.next_user_id();
    let span = info_span!("conn", user = user_id.get(), %peer);
    async move {
        info!("accepted");
        crate::metrics::record_connection_open();
        run(state.clone(), stream, user_id, peer).await;
        cleanup(&state, user_id);
        crate::metrics::record_connection_close();
        info!("closed");
    }
    .instrument(span)
    .await;
    limiter.release(ip);
}

async fn run(state: Arc<ServerState>, stream: MaybeTls, user_id: UserId, peer: SocketAddr) {
    if let MaybeTls::Plain(ref s) = stream {
        let _ = s.set_nodelay(true);
    }
    let (read_half, write_half) = tokio::io::split(stream);

    let (out_tx, out_rx) = mpsc::channel::<Message>(OUTBOUND_QUEUE);
    let user = Arc::new(User::new(user_id, peer, out_tx));
    state.insert_user(user.clone());

    let writer = tokio::spawn(write_loop(write_half, out_rx));

    // K-line check: reject banned hosts before registration.
    let peer_host = peer.ip().to_string();
    if let Some(kl) = state.is_klined(&peer_host) {
        let mut params = Params::new();
        params.push_trailing(Bytes::from(
            format!("Closing Link: You are banned ({})", kl.reason).into_bytes(),
        ));
        user.send(Message {
            tags: Tags::new(),
            prefix: None,
            verb: Verb::word(Bytes::from_static(b"ERROR")),
            params,
        });
        // Give the writer a moment to flush, then tear down.
        drop(user);
        let _ = writer.await;
        return;
    }

    // Registration deadline: drop unregistered connections after the
    // configured timeout. Once registered the deadline is cancelled.
    let deadline_secs = state.config().limits.registration_deadline_seconds;
    let deadline = tokio::time::sleep(Duration::from_secs(deadline_secs));
    tokio::pin!(deadline);
    read_loop_with_deadline(state.clone(), user.clone(), read_half, &mut deadline).await;

    // Drop the user so the mpsc sender goes away. The write task sees
    // its receiver close and exits cleanly.
    drop(user);
    let _ = writer.await;
}

async fn read_loop_with_deadline(
    state: Arc<ServerState>,
    user: Arc<User>,
    read_half: tokio::io::ReadHalf<MaybeTls>,
    deadline: &mut std::pin::Pin<&mut tokio::time::Sleep>,
) {
    let limits = &state.config().limits;
    let mut bucket = FloodBucket::new(limits.messages_per_second, limits.messages_burst);
    let mut reader = FramedRead::new(read_half, IrcCodec::new());
    let mut registered = false;
    loop {
        let frame = read_frame(&mut reader, registered, deadline).await;
        let Some(frame) = frame else { return };
        match frame {
            Ok(message) => {
                if !process_message(&state, &user, message, &mut bucket, registered).await {
                    return;
                }
                if !registered && user.is_registered() {
                    registered = true;
                }
            }
            Err(e) => {
                warn!(error = %e, "read error");
                return;
            }
        }
    }
}

/// Read a single frame, respecting the registration deadline when not yet registered.
async fn read_frame(
    reader: &mut FramedRead<tokio::io::ReadHalf<MaybeTls>, IrcCodec>,
    registered: bool,
    deadline: &mut Pin<&mut tokio::time::Sleep>,
) -> Option<Result<Message, irc_proto::CodecError>> {
    if registered {
        reader.next().await
    } else {
        tokio::select! {
            biased;
            frame = reader.next() => frame,
            () = deadline.as_mut() => {
                warn!("registration deadline exceeded, disconnecting");
                None
            }
        }
    }
}

/// Process a single inbound message with flood control and dispatch.
///
/// Returns `true` to keep reading, `false` to disconnect.
async fn process_message(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    message: Message,
    bucket: &mut FloodBucket,
    registered: bool,
) -> bool {
    debug!(verb = ?message.verb, params = message.params.len(), "recv");
    if registered && !bucket.try_consume() {
        disconnect_flooding(user);
        return false;
    }
    let cmd_label = match &message.verb {
        irc_proto::Verb::Word(w) => std::str::from_utf8(w).unwrap_or("UNKNOWN").to_owned(),
        irc_proto::Verb::Numeric(n) => format!("{n:03}"),
    };
    crate::metrics::record_message(&cmd_label);
    match dispatch(state, user, message).await {
        Outcome::Continue => true,
        Outcome::Disconnect => {
            debug!("handler requested disconnect");
            false
        }
    }
}

/// Send a flood-detected ERROR and record the metric.
fn disconnect_flooding(user: &Arc<User>) {
    warn!("flood detected, disconnecting");
    crate::metrics::record_flood_kick();
    let error_msg = Message {
        tags: Tags::new(),
        prefix: None,
        verb: Verb::word(Bytes::from_static(b"ERROR")),
        params: {
            let mut p = Params::new();
            p.push_trailing(Bytes::from_static(b"Flooding"));
            p
        },
    };
    user.send(error_msg);
}

async fn write_loop(write_half: tokio::io::WriteHalf<MaybeTls>, mut rx: mpsc::Receiver<Message>) {
    let mut writer = FramedWrite::new(write_half, IrcCodec::new());
    while let Some(msg) = rx.recv().await {
        if let Err(e) = writer.send(msg).await {
            warn!(error = %e, "write error");
            return;
        }
    }
}

/// Broadcast QUIT to every channel peer and remove the user from
/// state. Safe to call for partially-registered users (it becomes a
/// no-op for anyone without a nick).
fn cleanup(state: &ServerState, user_id: UserId) {
    let Some(user) = state.user(user_id) else {
        return;
    };
    let nick = user.snapshot().nick;
    let peers = state.channel_peers(user_id);
    let origin = user.origin_prefix();
    drop(user);
    let _ = state.purge_user_from_channels(user_id);
    if !peers.is_empty() {
        let quit = quit_line(&origin, None);
        for uid in peers {
            if let Some(u) = state.user(uid) {
                u.send(quit.clone());
            }
        }
    }
    // MONITOR: notify watchers the nick went offline.
    if let Some(n) = &nick {
        crate::handler::monitor::notify_offline(state, n);
    }
    state.remove_user(user_id);
}

fn quit_line(origin: &Bytes, reason: Option<&Bytes>) -> Message {
    let mut params = Params::new();
    if let Some(r) = reason {
        params.push_trailing(r.clone());
    } else {
        params.push_trailing(Bytes::from_static(b"Connection closed"));
    }
    Message {
        tags: Tags::new(),
        prefix: Some(Prefix::User {
            nick: origin.clone(),
            user: None,
            host: None,
        }),
        verb: Verb::word(Bytes::from_static(b"QUIT")),
        params,
    }
}
