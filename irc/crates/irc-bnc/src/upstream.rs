use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio_util::codec::Framed;
use tracing::{debug, error, info, warn};

use irc_proto::{Command, IrcCodec, Message};

use crate::buffer::MessageBuffer;
use crate::config::BncNetwork;

/// Shared state for a single upstream connection.
pub struct UpstreamState {
    /// Channels currently joined on this upstream.
    pub joined_channels: HashSet<Vec<u8>>,
    /// Per-target message buffers (channel or nick).
    pub buffers: HashMap<Vec<u8>, MessageBuffer>,
    /// Whether registration completed (`RPL_WELCOME` received).
    pub registered: bool,
    /// Our current nick on this upstream.
    pub nick: Bytes,
}

impl UpstreamState {
    /// Create initial state from network config.
    pub fn new(network: &BncNetwork) -> Self {
        Self {
            joined_channels: HashSet::new(),
            buffers: HashMap::new(),
            registered: false,
            nick: Bytes::from(network.nick.clone()),
        }
    }
}

/// Manages a single upstream IRC connection.
pub struct Upstream {
    config: BncNetwork,
    state: parking_lot::Mutex<UpstreamState>,
    /// Broadcast channel for forwarding live traffic to downstream clients.
    tx: broadcast::Sender<Message>,
}

impl Upstream {
    /// Create a new upstream manager for the given network config.
    pub fn new(config: BncNetwork) -> Self {
        let (tx, _) = broadcast::channel(256);
        let state = parking_lot::Mutex::new(UpstreamState::new(&config));
        Self { config, state, tx }
    }

    /// Subscribe to the live message broadcast.
    pub fn subscribe(&self) -> broadcast::Receiver<Message> {
        self.tx.subscribe()
    }

    /// Borrow the upstream state under lock.
    pub fn state(&self) -> parking_lot::MutexGuard<'_, UpstreamState> {
        self.state.lock()
    }

    /// Run the upstream connection loop. Connects, registers, and processes
    /// incoming messages indefinitely. Returns on disconnect.
    #[allow(clippy::cognitive_complexity)]
    pub async fn run(self: &std::sync::Arc<Self>) -> anyhow::Result<()> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        info!(addr = %addr, "connecting to upstream");

        let stream = TcpStream::connect(&addr).await?;
        let mut framed = Framed::new(stream, IrcCodec::new());

        // Register
        let nick_msg = Command::Nick {
            nick: Bytes::from(self.config.nick.clone()),
        }
        .to_message();
        framed.send(nick_msg).await?;

        let user_msg = Command::User {
            user: Bytes::from(self.config.user.clone()),
            mode: Bytes::from_static(b"0"),
            realname: Bytes::from(self.config.realname.clone()),
        }
        .to_message();
        framed.send(user_msg).await?;

        // Main read loop
        while let Some(result) = framed.next().await {
            let msg = match result {
                Ok(m) => m,
                Err(e) => {
                    error!(error = %e, "upstream codec error");
                    break;
                }
            };

            self.process_incoming(&msg, &mut framed).await?;
        }

        warn!("upstream disconnected");
        Ok(())
    }

    async fn process_incoming(
        &self,
        msg: &Message,
        framed: &mut Framed<TcpStream, IrcCodec>,
    ) -> anyhow::Result<()> {
        let Ok(cmd) = Command::parse(msg) else {
            // Forward as-is for unknown commands
            let _ = self.tx.send(msg.clone());
            return Ok(());
        };

        match cmd {
            Command::Ping { token, .. } => {
                let pong = Command::Pong {
                    token,
                    server: None,
                }
                .to_message();
                framed.send(pong).await?;
            }
            Command::Numeric { code: 1, .. } => {
                // RPL_WELCOME
                self.state.lock().registered = true;
                info!("upstream registered");
                let _ = self.tx.send(msg.clone());
            }
            Command::Privmsg { targets, text } | Command::Notice { targets, text } => {
                self.buffer_message(&targets, msg);
                let _ = self.tx.send(msg.clone());
                debug!(targets = ?targets.iter().map(|t| String::from_utf8_lossy(t).into_owned()).collect::<Vec<_>>(), text = %String::from_utf8_lossy(&text), "buffered message");
            }
            Command::Join { channels, .. } => {
                {
                    let mut state = self.state.lock();
                    for ch in &channels {
                        state.joined_channels.insert(ch.to_vec());
                    }
                }
                let _ = self.tx.send(msg.clone());
            }
            Command::Part { channels, .. } => {
                {
                    let mut state = self.state.lock();
                    for ch in &channels {
                        state.joined_channels.remove(ch.as_ref());
                    }
                }
                let _ = self.tx.send(msg.clone());
            }
            _ => {
                let _ = self.tx.send(msg.clone());
            }
        }

        Ok(())
    }

    fn buffer_message(&self, targets: &[Bytes], msg: &Message) {
        let now = timestamp_now();
        let mut state = self.state.lock();
        for target in targets {
            state
                .buffers
                .entry(target.to_vec())
                .or_default()
                .push(now.clone(), msg.clone());
        }
    }
}

/// Produce an ISO 8601 UTC timestamp as Bytes.
fn timestamp_now() -> Bytes {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    Bytes::from(format_timestamp(now.as_secs()))
}

#[allow(clippy::similar_names)] // doe/doy are from the canonical algorithm
fn format_timestamp(epoch_secs: u64) -> String {
    let secs_per_day: u64 = 86400;
    let total_days = epoch_secs / secs_per_day;
    let day_secs = epoch_secs % secs_per_day;
    let hours = day_secs / 3600;
    let minutes = (day_secs % 3600) / 60;
    let seconds = day_secs % 60;

    let (year, month, day) = days_to_date(total_days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

#[allow(clippy::similar_names)] // doe/doy are from the canonical Hinnant algorithm
fn days_to_date(days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
