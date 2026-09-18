use bytes::Bytes;
use irc_proto::params::Params;
use irc_proto::verb::Verb;
use irc_proto::{Message, Prefix, Tags};

use crate::upstream::UpstreamState;

/// The pseudo-user nick that intercepts admin commands.
pub const STATUS_NICK: &str = "*status";

/// Handle a message addressed to `*status` and return response NOTICEs.
///
/// Returns `None` if the message is not a `PRIVMSG` to `*status`.
pub fn handle_admin_command(msg: &Message, state: &UpstreamState) -> Option<Vec<Message>> {
    let irc_proto::Command::Privmsg { targets, text } = irc_proto::Command::parse(msg).ok()? else {
        return None;
    };

    let is_status = targets
        .iter()
        .any(|t| t.eq_ignore_ascii_case(STATUS_NICK.as_bytes()));
    if !is_status {
        return None;
    }

    let cmd = String::from_utf8_lossy(&text);
    let cmd = cmd.trim();
    let replies = match cmd.to_ascii_lowercase().as_str() {
        "help" => vec!["Available commands: listnetworks, status, help".to_owned()],
        "listnetworks" => {
            let networks: Vec<String> = state
                .joined_channels
                .iter()
                .map(|k| String::from_utf8_lossy(k).into_owned())
                .collect();
            if networks.is_empty() {
                vec!["No channels currently joined.".to_owned()]
            } else {
                vec![format!("Joined channels: {}", networks.join(", "))]
            }
        }
        "status" => {
            let status = if state.registered {
                "connected and registered"
            } else {
                "not registered"
            };
            vec![format!("Upstream status: {status}")]
        }
        _ => vec![format!("Unknown command: {cmd}. Try 'help'.")],
    };

    let notices = replies
        .into_iter()
        .map(|text| Message {
            tags: Tags::new(),
            prefix: Some(Prefix::user(
                Bytes::from_static(STATUS_NICK.as_bytes()),
                None,
                None,
            )),
            verb: Verb::Word(Bytes::from_static(b"NOTICE")),
            params: {
                let mut p = Params::from_iter_middle([Bytes::from_static(b"*")]);
                p.push_trailing(Bytes::from(text));
                p
            },
        })
        .collect();

    Some(notices)
}
