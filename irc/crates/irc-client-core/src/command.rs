//! Commands accepted from the frontend to the client.

use std::net::Ipv4Addr;
use std::path::PathBuf;

use bytes::Bytes;

use crate::event::NetworkId;

/// A command from the frontend to the IRC client.
#[derive(Debug, Clone)]
pub enum ClientCommand {
    /// Open a new connection to a network.
    Connect {
        /// Target network identifier.
        network: NetworkId,
        /// Hostname or IP.
        host: String,
        /// Port number.
        port: u16,
        /// Whether to use TLS.
        tls: bool,
        /// Desired nick.
        nick: Bytes,
        /// Username for USER command.
        user: Bytes,
        /// Realname for USER command.
        realname: Bytes,
    },
    /// Disconnect from a network.
    Disconnect {
        /// Target network.
        network: NetworkId,
    },
    /// Send a raw IRC line.
    SendRaw {
        /// Target network.
        network: NetworkId,
        /// Raw IRC line bytes.
        line: Bytes,
    },
    /// Send a PRIVMSG.
    SendPrivmsg {
        /// Target network.
        network: NetworkId,
        /// Target channel or nick.
        target: Bytes,
        /// Message text.
        text: Bytes,
    },
    /// Send a NOTICE.
    SendNotice {
        /// Target network.
        network: NetworkId,
        /// Target channel or nick.
        target: Bytes,
        /// Notice text.
        text: Bytes,
    },
    /// Join a channel.
    Join {
        /// Target network.
        network: NetworkId,
        /// Channel name.
        channel: Bytes,
    },
    /// Part from a channel.
    Part {
        /// Target network.
        network: NetworkId,
        /// Channel name.
        channel: Bytes,
        /// Optional part reason.
        reason: Option<Bytes>,
    },
    /// Change nick.
    ChangeNick {
        /// Target network.
        network: NetworkId,
        /// New nick.
        nick: Bytes,
    },
    /// Set channel topic.
    SetTopic {
        /// Target network.
        network: NetworkId,
        /// Channel name.
        channel: Bytes,
        /// New topic.
        topic: Bytes,
    },
    /// Quit from a network.
    Quit {
        /// Target network.
        network: NetworkId,
        /// Optional quit reason.
        reason: Option<Bytes>,
    },
    /// Request the channel list from the server.
    List {
        /// Target network.
        network: NetworkId,
    },
    /// Accept a DCC CHAT request.
    DccAcceptChat {
        /// Target network.
        network: NetworkId,
        /// Nick of the peer.
        nick: Bytes,
        /// Peer IP address.
        ip: Ipv4Addr,
        /// Peer TCP port.
        port: u16,
    },
    /// Accept a DCC SEND request.
    DccAcceptSend {
        /// Target network.
        network: NetworkId,
        /// Nick of the peer.
        nick: Bytes,
        /// Peer IP address.
        ip: Ipv4Addr,
        /// Peer TCP port.
        port: u16,
        /// Offered filename.
        filename: String,
        /// File size in bytes.
        size: u64,
        /// Local path to save the file.
        save_path: PathBuf,
    },
}
