//! MONITOR command handler (IRCv3).
//!
//! Tracks online/offline status of nicks for each connection.

use std::sync::Arc;

use bytes::Bytes;
use irc_proto::{Params, ReplyCode};

use crate::handler::Outcome;
use crate::numeric::{Target, numeric_text};
use crate::state::{ServerState, User};

/// Maximum monitor list entries per user.
const MONITOR_LIMIT: usize = 100;

/// Handle `MONITOR +/-/C/L/S`.
pub fn handle_monitor(state: &Arc<ServerState>, user: &Arc<User>, params: &Params) -> Outcome {
    if !user.is_registered() {
        user.send(numeric_text(
            state,
            Target::UNREGISTERED,
            ReplyCode::ERR_NOTREGISTERED,
            "You have not registered",
        ));
        return Outcome::Continue;
    }
    let sub = params.get(0).map(std::convert::AsRef::as_ref);
    match sub {
        Some(b"+") => {
            let targets = params.get(1).cloned().unwrap_or_default();
            monitor_add(state, user, &targets);
        }
        Some(b"-") => {
            let targets = params.get(1).cloned().unwrap_or_default();
            monitor_remove(user, &targets);
        }
        Some(b"C" | b"c") => {
            user.inner_write().monitor_list.clear();
        }
        Some(b"L" | b"l") => {
            monitor_list(state, user);
        }
        Some(b"S" | b"s") => {
            monitor_status(state, user);
        }
        _ => {
            // Sub with + or - prefix attached (e.g. "+nick1,nick2")
            if let Some(first) = params.get(0) {
                if first.starts_with(b"+") {
                    let targets = Bytes::copy_from_slice(&first[1..]);
                    monitor_add(state, user, &targets);
                } else if first.starts_with(b"-") {
                    let targets = Bytes::copy_from_slice(&first[1..]);
                    monitor_remove(user, &targets);
                }
            }
        }
    }
    Outcome::Continue
}

fn monitor_add(state: &Arc<ServerState>, user: &Arc<User>, targets: &Bytes) {
    let nick = user
        .snapshot()
        .nick
        .unwrap_or_else(|| Bytes::from_static(b"*"));
    let nicks: Vec<&[u8]> = targets
        .split(|b| *b == b',')
        .filter(|s| !s.is_empty())
        .collect();

    let mut inner = user.inner_write();
    let current_len = inner.monitor_list.len();
    if current_len + nicks.len() > MONITOR_LIMIT {
        drop(inner);
        user.send(numeric_text(
            state,
            Target(&nick),
            ReplyCode::ERR_MONLISTFULL,
            format!("{MONITOR_LIMIT} Monitor list is full"),
        ));
        return;
    }

    let mut online = Vec::new();
    let mut offline = Vec::new();
    for n in nicks {
        let nb = Bytes::copy_from_slice(n);
        if !inner
            .monitor_list
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&nb))
        {
            inner.monitor_list.push(nb.clone());
        }
        // Check online status (release lock briefly not needed since we
        // only read state).
        if state.user_by_nick(n).is_some() {
            online.push(nb);
        } else {
            offline.push(nb);
        }
    }
    drop(inner);

    if !online.is_empty() {
        let list = online
            .iter()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .collect::<Vec<_>>()
            .join(",");
        user.send(numeric_text(
            state,
            Target(&nick),
            ReplyCode::RPL_MONONLINE,
            list,
        ));
    }
    if !offline.is_empty() {
        let list = offline
            .iter()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .collect::<Vec<_>>()
            .join(",");
        user.send(numeric_text(
            state,
            Target(&nick),
            ReplyCode::RPL_MONOFFLINE,
            list,
        ));
    }
}

fn monitor_remove(user: &Arc<User>, targets: &Bytes) {
    let nicks: Vec<&[u8]> = targets
        .split(|b| *b == b',')
        .filter(|s| !s.is_empty())
        .collect();
    let mut inner = user.inner_write();
    for n in nicks {
        inner
            .monitor_list
            .retain(|existing| !existing.eq_ignore_ascii_case(n));
    }
}

fn monitor_list(state: &Arc<ServerState>, user: &Arc<User>) {
    let nick = user
        .snapshot()
        .nick
        .unwrap_or_else(|| Bytes::from_static(b"*"));
    let inner = user.snapshot();
    let list = inner
        .monitor_list
        .iter()
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .collect::<Vec<_>>()
        .join(",");
    if !list.is_empty() {
        user.send(numeric_text(
            state,
            Target(&nick),
            ReplyCode::RPL_MONLIST,
            list,
        ));
    }
    user.send(numeric_text(
        state,
        Target(&nick),
        ReplyCode::RPL_ENDOFMONLIST,
        "End of MONITOR list",
    ));
}

fn monitor_status(state: &Arc<ServerState>, user: &Arc<User>) {
    let nick = user
        .snapshot()
        .nick
        .unwrap_or_else(|| Bytes::from_static(b"*"));
    let inner = user.snapshot();
    let mut online = Vec::new();
    let mut offline = Vec::new();
    for monitored in &inner.monitor_list {
        if state.user_by_nick(monitored.as_ref()).is_some() {
            online.push(String::from_utf8_lossy(monitored).into_owned());
        } else {
            offline.push(String::from_utf8_lossy(monitored).into_owned());
        }
    }
    if !online.is_empty() {
        user.send(numeric_text(
            state,
            Target(&nick),
            ReplyCode::RPL_MONONLINE,
            online.join(","),
        ));
    }
    if !offline.is_empty() {
        user.send(numeric_text(
            state,
            Target(&nick),
            ReplyCode::RPL_MONOFFLINE,
            offline.join(","),
        ));
    }
}

/// Notify any users monitoring `nick` that it came online.
pub fn notify_online(state: &ServerState, nick: &[u8]) {
    let nick_str = String::from_utf8_lossy(nick).into_owned();
    for uh in state.users() {
        let u = uh.user();
        let watching = u
            .snapshot()
            .monitor_list
            .iter()
            .any(|m| m.eq_ignore_ascii_case(nick));
        if watching {
            let target_nick = u
                .snapshot()
                .nick
                .unwrap_or_else(|| Bytes::from_static(b"*"));
            u.send(numeric_text(
                state,
                Target(&target_nick),
                ReplyCode::RPL_MONONLINE,
                nick_str.clone(),
            ));
        }
    }
}

/// Notify any users monitoring `nick` that it went offline.
pub fn notify_offline(state: &ServerState, nick: &[u8]) {
    let nick_str = String::from_utf8_lossy(nick).into_owned();
    for uh in state.users() {
        let u = uh.user();
        let watching = u
            .snapshot()
            .monitor_list
            .iter()
            .any(|m| m.eq_ignore_ascii_case(nick));
        if watching {
            let target_nick = u
                .snapshot()
                .nick
                .unwrap_or_else(|| Bytes::from_static(b"*"));
            u.send(numeric_text(
                state,
                Target(&target_nick),
                ReplyCode::RPL_MONOFFLINE,
                nick_str.clone(),
            ));
        }
    }
}
