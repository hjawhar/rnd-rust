//! DCC (Direct Client-to-Client) connection management.
//!
//! Handles incoming and outgoing DCC CHAT and SEND sessions over raw TCP.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Unique identifier for a DCC transfer/session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransferId(u64);

impl TransferId {
    /// Return the raw numeric identifier.
    #[must_use]
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for TransferId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "dcc-{}", self.0)
    }
}

/// Progress updates emitted during DCC transfers.
#[derive(Debug, Clone)]
pub enum DccProgress {
    /// Bytes transferred so far.
    Progress {
        /// Transfer identifier.
        id: TransferId,
        /// Bytes transferred.
        bytes_transferred: u64,
        /// Total expected bytes (0 if unknown).
        total: u64,
    },
    /// Transfer completed successfully.
    Complete {
        /// Transfer identifier.
        id: TransferId,
    },
    /// Transfer failed.
    Error {
        /// Transfer identifier.
        id: TransferId,
        /// Error description.
        message: String,
    },
    /// A line received on a DCC CHAT session.
    ChatLine {
        /// Transfer identifier.
        id: TransferId,
        /// The line of text received.
        line: String,
    },
}

/// Pending DCC offer received from a remote peer.
#[derive(Debug, Clone)]
pub struct PendingOffer {
    /// Who sent the offer.
    pub from: String,
    /// The parsed request.
    pub request: irc_proto::dcc::DccRequest,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_transfer_id() -> TransferId {
    TransferId(NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

/// DCC connection manager.
///
/// Tracks pending offers and spawns async tasks for accepted connections.
pub struct DccManager {
    pending: Vec<PendingOffer>,
    progress_tx: mpsc::UnboundedSender<DccProgress>,
    progress_rx: Option<mpsc::UnboundedReceiver<DccProgress>>,
}

impl DccManager {
    /// Create a new DCC manager.
    #[must_use]
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            pending: Vec::new(),
            progress_tx: tx,
            progress_rx: Some(rx),
        }
    }

    /// Take the progress receiver. Can only be called once.
    pub fn take_progress_rx(&mut self) -> Option<mpsc::UnboundedReceiver<DccProgress>> {
        self.progress_rx.take()
    }

    /// Record a pending DCC offer from a remote peer.
    pub fn add_pending(&mut self, offer: PendingOffer) {
        self.pending.push(offer);
    }

    /// Return a reference to pending offers.
    #[must_use]
    pub fn pending_offers(&self) -> &[PendingOffer] {
        &self.pending
    }

    /// Accept a DCC CHAT request — connect to the peer and relay lines.
    ///
    /// Returns the transfer ID and a task handle. Lines are reported via
    /// [`DccProgress::ChatLine`].
    pub fn accept_chat(&self, ip: Ipv4Addr, port: u16) -> (TransferId, JoinHandle<()>) {
        let id = next_transfer_id();
        let tx = self.progress_tx.clone();
        let handle = tokio::spawn(async move {
            let result = run_chat_client(id, ip, port, &tx).await;
            if let Err(e) = result {
                let _ = tx.send(DccProgress::Error {
                    id,
                    message: e.to_string(),
                });
            }
        });
        (id, handle)
    }

    /// Accept a DCC SEND request — connect to the peer and receive the file.
    pub fn accept_send(
        &self,
        ip: Ipv4Addr,
        port: u16,
        save_path: &Path,
        size: u64,
    ) -> (TransferId, JoinHandle<()>) {
        let id = next_transfer_id();
        let tx = self.progress_tx.clone();
        let path = save_path.to_owned();
        let handle = tokio::spawn(async move {
            let result = run_recv_file(id, ip, port, &path, size, &tx).await;
            if let Err(e) = result {
                let _ = tx.send(DccProgress::Error {
                    id,
                    message: e.to_string(),
                });
            }
        });
        (id, handle)
    }

    /// Offer a DCC CHAT session — bind a listener and return the CTCP args
    /// the caller should send to the peer.
    ///
    /// # Errors
    ///
    /// Returns an error if the TCP listener cannot bind.
    pub async fn offer_chat(
        &self,
        bind_port: u16,
    ) -> std::io::Result<(TransferId, irc_proto::dcc::DccRequest, JoinHandle<()>)> {
        let listener = TcpListener::bind(("0.0.0.0", bind_port)).await?;
        let local_addr = listener.local_addr()?;
        let ip = Ipv4Addr::UNSPECIFIED; // caller should replace with external IP
        let port = local_addr.port();
        let request = irc_proto::dcc::DccRequest::Chat { ip, port };

        let id = next_transfer_id();
        let tx = self.progress_tx.clone();
        let handle = tokio::spawn(async move {
            let result = run_chat_server(id, listener, &tx).await;
            if let Err(e) = result {
                let _ = tx.send(DccProgress::Error {
                    id,
                    message: e.to_string(),
                });
            }
        });
        Ok((id, request, handle))
    }

    /// Offer a DCC SEND — bind a listener and return the CTCP args.
    ///
    /// # Errors
    ///
    /// Returns an error if the TCP listener cannot bind or the file cannot be read.
    pub async fn offer_send(
        &self,
        path: &Path,
        bind_port: u16,
    ) -> std::io::Result<(TransferId, irc_proto::dcc::DccRequest, JoinHandle<()>)> {
        let metadata = tokio::fs::metadata(path).await?;
        let size = metadata.len();
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        let listener = TcpListener::bind(("0.0.0.0", bind_port)).await?;
        let local_addr = listener.local_addr()?;
        let ip = Ipv4Addr::UNSPECIFIED;
        let port = local_addr.port();
        let request = irc_proto::dcc::DccRequest::Send {
            filename,
            ip,
            port,
            size,
        };

        let id = next_transfer_id();
        let tx = self.progress_tx.clone();
        let file_path = path.to_owned();
        let handle = tokio::spawn(async move {
            let result = run_send_file(id, listener, &file_path, size, &tx).await;
            if let Err(e) = result {
                let _ = tx.send(DccProgress::Error {
                    id,
                    message: e.to_string(),
                });
            }
        });
        Ok((id, request, handle))
    }
}

impl Default for DccManager {
    fn default() -> Self {
        Self::new()
    }
}

// -- internal async tasks --

const BUF_SIZE: usize = 8192;

async fn run_chat_client(
    id: TransferId,
    ip: Ipv4Addr,
    port: u16,
    tx: &mpsc::UnboundedSender<DccProgress>,
) -> std::io::Result<()> {
    let mut stream = TcpStream::connect((ip, port)).await?;
    let mut buf = vec![0u8; BUF_SIZE];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            let _ = tx.send(DccProgress::Complete { id });
            return Ok(());
        }
        let line = String::from_utf8_lossy(&buf[..n]).into_owned();
        let _ = tx.send(DccProgress::ChatLine { id, line });
    }
}

async fn run_chat_server(
    id: TransferId,
    listener: TcpListener,
    tx: &mpsc::UnboundedSender<DccProgress>,
) -> std::io::Result<()> {
    let (mut stream, _addr) = listener.accept().await?;
    let mut buf = vec![0u8; BUF_SIZE];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            let _ = tx.send(DccProgress::Complete { id });
            return Ok(());
        }
        let line = String::from_utf8_lossy(&buf[..n]).into_owned();
        let _ = tx.send(DccProgress::ChatLine { id, line });
    }
}

async fn run_recv_file(
    id: TransferId,
    ip: Ipv4Addr,
    port: u16,
    save_path: &PathBuf,
    total: u64,
    tx: &mpsc::UnboundedSender<DccProgress>,
) -> std::io::Result<()> {
    let mut stream = TcpStream::connect((ip, port)).await?;
    let mut file = tokio::fs::File::create(save_path).await?;
    let mut buf = vec![0u8; BUF_SIZE];
    let mut received: u64 = 0;

    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).await?;
        received += n as u64;

        // DCC SEND: acknowledge received bytes as big-endian u32.
        #[allow(clippy::cast_possible_truncation)]
        let ack = (received & 0xFFFF_FFFF) as u32;
        stream.write_all(&ack.to_be_bytes()).await?;

        let _ = tx.send(DccProgress::Progress {
            id,
            bytes_transferred: received,
            total,
        });
    }

    file.flush().await?;
    let _ = tx.send(DccProgress::Complete { id });
    Ok(())
}

async fn run_send_file(
    id: TransferId,
    listener: TcpListener,
    path: &PathBuf,
    total: u64,
    tx: &mpsc::UnboundedSender<DccProgress>,
) -> std::io::Result<()> {
    let (mut stream, _addr) = listener.accept().await?;
    let mut file = tokio::fs::File::open(path).await?;
    let mut buf = vec![0u8; BUF_SIZE];
    let mut sent: u64 = 0;

    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        stream.write_all(&buf[..n]).await?;
        sent += n as u64;

        let _ = tx.send(DccProgress::Progress {
            id,
            bytes_transferred: sent,
            total,
        });
    }

    let _ = tx.send(DccProgress::Complete { id });
    Ok(())
}
