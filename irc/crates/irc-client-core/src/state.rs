//! Client-side channel and nick state tracking.

use std::collections::HashMap;

use bytes::Bytes;
use irc_proto::Message;
use irc_proto::command::Command;
use irc_proto::prefix::Prefix;

/// State of a single channel as seen by the client.
#[derive(Debug, Clone)]
pub struct ChannelState {
    /// Channel name.
    pub name: Bytes,
    /// Current topic, if known.
    pub topic: Option<Bytes>,
    /// Known nicks in the channel.
    pub nicks: Vec<Bytes>,
}

/// Per-network state tracked by the client.
#[derive(Debug, Clone, Default)]
pub struct NetworkState {
    /// Our current nick, set after registration.
    pub nick: Option<Bytes>,
    /// Channels we have joined, keyed by lowercase name.
    pub channels: HashMap<Bytes, ChannelState>,
    /// Server name from the prefix of `RPL_WELCOME`.
    pub server_name: Option<Bytes>,
}

impl NetworkState {
    /// Create a new empty state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Update state from a parsed IRC message. Returns `true` if the message
    /// was meaningful to state tracking (i.e. we processed it).
    pub fn apply(&mut self, msg: &Message) -> bool {
        let Ok(cmd) = Command::parse(msg) else {
            return false;
        };
        match cmd {
            Command::Join { channels, .. } => self.apply_join(msg, channels),
            Command::Part { channels, .. } => self.apply_part(msg, &channels),
            Command::Nick { nick: new_nick } => self.apply_nick(msg, &new_nick),
            Command::Topic { channel, topic } => self.apply_topic(&channel, topic),
            Command::Quit { .. } => self.apply_quit(msg),
            Command::Numeric { code: 1, params } => self.apply_welcome(msg, &params),
            Command::Numeric { code: 332, params } => self.apply_rpl_topic(&params),
            Command::Numeric { code: 353, params } => self.apply_rpl_namreply(&params),
            _ => false,
        }
    }

    fn apply_join(&mut self, msg: &Message, channels: Vec<Bytes>) -> bool {
        let nick = extract_nick(msg.prefix.as_ref());
        let is_self = self.is_me(&nick);
        for channel in channels {
            if is_self {
                self.channels
                    .entry(channel.clone())
                    .or_insert_with(|| ChannelState {
                        name: channel.clone(),
                        topic: None,
                        nicks: Vec::new(),
                    });
            }
            if let Some(state) = self.channels.get_mut(&channel) {
                if !state.nicks.iter().any(|n| n == &nick) {
                    state.nicks.push(nick.clone());
                }
            }
        }
        true
    }

    fn apply_part(&mut self, msg: &Message, channels: &[Bytes]) -> bool {
        let nick = extract_nick(msg.prefix.as_ref());
        let is_self = self.is_me(&nick);
        for channel in channels {
            if is_self {
                self.channels.remove(channel);
            } else if let Some(state) = self.channels.get_mut(channel) {
                state.nicks.retain(|n| n != &nick);
            }
        }
        true
    }

    fn apply_nick(&mut self, msg: &Message, new_nick: &Bytes) -> bool {
        let old_nick = extract_nick(msg.prefix.as_ref());
        if self.is_me(&old_nick) {
            self.nick = Some(new_nick.clone());
        }
        for state in self.channels.values_mut() {
            for n in &mut state.nicks {
                if *n == old_nick {
                    *n = new_nick.clone();
                }
            }
        }
        true
    }

    fn apply_topic(&mut self, channel: &Bytes, topic: Option<Bytes>) -> bool {
        if let Some(state) = self.channels.get_mut(channel) {
            state.topic = topic;
        }
        true
    }

    fn apply_quit(&mut self, msg: &Message) -> bool {
        let nick = extract_nick(msg.prefix.as_ref());
        if self.is_me(&nick) {
            self.channels.clear();
        } else {
            for state in self.channels.values_mut() {
                state.nicks.retain(|n| n != &nick);
            }
        }
        true
    }

    fn apply_welcome(&mut self, msg: &Message, params: &irc_proto::Params) -> bool {
        if let Some(nick) = params.get(0) {
            self.nick = Some(nick.clone());
        }
        if let Some(Prefix::Server(name)) = &msg.prefix {
            self.server_name = Some(name.clone());
        }
        true
    }

    fn apply_rpl_topic(&mut self, params: &irc_proto::Params) -> bool {
        // RPL_TOPIC: <client> <channel> :<topic>
        if let (Some(channel), Some(topic)) = (params.get(1), params.get(2)) {
            if let Some(state) = self.channels.get_mut(channel) {
                state.topic = Some(topic.clone());
            }
        }
        true
    }

    fn apply_rpl_namreply(&mut self, params: &irc_proto::Params) -> bool {
        // RPL_NAMREPLY: <client> <symbol> <channel> :<nicks>
        if let (Some(channel), Some(nicks_raw)) = (params.get(2), params.get(3)) {
            if let Some(state) = self.channels.get_mut(channel) {
                for raw_nick in nicks_raw.split(|&b| b == b' ') {
                    if raw_nick.is_empty() {
                        continue;
                    }
                    let nick = strip_nick_prefix(raw_nick);
                    let nick_bytes = Bytes::copy_from_slice(nick);
                    if !state.nicks.iter().any(|n| n.as_ref() == nick) {
                        state.nicks.push(nick_bytes);
                    }
                }
            }
        }
        true
    }

    fn is_me(&self, nick: &[u8]) -> bool {
        self.nick.as_ref().is_some_and(|n| n.as_ref() == nick)
    }
}

/// Extract the nick from a message prefix, or return empty bytes.
fn extract_nick(prefix: Option<&Prefix>) -> Bytes {
    match prefix {
        Some(Prefix::User { nick, .. }) => nick.clone(),
        Some(Prefix::Server(name)) => name.clone(),
        None => Bytes::new(),
    }
}

/// Strip leading mode prefix characters from a nick in NAMES reply.
fn strip_nick_prefix(nick: &[u8]) -> &[u8] {
    // Common prefixes: @ (op), + (voice), % (halfop), ~ (owner), & (admin)
    let start = nick
        .iter()
        .position(|&b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'[' | b']' | b'\\' | b'^' | b'{' | b'}' | b'|' | b'_' | b'-' | b'`'
                )
        })
        .unwrap_or(0);
    &nick[start..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use irc_proto::Message;

    fn msg(s: &str) -> Message {
        Message::parse_slice(s.as_bytes()).expect("valid IRC message")
    }

    #[test]
    fn join_adds_channel_for_self() {
        let mut state = NetworkState::new();
        state.nick = Some(Bytes::from_static(b"alice"));

        state.apply(&msg(":alice!~a@host JOIN #rust"));

        assert!(state.channels.contains_key(&Bytes::from_static(b"#rust")));
        let ch = &state.channels[&Bytes::from_static(b"#rust")];
        assert!(ch.nicks.iter().any(|n| n.as_ref() == b"alice"));
    }

    #[test]
    fn join_adds_nick_to_existing_channel() {
        let mut state = NetworkState::new();
        state.nick = Some(Bytes::from_static(b"alice"));
        state.apply(&msg(":alice!~a@host JOIN #rust"));

        state.apply(&msg(":bob!~b@host JOIN #rust"));

        let ch = &state.channels[&Bytes::from_static(b"#rust")];
        assert!(ch.nicks.iter().any(|n| n.as_ref() == b"bob"));
    }

    #[test]
    fn part_removes_self_channel() {
        let mut state = NetworkState::new();
        state.nick = Some(Bytes::from_static(b"alice"));
        state.apply(&msg(":alice!~a@host JOIN #rust"));

        state.apply(&msg(":alice!~a@host PART #rust"));

        assert!(!state.channels.contains_key(&Bytes::from_static(b"#rust")));
    }

    #[test]
    fn part_removes_other_nick() {
        let mut state = NetworkState::new();
        state.nick = Some(Bytes::from_static(b"alice"));
        state.apply(&msg(":alice!~a@host JOIN #rust"));
        state.apply(&msg(":bob!~b@host JOIN #rust"));

        state.apply(&msg(":bob!~b@host PART #rust"));

        let ch = &state.channels[&Bytes::from_static(b"#rust")];
        assert!(!ch.nicks.iter().any(|n| n.as_ref() == b"bob"));
        assert!(ch.nicks.iter().any(|n| n.as_ref() == b"alice"));
    }

    #[test]
    fn nick_change_updates_state() {
        let mut state = NetworkState::new();
        state.nick = Some(Bytes::from_static(b"alice"));
        state.apply(&msg(":alice!~a@host JOIN #rust"));

        state.apply(&msg(":alice!~a@host NICK alice2"));

        assert_eq!(state.nick.as_deref(), Some(b"alice2".as_ref()));
        let ch = &state.channels[&Bytes::from_static(b"#rust")];
        assert!(ch.nicks.iter().any(|n| n.as_ref() == b"alice2"));
        assert!(!ch.nicks.iter().any(|n| n.as_ref() == b"alice"));
    }

    #[test]
    fn topic_change_updates_channel() {
        let mut state = NetworkState::new();
        state.nick = Some(Bytes::from_static(b"alice"));
        state.apply(&msg(":alice!~a@host JOIN #rust"));

        state.apply(&msg(":bob!~b@host TOPIC #rust :Welcome!"));

        let ch = &state.channels[&Bytes::from_static(b"#rust")];
        assert_eq!(ch.topic.as_deref(), Some(b"Welcome!".as_ref()));
    }

    #[test]
    fn quit_removes_user_from_channels() {
        let mut state = NetworkState::new();
        state.nick = Some(Bytes::from_static(b"alice"));
        state.apply(&msg(":alice!~a@host JOIN #rust"));
        state.apply(&msg(":bob!~b@host JOIN #rust"));

        state.apply(&msg(":bob!~b@host QUIT :bye"));

        let ch = &state.channels[&Bytes::from_static(b"#rust")];
        assert!(!ch.nicks.iter().any(|n| n.as_ref() == b"bob"));
    }

    #[test]
    fn rpl_welcome_sets_nick_and_server() {
        let mut state = NetworkState::new();

        state.apply(&msg(":irc.example.net 001 alice :Welcome to ExampleNet"));

        assert_eq!(state.nick.as_deref(), Some(b"alice".as_ref()));
        assert_eq!(
            state.server_name.as_deref(),
            Some(b"irc.example.net".as_ref())
        );
    }
}
