//! Channel operations: JOIN, PART, TOPIC, NAMES.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use irc_proto::{Message, Params, Prefix, ReplyCode, Tags, Verb};

use crate::handler::Outcome;
use crate::numeric::{Target, numeric, numeric_one, numeric_text};
use crate::state::{MemberMode, ServerState, Topic, User};

/// Handle `JOIN <channels> [keys]`.
pub fn handle_join(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    channels: Vec<Bytes>,
    _keys: &[Bytes],
) -> Outcome {
    if !ensure_registered(state, user) {
        return Outcome::Continue;
    }
    for raw in channels {
        if !is_valid_channel(raw.as_ref()) {
            user.send(numeric(
                state,
                Target::UNREGISTERED,
                ReplyCode::ERR_NOSUCHCHANNEL,
                [raw],
                Some("No such channel".into()),
            ));
            continue;
        }
        join_one(state, user, &raw);
    }
    Outcome::Continue
}

fn join_one(state: &Arc<ServerState>, user: &Arc<User>, chan_name: &Bytes) {
    let chan = state.channel_or_create(chan_name.as_ref());
    let mut guard = chan.write();
    if guard.has_member(user.id()) {
        return;
    }
    // First joiner becomes op by convention.
    let mode = if guard.has_op() || !guard.members.is_empty() {
        MemberMode::default()
    } else {
        MemberMode {
            op: true,
            voice: false,
        }
    };
    guard.add_member(user.id(), mode);
    let member_ids = guard.member_ids();
    let topic = guard.topic.clone();
    let canonical_name = guard.name.clone();
    drop(guard);

    // Broadcast JOIN to every member (including the joiner; mIRC-style
    // clients expect to see their own JOIN echoed).
    let origin = user.origin_prefix();
    let join_msg = join_line(&origin, &canonical_name);
    broadcast(state, &member_ids, &join_msg);

    // Send TOPIC / NOTOPIC + NAMES to the joining user.
    let nick = user
        .snapshot()
        .nick
        .unwrap_or_else(|| Bytes::from_static(b"*"));
    if let Some(t) = topic {
        send_topic(state, user, &nick, &canonical_name, &t);
    }
    send_names(state, user, &nick, &canonical_name);
}

fn join_line(origin: &Bytes, chan: &Bytes) -> Message {
    let mut params = Params::new();
    params.push(chan.clone());
    Message {
        tags: Tags::new(),
        prefix: Some(Prefix::User {
            nick: origin.clone(),
            user: None,
            host: None,
        }),
        verb: Verb::word(Bytes::from_static(b"JOIN")),
        params,
    }
}

/// Handle `PART <channels> [:reason]`.
pub fn handle_part(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    channels: Vec<Bytes>,
    reason: Option<&Bytes>,
) -> Outcome {
    if !ensure_registered(state, user) {
        return Outcome::Continue;
    }
    let origin = user.origin_prefix();
    for raw in channels {
        let Some(chan) = state.channel(raw.as_ref()) else {
            user.send(numeric(
                state,
                Target::UNREGISTERED,
                ReplyCode::ERR_NOSUCHCHANNEL,
                [raw],
                Some("No such channel".into()),
            ));
            continue;
        };
        let mut guard = chan.write();
        if !guard.has_member(user.id()) {
            let canonical = guard.name.clone();
            drop(guard);
            user.send(numeric(
                state,
                Target::UNREGISTERED,
                ReplyCode::ERR_NOTONCHANNEL,
                [canonical],
                Some("You're not on that channel".into()),
            ));
            continue;
        }
        guard.remove_member(user.id());
        let member_ids = guard.member_ids();
        let canonical = guard.name.clone();
        drop(guard);

        let part_msg = part_line(&origin, &canonical, reason);
        // The departing user also gets the PART echoed.
        let mut recipients = member_ids;
        recipients.push(user.id());
        broadcast(state, &recipients, &part_msg);

        state.remove_empty_channel(canonical.as_ref());
    }
    Outcome::Continue
}

fn part_line(origin: &Bytes, chan: &Bytes, reason: Option<&Bytes>) -> Message {
    let mut params = Params::new();
    params.push(chan.clone());
    if let Some(r) = reason {
        params.push_trailing(r.clone());
    }
    Message {
        tags: Tags::new(),
        prefix: Some(Prefix::User {
            nick: origin.clone(),
            user: None,
            host: None,
        }),
        verb: Verb::word(Bytes::from_static(b"PART")),
        params,
    }
}

/// Handle `TOPIC <channel> [:topic]` (read without `:topic`, write
/// with it).
pub fn handle_topic(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    channel: Bytes,
    new_topic: Option<Bytes>,
) -> Outcome {
    if !ensure_registered(state, user) {
        return Outcome::Continue;
    }
    let Some(chan) = state.channel(channel.as_ref()) else {
        user.send(numeric(
            state,
            Target::UNREGISTERED,
            ReplyCode::ERR_NOSUCHCHANNEL,
            [channel],
            Some("No such channel".into()),
        ));
        return Outcome::Continue;
    };
    let nick = user
        .snapshot()
        .nick
        .unwrap_or_else(|| Bytes::from_static(b"*"));
    match new_topic {
        None => {
            let guard = chan.read();
            let topic = guard.topic.clone();
            let name = guard.name.clone();
            drop(guard);
            if let Some(t) = topic {
                send_topic(state, user, &nick, &name, &t);
            } else {
                user.send(numeric_one(
                    state,
                    Target(&nick),
                    ReplyCode::RPL_NOTOPIC,
                    name,
                    "No topic is set",
                ));
            }
        }
        Some(text) => {
            let mut guard = chan.write();
            if !guard.has_member(user.id()) {
                let canonical = guard.name.clone();
                drop(guard);
                user.send(numeric(
                    state,
                    Target::UNREGISTERED,
                    ReplyCode::ERR_NOTONCHANNEL,
                    [canonical],
                    Some("You're not on that channel".into()),
                ));
                return Outcome::Continue;
            }
            let topic_text = text.clone();
            guard.topic = Some(Topic {
                text,
                setter: nick,
                set_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs()),
            });
            let member_ids = guard.member_ids();
            let name = guard.name.clone();
            drop(guard);
            let origin = user.origin_prefix();
            let topic_msg = topic_change_line(&origin, &name, &topic_text);
            broadcast(state, &member_ids, &topic_msg);
        }
    }
    Outcome::Continue
}

fn topic_change_line(origin: &Bytes, chan: &Bytes, text: &Bytes) -> Message {
    let mut params = Params::new();
    params.push(chan.clone());
    params.push_trailing(text.clone());
    Message {
        tags: Tags::new(),
        prefix: Some(Prefix::User {
            nick: origin.clone(),
            user: None,
            host: None,
        }),
        verb: Verb::word(Bytes::from_static(b"TOPIC")),
        params,
    }
}

/// Handle `NAMES <channels>`.
pub fn handle_names(state: &Arc<ServerState>, user: &Arc<User>, channels: Vec<Bytes>) -> Outcome {
    if !ensure_registered(state, user) {
        return Outcome::Continue;
    }
    let nick = user
        .snapshot()
        .nick
        .unwrap_or_else(|| Bytes::from_static(b"*"));
    for raw in channels {
        send_names(state, user, &nick, &raw);
    }
    Outcome::Continue
}

fn send_names(state: &Arc<ServerState>, user: &Arc<User>, nick: &Bytes, channel: &Bytes) {
    let Some(chan) = state.channel(channel.as_ref()) else {
        // RPL_ENDOFNAMES always sent, even for non-existent channels,
        // to let the client close its NAMES window.
        user.send(numeric_one(
            state,
            Target(nick),
            ReplyCode::RPL_ENDOFNAMES,
            channel.clone(),
            "End of /NAMES list",
        ));
        return;
    };
    let guard = chan.read();
    let canonical = guard.name.clone();
    let mut entries: Vec<(Bytes, MemberMode)> = Vec::with_capacity(guard.members.len());
    for (uid, mode) in &guard.members {
        if let Some(u) = state.user(*uid) {
            let snap_nick = u
                .snapshot()
                .nick
                .unwrap_or_else(|| Bytes::from_static(b"*"));
            entries.push((snap_nick, *mode));
        }
    }
    drop(guard);
    entries.sort_by(|a, b| a.0.as_ref().cmp(b.0.as_ref()));

    // Pack into a single RPL_NAMREPLY for the MVP; chunking for very
    // large channels lands with ISUPPORT NAMESLEN in a later phase.
    let mut line = Vec::with_capacity(entries.iter().map(|(n, _)| n.len() + 2).sum());
    for (i, (n, mode)) in entries.iter().enumerate() {
        if i > 0 {
            line.push(b' ');
        }
        if let Some(pb) = mode.prefix_byte() {
            line.push(pb);
        }
        line.extend_from_slice(n.as_ref());
    }
    // RPL_NAMREPLY has the extra `=` symbol (public channel). A `*`
    // would mean private, `@` secret; Phase 4 wires this when modes
    // land.
    let symbol = Bytes::from_static(b"=");
    user.send(numeric(
        state,
        Target(nick),
        ReplyCode::RPL_NAMREPLY,
        [symbol, canonical.clone()],
        Some(Bytes::from(line)),
    ));
    user.send(numeric_one(
        state,
        Target(nick),
        ReplyCode::RPL_ENDOFNAMES,
        canonical,
        "End of /NAMES list",
    ));
}

fn send_topic(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    nick: &Bytes,
    chan: &Bytes,
    topic: &Topic,
) {
    user.send(numeric(
        state,
        Target(nick),
        ReplyCode::RPL_TOPIC,
        [chan.clone()],
        Some(topic.text.clone()),
    ));
    user.send(numeric(
        state,
        Target(nick),
        ReplyCode::RPL_TOPICWHOTIME,
        [
            chan.clone(),
            topic.setter.clone(),
            Bytes::from(topic.set_at.to_string().into_bytes()),
        ],
        None,
    ));
}

fn ensure_registered(state: &Arc<ServerState>, user: &Arc<User>) -> bool {
    if user.is_registered() {
        return true;
    }
    user.send(numeric_text(
        state,
        Target::UNREGISTERED,
        ReplyCode::ERR_NOTREGISTERED,
        "You have not registered",
    ));
    false
}

fn is_valid_channel(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.len() > 50 {
        return false;
    }
    if !matches!(bytes[0], b'#' | b'&' | b'+' | b'!') {
        return false;
    }
    bytes
        .iter()
        .skip(1)
        .all(|b| !matches!(*b, 0 | 0x07 | b'\r' | b'\n' | b' ' | b',' | b':'))
}

/// Fan-out: send `msg` to every user in `recipients` that is still
/// live. Silently skips disconnected or closed-queue users.
pub(crate) fn broadcast(
    state: &Arc<ServerState>,
    recipients: &[crate::state::UserId],
    msg: &Message,
) {
    for uid in recipients {
        if let Some(u) = state.user(*uid) {
            u.send(msg.clone());
        }
    }
}
