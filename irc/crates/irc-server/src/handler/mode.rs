//! `MODE` command handler — channel and user mode queries and changes.

use std::sync::Arc;

use bytes::Bytes;
use irc_proto::{
    Message, ModeSpec, Params, Prefix, ReplyCode, Tags, Verb, parse_channel_modes, parse_user_modes,
};
use tracing::debug;

use crate::handler::Outcome;
use crate::handler::channel::broadcast;
use crate::numeric::{Target, numeric, numeric_one};
use crate::state::{ServerState, User};

/// Dispatch a `MODE` command to the appropriate channel or user handler.
pub fn handle_mode(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    target: &Bytes,
    changes: Option<Bytes>,
    args: &[Bytes],
) -> Outcome {
    if !ensure_registered(state, user) {
        return Outcome::Continue;
    }
    if is_channel_target(target) {
        handle_channel_mode(state, user, target, changes, args)
    } else {
        handle_user_mode(state, user, target, changes)
    }
}

fn is_channel_target(target: &[u8]) -> bool {
    matches!(target.first(), Some(b'#' | b'&' | b'+' | b'!'))
}

// ---------------------------------------------------------------------------
// Channel modes
// ---------------------------------------------------------------------------

fn handle_channel_mode(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    target: &Bytes,
    changes: Option<Bytes>,
    args: &[Bytes],
) -> Outcome {
    let Some(chan_lock) = state.channel(target.as_ref()) else {
        let nick = nick_or_star(user);
        user.send(numeric_one(
            state,
            Target(&nick),
            ReplyCode::ERR_NOSUCHCHANNEL,
            target.clone(),
            "No such channel",
        ));
        return Outcome::Continue;
    };

    let nick = nick_or_star(user);

    match changes {
        None => {
            let guard = chan_lock.read();
            let mode_string = channel_mode_string(&guard);
            let name = guard.name.clone();
            drop(guard);
            user.send(numeric(
                state,
                Target(&nick),
                ReplyCode::RPL_CHANNELMODEIS,
                [name, Bytes::from(mode_string)],
                None,
            ));
        }
        Some(change_str) => {
            return apply_channel_modes(state, user, &nick, &chan_lock, &change_str, args);
        }
    }

    Outcome::Continue
}

/// Apply parsed mode changes to a channel. Split out from
/// `handle_channel_mode` so the function stays under the line limit.
fn apply_channel_modes(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    nick: &Bytes,
    chan_lock: &Arc<parking_lot::RwLock<crate::state::Channel>>,
    change_str: &Bytes,
    args: &[Bytes],
) -> Outcome {
    let mut guard = chan_lock.write();

    if !guard.members.get(&user.id()).is_some_and(|m| m.op) {
        let name = guard.name.clone();
        drop(guard);
        user.send(numeric_one(
            state,
            Target(nick),
            ReplyCode::ERR_CHANOPRIVSNEEDED,
            name,
            "You're not channel operator",
        ));
        return Outcome::Continue;
    }

    let spec = ModeSpec::rfc2812();
    let parsed = parse_channel_modes(change_str, args, &spec);

    let mut applied = bytes::BytesMut::new();
    let mut applied_args: Vec<Bytes> = Vec::new();
    let mut last_dir: Option<bool> = None;

    for ch in &parsed {
        match ch.letter {
            b'n' | b't' | b'm' | b'i' => {
                apply_flag_mode(&mut guard, ch.letter, ch.adding);
                push_letter(&mut applied, &mut last_dir, ch.adding, ch.letter);
            }
            b'k' => {
                if ch.adding {
                    if let Some(key) = &ch.arg {
                        guard.mode_k = Some(key.clone());
                        push_letter(&mut applied, &mut last_dir, true, b'k');
                        applied_args.push(key.clone());
                    }
                } else {
                    guard.mode_k = None;
                    push_letter(&mut applied, &mut last_dir, false, b'k');
                }
            }
            b'l' => {
                if ch.adding {
                    if let Some(n) = ch
                        .arg
                        .as_ref()
                        .and_then(|a| std::str::from_utf8(a).ok())
                        .and_then(|s| s.parse::<u32>().ok())
                    {
                        guard.mode_l = Some(n);
                        push_letter(&mut applied, &mut last_dir, true, b'l');
                        if let Some(a) = &ch.arg {
                            applied_args.push(a.clone());
                        }
                    }
                } else {
                    guard.mode_l = None;
                    push_letter(&mut applied, &mut last_dir, false, b'l');
                }
            }
            b'o' | b'v' => {
                let Some(nick_arg) = &ch.arg else { continue };
                let Some(target_user) = state.user_by_nick(nick_arg) else {
                    continue;
                };
                let Some(member) = guard.members.get_mut(&target_user.id()) else {
                    continue;
                };
                if ch.letter == b'o' {
                    member.op = ch.adding;
                } else {
                    member.voice = ch.adding;
                }
                push_letter(&mut applied, &mut last_dir, ch.adding, ch.letter);
                applied_args.push(nick_arg.clone());
            }
            _ => {
                debug!(letter = %char::from(ch.letter), "ignoring unknown channel mode");
            }
        }
    }

    if applied.is_empty() {
        return Outcome::Continue;
    }

    let member_ids = guard.member_ids();
    let chan_name = guard.name.clone();
    drop(guard);

    let origin = user.origin_prefix();
    let mode_msg = mode_change_line(&origin, &chan_name, &applied.freeze(), &applied_args);
    broadcast(state, &member_ids, &mode_msg);

    Outcome::Continue
}

fn apply_flag_mode(chan: &mut crate::state::Channel, letter: u8, adding: bool) {
    match letter {
        b'n' => chan.mode_n = adding,
        b't' => chan.mode_t = adding,
        b'm' => chan.mode_m = adding,
        b'i' => chan.mode_i = adding,
        _ => {}
    }
}

fn channel_mode_string(chan: &crate::state::Channel) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.push(b'+');
    if chan.mode_n {
        out.push(b'n');
    }
    if chan.mode_t {
        out.push(b't');
    }
    if chan.mode_m {
        out.push(b'm');
    }
    if chan.mode_i {
        out.push(b'i');
    }
    if chan.mode_k.is_some() {
        out.push(b'k');
    }
    if chan.mode_l.is_some() {
        out.push(b'l');
    }
    out
}

// ---------------------------------------------------------------------------
// User modes
// ---------------------------------------------------------------------------

fn handle_user_mode(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    target: &Bytes,
    changes: Option<Bytes>,
) -> Outcome {
    let nick = nick_or_star(user);

    if !state.casemap().eq_bytes(target, &nick) {
        user.send(numeric_one(
            state,
            Target(&nick),
            ReplyCode::ERR_USERSDONTMATCH,
            target.clone(),
            "Can't change mode for other users",
        ));
        return Outcome::Continue;
    }

    let reply = match changes {
        None => {
            let snap = user.snapshot();
            user_mode_string(&snap)
        }
        Some(change_str) => {
            let parsed = parse_user_modes(&change_str);
            {
                let mut inner = user.inner_write();
                for ch in &parsed {
                    match ch.letter {
                        b'i' => inner.mode_i = ch.adding,
                        b'w' => inner.mode_w = ch.adding,
                        _ => {}
                    }
                }
            }
            let snap = user.snapshot();
            user_mode_string(&snap)
        }
    };

    let mut params = Params::new();
    params.push(nick);
    params.push_trailing(Bytes::from(reply));
    user.send(Message {
        tags: Tags::new(),
        prefix: Some(Prefix::Server(crate::numeric::server_name_bytes(state))),
        verb: Verb::word(Bytes::from_static(b"MODE")),
        params,
    });

    Outcome::Continue
}

fn user_mode_string(inner: &crate::state::user::UserInner) -> Vec<u8> {
    let mut out = Vec::with_capacity(4);
    out.push(b'+');
    if inner.mode_i {
        out.push(b'i');
    }
    if inner.mode_w {
        out.push(b'w');
    }
    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ensure_registered(state: &Arc<ServerState>, user: &Arc<User>) -> bool {
    if user.is_registered() {
        return true;
    }
    user.send(crate::numeric::numeric_text(
        state,
        Target::UNREGISTERED,
        ReplyCode::ERR_NOTREGISTERED,
        "You have not registered",
    ));
    false
}

fn nick_or_star(user: &User) -> Bytes {
    user.snapshot()
        .nick
        .unwrap_or_else(|| Bytes::from_static(b"*"))
}

fn push_letter(buf: &mut bytes::BytesMut, last_dir: &mut Option<bool>, adding: bool, letter: u8) {
    if *last_dir != Some(adding) {
        buf.extend_from_slice(if adding { b"+" } else { b"-" });
        *last_dir = Some(adding);
    }
    buf.extend_from_slice(&[letter]);
}

fn mode_change_line(origin: &Bytes, chan: &Bytes, changes: &Bytes, args: &[Bytes]) -> Message {
    let mut params = Params::new();
    params.push(chan.clone());
    params.push(changes.clone());
    for arg in args {
        params.push(arg.clone());
    }
    Message {
        tags: Tags::new(),
        prefix: Some(Prefix::User {
            nick: origin.clone(),
            user: None,
            host: None,
        }),
        verb: Verb::word(Bytes::from_static(b"MODE")),
        params,
    }
}
