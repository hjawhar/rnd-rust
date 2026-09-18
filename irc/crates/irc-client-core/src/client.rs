//! Multi-network client manager.

use std::collections::HashMap;

use tokio::sync::mpsc;
use tracing::warn;

use crate::command::ClientCommand;
use crate::event::{ClientEvent, NetworkId};
use crate::network::{self, NetworkHandle};

/// Channel buffer size for internal event/command channels.
const CHANNEL_BUF: usize = 256;

/// Multi-network IRC client.
///
/// The client owns the event loop that routes commands from the frontend to
/// the appropriate network session and collects events from all sessions
/// into a single event stream.
pub struct Client {
    event_tx: mpsc::Sender<ClientEvent>,
    command_rx: mpsc::Receiver<ClientCommand>,
    networks: HashMap<NetworkId, NetworkHandle>,
}

impl Client {
    /// Create a new client, returning the client, the event receiver the
    /// frontend reads from, and the command sender the frontend writes to.
    #[must_use]
    pub fn new() -> (
        Self,
        mpsc::Receiver<ClientEvent>,
        mpsc::Sender<ClientCommand>,
    ) {
        let (event_tx, event_rx) = mpsc::channel(CHANNEL_BUF);
        let (command_tx, command_rx) = mpsc::channel(CHANNEL_BUF);
        let client = Self {
            event_tx,
            command_rx,
            networks: HashMap::new(),
        };
        (client, event_rx, command_tx)
    }

    /// Run the client event loop. This processes commands from the frontend
    /// and routes them to the appropriate network session. Returns when the
    /// command channel is closed.
    pub async fn run(&mut self) {
        while let Some(cmd) = self.command_rx.recv().await {
            self.handle_command(cmd).await;
        }
        // Shut down all networks when the command channel closes.
        self.networks.clear();
    }

    async fn handle_command(&mut self, cmd: ClientCommand) {
        match &cmd {
            ClientCommand::Connect {
                network,
                host,
                port,
                tls,
                nick,
                user,
                realname,
            } => {
                let id = *network;
                let (net_cmd_tx, net_cmd_rx) = mpsc::channel(CHANNEL_BUF);
                let event_tx = self.event_tx.clone();
                let config = network::NetworkConfig {
                    id,
                    host: host.clone(),
                    port: *port,
                    tls: *tls,
                    nick: nick.clone(),
                    user: user.clone(),
                    realname: realname.clone(),
                };

                let join_handle = tokio::spawn(async move {
                    network::run_network(config, event_tx, net_cmd_rx).await;
                });

                self.networks.insert(
                    id,
                    NetworkHandle {
                        command_tx: net_cmd_tx,
                        join_handle,
                    },
                );
            }
            ClientCommand::Disconnect { network } => {
                if let Some(handle) = self.networks.remove(network) {
                    // Drop the command sender, which causes the session to exit.
                    drop(handle.command_tx);
                    // We don't await the join handle here to avoid blocking
                    // the event loop; the task will clean up on its own.
                }
            }
            other => {
                let network_id = *command_network_id(other);
                if let Some(handle) = self.networks.get(&network_id) {
                    if handle.command_tx.send(cmd).await.is_err() {
                        warn!("network {} command channel closed, removing", network_id.0);
                        self.networks.remove(&network_id);
                    }
                } else {
                    warn!("command for unknown network {}", network_id.0);
                }
            }
        }
    }
}

/// Extract the network id from any command variant.
fn command_network_id(cmd: &ClientCommand) -> &NetworkId {
    match cmd {
        ClientCommand::Connect { network, .. }
        | ClientCommand::Disconnect { network }
        | ClientCommand::SendRaw { network, .. }
        | ClientCommand::SendPrivmsg { network, .. }
        | ClientCommand::SendNotice { network, .. }
        | ClientCommand::Join { network, .. }
        | ClientCommand::Part { network, .. }
        | ClientCommand::ChangeNick { network, .. }
        | ClientCommand::SetTopic { network, .. }
        | ClientCommand::Quit { network, .. }
        | ClientCommand::List { network, .. }
        | ClientCommand::DccAcceptChat { network, .. }
        | ClientCommand::DccAcceptSend { network, .. } => network,
    }
}
