//! Registration state machine.
//!
//! The flow a fresh connection follows:
//!
//! 1. Optional `CAP LS [302]` — server replies with an empty cap list
//!    in Phase 2; Phase 5 wires real IRCv3 caps.
//! 2. Optional `PASS <password>` stashed for a later `OPER` check
//!    (Phase 4 lands the check itself).
//! 3. `NICK <nickname>` — must be valid under the server's casemap
//!    and not in use by another user.
//! 4. `USER <user> <mode> <*> :<realname>` — supplies the remaining
//!    identity fields.
//! 5. `CAP END` if negotiation was active.
//! 6. Server emits the welcome burst (001-005, 251-255 LUSERS, MOTD).
//!
//! Handlers are synchronous — they only mutate in-memory state and
//! push to per-connection mpsc queues.

use std::sync::Arc;

use bytes::Bytes;
use irc_proto::{Casemap, Message, Params, Prefix, ReplyCode, Tags, Verb};
use tracing::debug;

use crate::handler::Outcome;
use crate::numeric::{Target, numeric, numeric_text, server_name_bytes};
use crate::state::{ServerState, User, UserRegInfo};

/// Handle `CAP` subcommands (IRCv3 capability negotiation).
pub fn handle_cap(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    subcommand: &irc_proto::CapSub,
    args: &[Bytes],
) -> Outcome {
    use irc_proto::CapSub;
    match subcommand {
        CapSub::Ls => {
            user.set_cap_negotiating(true);
            let is_302 = args.first().is_some_and(|a| a.as_ref() == b"302");
            user.send(build_cap_ls(state, user, is_302));
        }
        CapSub::List => {
            let enabled = user.caps().enabled_names().join(" ");
            user.send(build_cap_msg(state, user, b"LIST", &enabled));
        }
        CapSub::Req => {
            user.set_cap_negotiating(true);
            handle_cap_req(state, user, args);
        }
        CapSub::End => {
            user.set_cap_negotiating(false);
            maybe_finalize(state, user);
        }
        CapSub::Ack | CapSub::Nak | CapSub::New | CapSub::Del | CapSub::Other(_) => {}
    }
    Outcome::Continue
}

fn handle_cap_req(state: &Arc<ServerState>, user: &Arc<User>, args: &[Bytes]) {
    // The requested caps arrive either as multiple params or as a
    // single space-separated trailing param.
    let raw: Vec<u8> = args
        .iter()
        .flat_map(|a| {
            let mut v = a.to_vec();
            v.push(b' ');
            v
        })
        .collect();
    let raw_str = String::from_utf8_lossy(&raw);
    let names: Vec<&str> = raw_str.split_whitespace().collect();

    if names.is_empty() || !crate::caps::all_known(&names) {
        user.send(build_cap_msg(state, user, b"NAK", &names.join(" ")));
        return;
    }
    // All valid — enable (or disable with `-` prefix).
    for name in &names {
        if let Some(stripped) = name.strip_prefix('-') {
            user.disable_cap(stripped);
        } else {
            user.enable_cap(name);
        }
    }
    user.send(build_cap_msg(state, user, b"ACK", &names.join(" ")));
}

fn build_cap_ls(state: &Arc<ServerState>, user: &Arc<User>, is_302: bool) -> Message {
    let cap_list = if is_302 {
        crate::caps::ADVERTISED_CAPS
            .iter()
            .map(|c| {
                if *c == "sasl" {
                    format!("sasl={}", crate::caps::SASL_CAP_VALUE)
                } else {
                    (*c).to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        crate::caps::ADVERTISED_CAPS.join(" ")
    };
    build_cap_msg(state, user, b"LS", &cap_list)
}

fn build_cap_msg(state: &Arc<ServerState>, user: &Arc<User>, sub: &[u8], value: &str) -> Message {
    let nick_or_star = user
        .snapshot()
        .nick
        .unwrap_or_else(|| Bytes::from_static(b"*"));
    let mut params = Params::new();
    params.push(nick_or_star);
    params.push(Bytes::copy_from_slice(sub));
    params.push_trailing(Bytes::from(value.to_owned().into_bytes()));
    Message {
        tags: Tags::new(),
        prefix: Some(Prefix::Server(server_name_bytes(state))),
        verb: Verb::word(Bytes::from_static(b"CAP")),
        params,
    }
}

/// Handle `NICK`.
pub fn handle_nick(state: &Arc<ServerState>, user: &Arc<User>, requested: Bytes) -> Outcome {
    if requested.is_empty() {
        user.send(numeric_text(
            state,
            Target::UNREGISTERED,
            ReplyCode::ERR_NONICKNAMEGIVEN,
            "No nickname given",
        ));
        return Outcome::Continue;
    }
    if !is_valid_nick(state.casemap(), requested.as_ref()) {
        user.send(numeric(
            state,
            Target::UNREGISTERED,
            ReplyCode::ERR_ERRONEUSNICKNAME,
            [requested],
            Some("Erroneous nickname".into()),
        ));
        return Outcome::Continue;
    }
    if state.claim_nick(user.id(), requested.as_ref()).is_err() {
        user.send(numeric(
            state,
            Target::UNREGISTERED,
            ReplyCode::ERR_NICKNAMEINUSE,
            [requested],
            Some("Nickname is already in use".into()),
        ));
        return Outcome::Continue;
    }
    let prior = user.snapshot().nick;
    user.set_nick(requested.clone());
    if user.is_registered() {
        if let Some(ref old) = prior {
            // MONITOR: old nick going offline, new nick coming online.
            crate::handler::monitor::notify_offline(state, old);
            crate::handler::monitor::notify_online(state, requested.as_ref());
            send_nick_change(user, old, requested);
        }
    } else {
        maybe_finalize(state, user);
    }
    Outcome::Continue
}

fn send_nick_change(user: &Arc<User>, old_nick: &[u8], new_nick: Bytes) {
    let prefix_bytes = user.origin_prefix_with_nick(old_nick);
    let mut params = Params::new();
    params.push(new_nick);
    let msg = Message {
        tags: Tags::new(),
        prefix: Some(Prefix::User {
            nick: prefix_bytes,
            user: None,
            host: None,
        }),
        verb: Verb::word(Bytes::from_static(b"NICK")),
        params,
    };
    user.send(msg);
}

/// Handle `USER <user> <mode> <*> :<realname>`.
pub fn handle_user(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    user_name: Bytes,
    _mode: Bytes,
    realname: Bytes,
) -> Outcome {
    if user.is_registered() {
        user.send(numeric_text(
            state,
            Target::UNREGISTERED,
            ReplyCode::ERR_ALREADYREGISTERED,
            "You may not reregister",
        ));
        return Outcome::Continue;
    }
    user.set_reg_info(UserRegInfo {
        user_name,
        realname,
    });
    maybe_finalize(state, user);
    Outcome::Continue
}

/// Handle `PASS <password>` — stash for later oper / account check.
pub fn handle_pass(_state: &Arc<ServerState>, user: &Arc<User>, password: Bytes) -> Outcome {
    if user.is_registered() {
        return Outcome::Continue;
    }
    user.set_pass(password);
    Outcome::Continue
}

/// Attempt to complete registration if all prerequisites are met.
fn maybe_finalize(state: &Arc<ServerState>, user: &Arc<User>) {
    let snap = user.snapshot();
    if snap.registered || snap.cap_negotiating || snap.nick.is_none() || snap.reg.is_none() {
        return;
    }
    let nick = snap.nick.expect("checked above");
    // Apply IP-based cloak before welcome burst so the cloaked host
    // is visible in every subsequent protocol message.
    let cloaked = state.cloak().cloak_ip(&user.peer().ip());
    user.set_cloaked_host(bytes::Bytes::from(cloaked));
    user.mark_registered();
    debug!(user = user.id().get(), ?nick, "registration complete");
    send_welcome_burst(state, user, nick.as_ref());
}

fn send_welcome_burst(state: &Arc<ServerState>, user: &Arc<User>, nick: &[u8]) {
    let cfg = state.config();
    let sv = cfg.server_name.as_str();
    let net = cfg.network_name.as_str();
    let nick_str = std::str::from_utf8(nick).unwrap_or("*");

    user.send(numeric_text(
        state,
        Target(nick),
        ReplyCode::RPL_WELCOME,
        format!("Welcome to the {net} Network, {nick_str}"),
    ));
    user.send(numeric_text(
        state,
        Target(nick),
        ReplyCode::RPL_YOURHOST,
        format!(
            "Your host is {sv}, running version {}",
            env!("CARGO_PKG_VERSION")
        ),
    ));
    user.send(numeric_text(
        state,
        Target(nick),
        ReplyCode::RPL_CREATED,
        format!("This server was compiled at {}", env!("CARGO_PKG_VERSION")),
    ));
    user.send(build_myinfo(state, nick));
    user.send(build_isupport(state, nick));

    let count = state.registered_count().max(1);
    user.send(numeric_text(
        state,
        Target(nick),
        ReplyCode::RPL_LUSERCLIENT,
        format!("There are {count} users and 0 invisible on 1 servers"),
    ));

    send_motd(state, user, nick);
}

fn build_myinfo(state: &Arc<ServerState>, nick: &[u8]) -> Message {
    let sv = Bytes::copy_from_slice(state.config().server_name.as_bytes());
    let version = Bytes::from_static(concat!("irc-server-", env!("CARGO_PKG_VERSION")).as_bytes());
    let user_modes = Bytes::from_static(b"iw");
    let chan_modes = Bytes::from_static(b"ntmikl");
    numeric(
        state,
        Target(nick),
        ReplyCode::RPL_MYINFO,
        [sv, version, user_modes, chan_modes],
        None,
    )
}

fn build_isupport(state: &Arc<ServerState>, nick: &[u8]) -> Message {
    let tokens = [
        format!("NETWORK={}", state.config().network_name),
        format!("CASEMAPPING={}", casemap_token(state.casemap())),
        "CHANTYPES=#".into(),
        "PREFIX=(ov)@+".into(),
        "CHANMODES=b,k,l,imnpst".into(),
        "NICKLEN=32".into(),
        "CHANNELLEN=50".into(),
    ];
    numeric(
        state,
        Target(nick),
        ReplyCode::RPL_ISUPPORT,
        tokens.into_iter().map(|s| Bytes::from(s.into_bytes())),
        Some("are supported by this server".into()),
    )
}

const fn casemap_token(cm: Casemap) -> &'static str {
    match cm {
        Casemap::Ascii => "ascii",
        Casemap::Rfc1459 => "rfc1459",
        Casemap::Rfc1459Strict => "rfc1459-strict",
    }
}

fn send_motd(state: &Arc<ServerState>, user: &Arc<User>, nick: &[u8]) {
    let Some(text) = state.config().motd.as_deref() else {
        user.send(numeric_text(
            state,
            Target(nick),
            ReplyCode::ERR_NOMOTD,
            "MOTD file is missing",
        ));
        return;
    };
    user.send(numeric_text(
        state,
        Target(nick),
        ReplyCode::RPL_MOTDSTART,
        format!("- {} Message of the day -", state.config().server_name),
    ));
    for line in text.lines() {
        user.send(numeric_text(
            state,
            Target(nick),
            ReplyCode::RPL_MOTD,
            format!("- {line}"),
        ));
    }
    user.send(numeric_text(
        state,
        Target(nick),
        ReplyCode::RPL_ENDOFMOTD,
        "End of /MOTD command",
    ));
}

fn is_valid_nick(_casemap: Casemap, bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.len() > 32 {
        return false;
    }
    let first_ok = matches!(
        bytes[0],
        b'a'..=b'z'
            | b'A'..=b'Z'
            | b'['
            | b']'
            | b'\\'
            | b'`'
            | b'_'
            | b'^'
            | b'{'
            | b'|'
            | b'}'
    );
    if !first_ok {
        return false;
    }
    bytes.iter().skip(1).all(|b| {
        matches!(
            *b,
            b'a'..=b'z'
                | b'A'..=b'Z'
                | b'0'..=b'9'
                | b'['
                | b']'
                | b'\\'
                | b'`'
                | b'_'
                | b'^'
                | b'{'
                | b'|'
                | b'}'
                | b'-'
        )
    })
}

#[cfg(test)]
mod tests {
    use super::is_valid_nick;
    use irc_proto::Casemap;

    #[test]
    fn validator_accepts_typical_nicks() {
        assert!(is_valid_nick(Casemap::Rfc1459, b"alice"));
        assert!(is_valid_nick(Casemap::Rfc1459, b"Alice_42"));
        assert!(is_valid_nick(Casemap::Rfc1459, b"[bot]"));
    }

    #[test]
    fn validator_rejects_leading_digit_or_punct() {
        assert!(!is_valid_nick(Casemap::Rfc1459, b"9bob"));
        assert!(!is_valid_nick(Casemap::Rfc1459, b"-bob"));
    }

    #[test]
    fn validator_rejects_control_or_space() {
        assert!(!is_valid_nick(Casemap::Rfc1459, b"ali ce"));
        assert!(!is_valid_nick(Casemap::Rfc1459, b"ali\0ce"));
    }
}
