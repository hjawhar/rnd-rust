use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use iced::widget::{column, container, row, text};
use iced::{Element, Fill, Subscription, Task, Theme};
use tokio::sync::mpsc;
use tracing::{info, warn};

use irc_client_core::{Client, ClientCommand, ClientEvent, NetworkId};

use crate::theme;
use crate::theme::ThemeChoice;
use crate::views;
use crate::views::connect_dialog::{ConnectField, ConnectForm};

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WindowId(u64);

static NEXT_WINDOW_ID: AtomicU64 = AtomicU64::new(1);

fn next_window_id() -> WindowId {
    WindowId(NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed))
}

static NEXT_NETWORK_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_network_id() -> NetworkId {
    NetworkId(NEXT_NETWORK_COUNTER.fetch_add(1, Ordering::Relaxed))
}

// ---------------------------------------------------------------------------
// Display types
// ---------------------------------------------------------------------------

pub(crate) struct DisplayMessage {
    pub(crate) timestamp: String,
    pub(crate) from: String,
    pub(crate) text: String,
    pub(crate) is_action: bool,
}

/// A window: either a "Status" window for a server, a channel, or a
/// private query.
pub(crate) struct Window {
    pub(crate) id: WindowId,
    pub(crate) network: NetworkId,
    /// Empty for the status window, channel/nick name otherwise.
    pub(crate) target: Bytes,
    pub(crate) messages: Vec<DisplayMessage>,
    pub(crate) topic: Option<String>,
    pub(crate) nicks: Vec<String>,
    /// True if this is the per-server status window.
    pub(crate) is_status: bool,
}

/// Summary info for a connected server shown in the treebar.
pub(crate) struct NetworkInfo {
    pub(crate) name: String,
    pub(crate) nick: String,
    pub(crate) connected: bool,
    pub(crate) status_window: WindowId,
    pub(crate) windows: Vec<WindowRef>,
}

pub(crate) struct WindowRef {
    pub(crate) id: WindowId,
    pub(crate) target: Bytes,
}

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

/// The view the app is currently showing.
enum ViewState {
    /// Show the connect dialog (no servers connected yet, or user
    /// pressed File > Connect).
    ConnectDialog,
    /// Normal IRC view with treebar + scrollback + nicklist.
    Irc,
}

pub(crate) struct IrcApp {
    command_tx: mpsc::Sender<ClientCommand>,
    active_window: Option<WindowId>,
    windows: HashMap<WindowId, Window>,
    networks: HashMap<NetworkId, NetworkInfo>,
    input_value: String,
    view_state: ViewState,
    connect_form: ConnectForm,
    own_nick: HashMap<NetworkId, String>,

    // Channel list
    channel_list_entries: Vec<views::channel_list::ListEntry>,
    channel_list_filter: String,
    channel_list_loading: bool,
    show_channel_list: bool,
    /// Current light/dark theme selection.
    theme_choice: ThemeChoice,
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) enum Msg {
    IrcEvent(ClientEvent),
    InputChanged(String),
    InputSubmit,
    WindowSelected(WindowId),
    ConnectFormChanged(ConnectField),
    ConnectSubmit,
    ConnectCancel,
    ShowConnectDialog,
    ListFilterChanged(String),
    ListJoinChannel(Bytes),
    #[allow(dead_code)]
    ListClose,
    #[allow(dead_code)]
    Noop,
    /// Toggle between light and dark themes.
    ToggleTheme,
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

impl IrcApp {
    pub(crate) fn new() -> (Self, Task<Msg>) {
        let (client, event_rx, command_tx) = Client::new();
        park_event_rx(event_rx);

        tokio::spawn(async move {
            let mut c = client;
            c.run().await;
        });

        let app = Self {
            command_tx,
            active_window: None,
            windows: HashMap::new(),
            networks: HashMap::new(),
            input_value: String::new(),
            view_state: ViewState::ConnectDialog,
            connect_form: ConnectForm::default_local(),
            own_nick: HashMap::new(),
            channel_list_entries: Vec::new(),
            channel_list_filter: String::new(),
            channel_list_loading: false,
            show_channel_list: false,
            theme_choice: ThemeChoice::default(),
        };

        (app, Task::none())
    }
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

impl IrcApp {
    pub(crate) fn update(&mut self, message: Msg) -> Task<Msg> {
        match message {
            Msg::IrcEvent(event) => self.handle_event(event),
            Msg::InputChanged(value) => self.input_value = value,
            Msg::InputSubmit => self.submit_input(),
            Msg::WindowSelected(id) => {
                if self.windows.contains_key(&id) {
                    self.active_window = Some(id);
                }
            }
            Msg::ConnectFormChanged(field) => match field {
                ConnectField::Host(v) => self.connect_form.host = v,
                ConnectField::Port(v) => self.connect_form.port = v,
                ConnectField::Nick(v) => self.connect_form.nick = v,
                ConnectField::User(v) => self.connect_form.user = v,
                ConnectField::Realname(v) => self.connect_form.realname = v,
                ConnectField::Tls(v) => self.connect_form.tls = v,
            },
            Msg::ConnectSubmit => self.do_connect(),
            Msg::ConnectCancel => {
                if !self.networks.is_empty() {
                    self.view_state = ViewState::Irc;
                }
            }
            Msg::ShowConnectDialog => {
                self.connect_form = ConnectForm::default_local();
                self.view_state = ViewState::ConnectDialog;
            }
            Msg::ListFilterChanged(v) => self.channel_list_filter = v,
            Msg::ListJoinChannel(chan) => {
                if let Some(net_id) = self.active_network() {
                    self.send_cmd(ClientCommand::Join {
                        network: net_id,
                        channel: chan,
                    });
                }
                self.show_channel_list = false;
            }
            Msg::ListClose => self.show_channel_list = false,
            Msg::Noop => {}
            Msg::ToggleTheme => {
                self.theme_choice = self.theme_choice.toggle();
            }
        }
        Task::none()
    }

    fn do_connect(&mut self) {
        let form = &self.connect_form;
        let port: u16 = form.port.parse().unwrap_or(6667);
        let net_id = next_network_id();

        // Create the status window for this server.
        let status_id = next_window_id();
        self.windows.insert(
            status_id,
            Window {
                id: status_id,
                network: net_id,
                target: Bytes::from_static(b"Status"),
                messages: vec![DisplayMessage {
                    timestamp: now_stamp(),
                    from: String::from("*"),
                    text: format!("Connecting to {}:{}...", form.host, port),
                    is_action: false,
                }],
                topic: None,
                nicks: Vec::new(),
                is_status: true,
            },
        );

        self.networks.insert(
            net_id,
            NetworkInfo {
                name: format!("{}:{}", form.host, port),
                nick: form.nick.clone(),
                connected: false,
                status_window: status_id,
                windows: Vec::new(),
            },
        );

        self.own_nick.insert(net_id, form.nick.clone());
        self.active_window = Some(status_id);
        self.view_state = ViewState::Irc;

        let cmd = ClientCommand::Connect {
            network: net_id,
            host: form.host.clone(),
            port,
            tls: form.tls,
            nick: Bytes::from(form.nick.clone().into_bytes()),
            user: Bytes::from(form.user.clone().into_bytes()),
            realname: Bytes::from(form.realname.clone().into_bytes()),
        };
        self.send_cmd(cmd);
        info!(host = %form.host, port, nick = %form.nick, "connect requested");
    }

    #[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
    fn handle_event(&mut self, event: ClientEvent) {
        match event {
            ClientEvent::Connected { network } => {
                if let Some(info) = self.networks.get_mut(&network) {
                    info.connected = true;
                }
                self.push_status(network, "Connected.");
            }
            ClientEvent::Disconnected { network, reason } => {
                if let Some(info) = self.networks.get_mut(&network) {
                    info.connected = false;
                }
                self.push_status(network, &format!("Disconnected: {reason}"));
            }
            ClientEvent::Registered { network, nick } => {
                let nick_str = String::from_utf8_lossy(&nick).into_owned();
                if let Some(info) = self.networks.get_mut(&network) {
                    nick_str.clone_into(&mut info.nick);
                    nick_str.clone_into(&mut info.name);
                }
                self.own_nick.insert(network, nick_str.clone());
                self.push_status(network, &format!("Registered as {nick_str}"));
            }
            ClientEvent::Message {
                network,
                target,
                from,
                text,
            } => {
                let from_str = String::from_utf8_lossy(&from).into_owned();
                let text_str = String::from_utf8_lossy(&text).into_owned();
                let own = self.own_nick.get(&network).cloned().unwrap_or_default();
                // If target is our nick, it's a PM — use the sender as the window key.
                let win_target = if target.as_ref() == own.as_bytes() {
                    from.clone()
                } else {
                    target
                };
                let win_id = self.ensure_window(network, win_target);
                if let Some(win) = self.windows.get_mut(&win_id) {
                    win.messages.push(DisplayMessage {
                        timestamp: now_stamp(),
                        from: from_str.clone(),
                        text: text_str.clone(),
                        is_action: false,
                    });
                }
                // Notification
                let is_pm = from.as_ref() != own.as_bytes()
                    && self
                        .windows
                        .get(&win_id)
                        .is_some_and(|w| !w.target.as_ref().starts_with(b"#"));
                let is_highlight = !is_pm
                    && text_str
                        .to_ascii_lowercase()
                        .contains(&own.to_ascii_lowercase());
                if is_pm || is_highlight {
                    crate::notifications::notify_message(&from_str, &text_str, is_pm);
                }
            }
            ClientEvent::Notice {
                network,
                target,
                from,
                text,
            } => {
                let from_str = String::from_utf8_lossy(&from).into_owned();
                let text_str = String::from_utf8_lossy(&text).into_owned();
                // Server notices go to the status window.
                if target.as_ref() == b"*" || from.is_empty() {
                    self.push_status(network, &format!("-{from_str}- {text_str}"));
                } else {
                    let win_id = self.ensure_window(network, target);
                    if let Some(win) = self.windows.get_mut(&win_id) {
                        win.messages.push(DisplayMessage {
                            timestamp: now_stamp(),
                            from: from_str,
                            text: format!("-NOTICE- {text_str}"),
                            is_action: false,
                        });
                    }
                }
            }
            ClientEvent::Join {
                network,
                channel,
                nick,
            } => {
                let nick_str = String::from_utf8_lossy(&nick).into_owned();
                let own = self.own_nick.get(&network).cloned().unwrap_or_default();
                let is_self = nick_str == own;
                let win_id = self.ensure_window(network, channel.clone());
                if let Some(win) = self.windows.get_mut(&win_id) {
                    if !win.nicks.contains(&nick_str) {
                        win.nicks.push(nick_str.clone());
                        win.nicks.sort();
                    }
                    win.messages.push(DisplayMessage {
                        timestamp: now_stamp(),
                        from: String::from("-->"),
                        text: format!("{nick_str} has joined"),
                        is_action: true,
                    });
                }
                // Show welcome guidelines when WE join a channel.
                if is_self {
                    self.show_welcome_guidelines(network, channel.as_ref());
                }
            }
            ClientEvent::Part {
                network,
                channel,
                nick,
                reason,
            } => {
                let nick_str = String::from_utf8_lossy(&nick).into_owned();
                let win_id = self.ensure_window(network, channel);
                if let Some(win) = self.windows.get_mut(&win_id) {
                    win.nicks.retain(|n| n != &nick_str);
                    let reason_str = reason
                        .as_ref()
                        .map(|r| format!(" ({})", String::from_utf8_lossy(r)))
                        .unwrap_or_default();
                    win.messages.push(DisplayMessage {
                        timestamp: now_stamp(),
                        from: String::from("<--"),
                        text: format!("{nick_str} has left{reason_str}"),
                        is_action: true,
                    });
                }
            }
            ClientEvent::TopicChange {
                network,
                channel,
                topic,
            } => {
                let topic_str = String::from_utf8_lossy(&topic).into_owned();
                let win_id = self.ensure_window(network, channel);
                if let Some(win) = self.windows.get_mut(&win_id) {
                    win.topic = Some(topic_str);
                }
            }
            ClientEvent::Numeric {
                network,
                code,
                params,
            } => {
                // Show all numerics in the status window.
                let text = params
                    .iter()
                    .skip(1) // skip our nick
                    .map(|p| String::from_utf8_lossy(p).into_owned())
                    .collect::<Vec<_>>()
                    .join(" ");
                self.push_status(network, &format!("[{code:03}] {text}"));
            }
            ClientEvent::ListEntry {
                network: _,
                channel,
                user_count,
                topic,
            } => {
                self.channel_list_entries
                    .push(views::channel_list::ListEntry {
                        channel,
                        user_count,
                        topic: String::from_utf8_lossy(&topic).into_owned(),
                    });
            }
            ClientEvent::ListEnd { .. } => {
                self.channel_list_loading = false;
            }
            ClientEvent::NickChange {
                network,
                old,
                new_nick,
            } => {
                let old_str = String::from_utf8_lossy(&old).into_owned();
                let new_str = String::from_utf8_lossy(&new_nick).into_owned();
                let own = self.own_nick.get(&network).cloned().unwrap_or_default();
                if old_str == own {
                    self.own_nick.insert(network, new_str.clone());
                    if let Some(info) = self.networks.get_mut(&network) {
                        new_str.clone_into(&mut info.nick);
                    }
                }
                // Update nick lists in all windows of this network.
                for win in self.windows.values_mut() {
                    if win.network == network {
                        if let Some(pos) = win.nicks.iter().position(|n| n == &old_str) {
                            win.nicks[pos].clone_from(&new_str);
                            win.nicks.sort();
                        }
                    }
                }
                self.push_status(network, &format!("{old_str} is now known as {new_str}"));
            }
            ClientEvent::Quit {
                network,
                nick,
                reason,
            } => {
                let nick_str = String::from_utf8_lossy(&nick).into_owned();
                let reason_str = reason
                    .as_ref()
                    .map(|r| format!(" ({})", String::from_utf8_lossy(r)))
                    .unwrap_or_default();
                for win in self.windows.values_mut() {
                    if win.network == network {
                        win.nicks.retain(|n| n != &nick_str);
                    }
                }
                self.push_status(network, &format!("{nick_str} has quit{reason_str}"));
            }
            ClientEvent::Error { network, message } => {
                self.push_status(network, &format!("ERROR: {message}"));
            }
            ClientEvent::DccChatRequest { .. }
            | ClientEvent::DccSendRequest { .. }
            | ClientEvent::DccProgress { .. }
            | ClientEvent::DccComplete { .. } => {}
        }
    }

    #[allow(clippy::too_many_lines)]
    fn submit_input(&mut self) {
        let text = std::mem::take(&mut self.input_value);
        if text.is_empty() {
            return;
        }

        // Commands that work without an active window.
        if let Some(rest) = text.strip_prefix('/') {
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            let cmd = parts[0].to_ascii_lowercase();
            let args = parts.get(1).copied().unwrap_or("");

            match cmd.as_str() {
                "connect" | "server" => {
                    // /connect host [port] [nick]
                    // /server host [port] [nick]
                    let tokens: Vec<&str> = args.split_whitespace().collect();
                    if tokens.is_empty() {
                        self.view_state = ViewState::ConnectDialog;
                        return;
                    }
                    tokens[0].clone_into(&mut self.connect_form.host);
                    if let Some(p) = tokens.get(1) {
                        (*p).clone_into(&mut self.connect_form.port);
                    }
                    if let Some(n) = tokens.get(2) {
                        (*n).clone_into(&mut self.connect_form.nick);
                        (*n).clone_into(&mut self.connect_form.user);
                    }
                    self.do_connect();
                    return;
                }
                "quit" => {
                    if let Some(net_id) = self.active_network() {
                        let reason = if args.is_empty() {
                            None
                        } else {
                            Some(Bytes::from(args.to_owned().into_bytes()))
                        };
                        self.send_cmd(ClientCommand::Quit {
                            network: net_id,
                            reason,
                        });
                    }
                    return;
                }
                _ => {}
            }
        }

        let Some(win_id) = self.active_window else {
            return;
        };
        let Some(win) = self.windows.get(&win_id) else {
            return;
        };
        let net_id = win.network;

        if let Some(rest) = text.strip_prefix('/') {
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            let cmd = parts[0].to_ascii_lowercase();
            let args = parts.get(1).copied().unwrap_or("");

            match cmd.as_str() {
                "join" | "j" => {
                    let chan = if args.starts_with('#') {
                        args.to_owned()
                    } else {
                        format!("#{args}")
                    };
                    self.send_cmd(ClientCommand::Join {
                        network: net_id,
                        channel: Bytes::from(chan.into_bytes()),
                    });
                }
                "part" | "leave" => {
                    let chan = if args.is_empty() {
                        win.target.clone()
                    } else {
                        Bytes::from(args.to_owned().into_bytes())
                    };
                    self.send_cmd(ClientCommand::Part {
                        network: net_id,
                        channel: chan,
                        reason: None,
                    });
                }
                "nick" => {
                    if !args.is_empty() {
                        self.send_cmd(ClientCommand::ChangeNick {
                            network: net_id,
                            nick: Bytes::from(args.to_owned().into_bytes()),
                        });
                    }
                }
                "msg" | "privmsg" | "query" => {
                    let msg_parts: Vec<&str> = args.splitn(2, ' ').collect();
                    if msg_parts.len() == 2 {
                        let target = Bytes::from(msg_parts[0].to_owned().into_bytes());
                        let body = Bytes::from(msg_parts[1].to_owned().into_bytes());
                        self.send_cmd(ClientCommand::SendPrivmsg {
                            network: net_id,
                            target: target.clone(),
                            text: body,
                        });
                        // Open the query window.
                        self.ensure_window(net_id, target);
                    }
                }
                "topic" => {
                    self.send_cmd(ClientCommand::SetTopic {
                        network: net_id,
                        channel: win.target.clone(),
                        topic: Bytes::from(args.to_owned().into_bytes()),
                    });
                }
                "list" => {
                    self.channel_list_entries.clear();
                    self.channel_list_filter.clear();
                    self.channel_list_loading = true;
                    self.show_channel_list = true;
                    self.send_cmd(ClientCommand::List { network: net_id });
                }
                "raw" | "quote" => {
                    if !args.is_empty() {
                        self.send_cmd(ClientCommand::SendRaw {
                            network: net_id,
                            line: Bytes::from(args.to_owned().into_bytes()),
                        });
                    }
                }
                "help" | "commands" => {
                    self.show_help(net_id);
                }
                "theme" => {
                    self.theme_choice = self.theme_choice.toggle();
                    self.push_status(
                        net_id,
                        &format!("Theme switched to {}", self.theme_choice.label()),
                    );
                }
                _ => {
                    self.push_status(net_id, &format!("Unknown command: /{cmd}"));
                }
            }
            return;
        }

        // Regular text → PRIVMSG to the active channel/query.
        if win.is_status {
            self.push_status(
                net_id,
                "Cannot send text to the status window. Use /join #channel first.",
            );
        } else {
            let target = win.target.clone();
            let own_nick = self.own_nick.get(&net_id).cloned().unwrap_or_default();
            self.send_cmd(ClientCommand::SendPrivmsg {
                network: net_id,
                target,
                text: Bytes::from(text.clone().into_bytes()),
            });
            if let Some(w) = self.windows.get_mut(&win_id) {
                w.messages.push(DisplayMessage {
                    timestamp: now_stamp(),
                    from: own_nick,
                    text,
                    is_action: false,
                });
            }
        }
    }

    // -- helpers --

    fn push_status(&mut self, network: NetworkId, text: &str) {
        let status_id = self.networks.get(&network).map(|n| n.status_window);
        if let Some(id) = status_id {
            if let Some(win) = self.windows.get_mut(&id) {
                win.messages.push(DisplayMessage {
                    timestamp: now_stamp(),
                    from: String::from("*"),
                    text: text.to_owned(),
                    is_action: false,
                });
            }
        }
    }

    fn show_help(&mut self, network: NetworkId) {
        let lines = [
            "=== Available Commands ===",
            "/connect <host> [port] [nick]  — Connect to a server",
            "/server <host> [port] [nick]   — Alias for /connect",
            "/join <#channel>               — Join a channel",
            "/part [#channel] [reason]      — Leave a channel",
            "/nick <newnick>                — Change your nickname",
            "/msg <nick> <message>          — Send a private message",
            "/topic <text>                  — Set the channel topic",
            "/list                          — Browse channel list",
            "/quit [reason]                 — Disconnect from server",
            "/raw <line>                    — Send a raw IRC command",
            "/theme                         — Toggle light/dark theme",
            "/help                          — Show this help",
            "",
            "=== Guidelines ===",
            "1. Be respectful to others.",
            "2. No spam or flooding — the server enforces rate limits.",
            "3. Register your nick with /msg NickServ REGISTER",
            "   or use: REGISTER <account> <email> <password>",
            "4. Channel ops (@) can set topic, kick, and ban users.",
            "5. Use /list to discover channels.",
            "6. Private messages: /msg <nick> <text>",
        ];
        for line in lines {
            self.push_status(network, line);
        }
    }

    fn show_welcome_guidelines(&mut self, network: NetworkId, channel: &[u8]) {
        let chan = String::from_utf8_lossy(channel);
        let win_id = self.ensure_window(network, Bytes::copy_from_slice(channel));
        let lines = [
            format!("Welcome to {chan}!"),
            String::from("Type /help for a list of commands."),
            String::from("Be respectful. No spam. Have fun."),
        ];
        if let Some(win) = self.windows.get_mut(&win_id) {
            for line in lines {
                win.messages.push(DisplayMessage {
                    timestamp: now_stamp(),
                    from: String::from("*"),
                    text: line,
                    is_action: false,
                });
            }
        }
    }

    fn ensure_window(&mut self, network: NetworkId, target: Bytes) -> WindowId {
        // Check existing.
        for win in self.windows.values() {
            if win.network == network && win.target == target {
                return win.id;
            }
        }

        let id = next_window_id();
        self.windows.insert(
            id,
            Window {
                id,
                network,
                target: target.clone(),
                messages: Vec::new(),
                topic: None,
                nicks: Vec::new(),
                is_status: false,
            },
        );

        if let Some(info) = self.networks.get_mut(&network) {
            info.windows.push(WindowRef { id, target });
        }

        if self.active_window.is_none() {
            self.active_window = Some(id);
        }

        id
    }

    fn active_network(&self) -> Option<NetworkId> {
        self.active_window
            .and_then(|id| self.windows.get(&id))
            .map(|w| w.network)
    }

    fn send_cmd(&self, cmd: ClientCommand) {
        let tx = self.command_tx.clone();
        tokio::spawn(async move {
            if tx.send(cmd).await.is_err() {
                warn!("command channel closed");
            }
        });
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

impl IrcApp {
    pub(crate) fn view(&self) -> Element<'_, Msg> {
        match &self.view_state {
            ViewState::ConnectDialog => views::connect_dialog::view(&self.connect_form),
            ViewState::Irc => self.irc_view(),
        }
    }

    fn irc_view(&self) -> Element<'_, Msg> {
        let topic_text = self
            .active_window
            .and_then(|id| self.windows.get(&id))
            .and_then(|w| w.topic.as_deref())
            .unwrap_or("No topic");

        let topic_bar = container(text(topic_text).size(12))
            .style(theme::topic_bar)
            .width(Fill)
            .padding(4);

        let treebar = views::treebar::view(self.networks.iter(), self.active_window);

        let (messages, nicks): (&[DisplayMessage], &[String]) = self
            .active_window
            .and_then(|id| self.windows.get(&id))
            .map_or((&[], &[]), |w| (w.messages.as_slice(), w.nicks.as_slice()));

        let scrollback = views::scrollback::view(messages);
        let nicklist = views::nicklist::view(nicks);
        let input = views::input::view(&self.input_value);

        // Status bar
        let active_info = self.active_window.and_then(|id| {
            let win = self.windows.get(&id)?;
            let net = self.networks.get(&win.network)?;
            Some((win, net))
        });
        let status_text = if let Some((win, net)) = active_info {
            let target = String::from_utf8_lossy(&win.target);
            let conn = if net.connected {
                "connected"
            } else {
                "disconnected"
            };
            format!(
                " {} | {} | {} | [/connect to add server]",
                net.nick, target, conn
            )
        } else {
            String::from(" [/connect host port nick] or use File > Connect")
        };

        let theme_label = format!("Theme: {}", self.theme_choice.label());
        let theme_btn = iced::widget::button(text(theme_label).size(11))
            .padding([2, 8])
            .on_press(Msg::ToggleTheme);

        let status_row = row![
            container(text(status_text).size(11))
                .style(theme::topic_bar)
                .width(Fill)
                .padding(2),
            theme_btn,
        ];

        let middle = column![
            topic_bar,
            row![treebar, scrollback, nicklist].height(Fill),
            input,
            status_row,
        ];

        if self.show_channel_list {
            let list_view = views::channel_list::view(
                &self.channel_list_entries,
                &self.channel_list_filter,
                self.channel_list_loading,
            );
            return container(list_view).width(Fill).height(Fill).into();
        }

        container(middle).width(Fill).height(Fill).into()
    }
}

// ---------------------------------------------------------------------------
// Theme + Subscription
// ---------------------------------------------------------------------------

impl IrcApp {
    pub(crate) fn theme(&self) -> Theme {
        self.theme_choice.to_iced()
    }
}

impl IrcApp {
    #[allow(clippy::unused_self)]
    pub(crate) fn subscription(&self) -> Subscription<Msg> {
        irc_event_worker()
    }
}

fn irc_event_worker() -> Subscription<Msg> {
    Subscription::run(|| {
        iced::stream::channel(64, |mut output| async move {
            let Some(mut rx) = EVENT_RX_SLOT.take() else {
                std::future::pending::<()>().await;
                return;
            };
            while let Some(event) = rx.recv().await {
                if output.try_send(Msg::IrcEvent(event)).is_err() {
                    break;
                }
            }
        })
    })
}

static EVENT_RX_SLOT: ChannelSlot = ChannelSlot::new();

struct ChannelSlot {
    inner: std::sync::Mutex<Option<mpsc::Receiver<ClientEvent>>>,
}

impl ChannelSlot {
    const fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(None),
        }
    }

    fn store(&self, rx: mpsc::Receiver<ClientEvent>) {
        *self.inner.lock().expect("lock poisoned") = Some(rx);
    }

    fn take(&self) -> Option<mpsc::Receiver<ClientEvent>> {
        self.inner.lock().expect("lock poisoned").take()
    }
}

pub(crate) fn park_event_rx(rx: mpsc::Receiver<ClientEvent>) {
    EVENT_RX_SLOT.store(rx);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_stamp() -> String {
    // Simple HH:MM from system time. Avoids chrono dependency.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let hours = (secs / 3600) % 24;
    let minutes = (secs / 60) % 60;
    format!("{hours:02}:{minutes:02}")
}
