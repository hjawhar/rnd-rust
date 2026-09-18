//! Events emitted by the client to the frontend.

use std::net::Ipv4Addr;

use crate::dcc::TransferId;
use bytes::Bytes;

/// Opaque identifier for a network connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetworkId(
    /// The raw numeric identifier.
    pub u64,
);

/// An event produced by the IRC client for consumption by the frontend.
#[derive(Debug, Clone)]
pub enum ClientEvent {
    /// TCP (or TLS) connection established.
    Connected {
        /// Which network connected.
        network: NetworkId,
    },
    /// Connection lost or closed.
    Disconnected {
        /// Which network disconnected.
        network: NetworkId,
        /// Human-readable reason.
        reason: String,
    },
    /// Registration complete (`RPL_WELCOME` received).
    Registered {
        /// Which network.
        network: NetworkId,
        /// The nick confirmed by the server.
        nick: Bytes,
    },
    /// A PRIVMSG was received.
    Message {
        /// Which network.
        network: NetworkId,
        /// Channel or nick the message was sent to.
        target: Bytes,
        /// Sender nick.
        from: Bytes,
        /// Message body.
        text: Bytes,
    },
    /// A NOTICE was received.
    Notice {
        /// Which network.
        network: NetworkId,
        /// Channel or nick the notice was sent to.
        target: Bytes,
        /// Sender nick.
        from: Bytes,
        /// Notice body.
        text: Bytes,
    },
    /// A user joined a channel.
    Join {
        /// Which network.
        network: NetworkId,
        /// Channel name.
        channel: Bytes,
        /// Nick of the joiner.
        nick: Bytes,
    },
    /// A user left a channel.
    Part {
        /// Which network.
        network: NetworkId,
        /// Channel name.
        channel: Bytes,
        /// Nick of the parter.
        nick: Bytes,
        /// Optional part reason.
        reason: Option<Bytes>,
    },
    /// A user changed their nick.
    NickChange {
        /// Which network.
        network: NetworkId,
        /// Previous nick.
        old: Bytes,
        /// New nick.
        new_nick: Bytes,
    },
    /// Channel topic was changed.
    TopicChange {
        /// Which network.
        network: NetworkId,
        /// Channel name.
        channel: Bytes,
        /// New topic text.
        topic: Bytes,
    },
    /// A user quit the server.
    Quit {
        /// Which network.
        network: NetworkId,
        /// Nick of the quitter.
        nick: Bytes,
        /// Optional quit reason.
        reason: Option<Bytes>,
    },
    /// A numeric reply not handled by a more specific variant.
    Numeric {
        /// Which network.
        network: NetworkId,
        /// Three-digit numeric code.
        code: u16,
        /// Raw params from the numeric.
        params: Vec<Bytes>,
    },
    /// An error condition on the network.
    Error {
        /// Which network.
        network: NetworkId,
        /// Description of the error.
        message: String,
    },
    /// A single entry from a LIST reply.
    ListEntry {
        /// Which network.
        network: NetworkId,
        /// Channel name.
        channel: Bytes,
        /// Number of visible users.
        user_count: u32,
        /// Channel topic (may be empty).
        topic: Bytes,
    },
    /// End of LIST reply.
    ListEnd {
        /// Which network.
        network: NetworkId,
    },
    /// A DCC CHAT request was received.
    DccChatRequest {
        /// Which network.
        network: NetworkId,
        /// Who sent the request.
        from: Bytes,
        /// Peer IP address.
        ip: Ipv4Addr,
        /// Peer TCP port.
        port: u16,
    },
    /// A DCC SEND request was received.
    DccSendRequest {
        /// Which network.
        network: NetworkId,
        /// Who sent the request.
        from: Bytes,
        /// Offered filename.
        filename: String,
        /// Peer IP address.
        ip: Ipv4Addr,
        /// Peer TCP port.
        port: u16,
        /// File size in bytes.
        size: u64,
    },
    /// Progress update for an ongoing DCC transfer.
    DccProgress {
        /// Transfer identifier.
        transfer_id: TransferId,
        /// Bytes transferred so far.
        bytes_transferred: u64,
        /// Total expected bytes.
        total: u64,
    },
    /// A DCC transfer completed.
    DccComplete {
        /// Transfer identifier.
        transfer_id: TransferId,
    },
}
