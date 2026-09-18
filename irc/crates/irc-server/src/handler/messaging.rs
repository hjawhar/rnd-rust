//! Text messaging: PRIVMSG, NOTICE.

use std::sync::Arc;

use bytes::Bytes;
use irc_proto::{Message, Params, Prefix, ReplyCode, Tag, TagKey, Tags, Verb};

use crate::handler::Outcome;
use crate::handler::channel::broadcast;
use crate::numeric::{Target, numeric, numeric_text};
use crate::state::{ServerState, User};

/// Deliver `PRIVMSG <targets> :<text>`.
///
/// Errors on missing targets are surfaced as `ERR_NOSUCHNICK` /
/// `ERR_NOSUCHCHANNEL`. Each target is routed independently.
pub fn handle_privmsg(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    targets: Vec<Bytes>,
    text: &Bytes,
) -> Outcome {
    deliver(state, user, targets, text, b"PRIVMSG", true)
}

/// Deliver `NOTICE <targets> :<text>` — the no-auto-reply variant.
///
/// Per RFC, NOTICE never generates automatic server errors (the
/// receiving client is supposed to never reply). We therefore skip
/// the error numerics even when a target is missing.
pub fn handle_notice(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    targets: Vec<Bytes>,
    text: &Bytes,
) -> Outcome {
    deliver(state, user, targets, text, b"NOTICE", false)
}

fn deliver(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    targets: Vec<Bytes>,
    text: &Bytes,
    verb: &'static [u8],
    emit_errors: bool,
) -> Outcome {
    if !user.is_registered() {
        if emit_errors {
            user.send(numeric_text(
                state,
                Target::UNREGISTERED,
                ReplyCode::ERR_NOTREGISTERED,
                "You have not registered",
            ));
        }
        return Outcome::Continue;
    }
    let origin = user.origin_prefix();
    for target in targets {
        let is_channel = target
            .first()
            .is_some_and(|b| matches!(*b, b'#' | b'&' | b'+' | b'!'));
        if is_channel {
            deliver_to_channel(state, user, &origin, &target, text, verb, emit_errors);
        } else {
            deliver_to_user(state, user, &origin, &target, text, verb, emit_errors);
        }
    }
    Outcome::Continue
}

fn deliver_to_channel(
    state: &Arc<ServerState>,
    from_user: &Arc<User>,
    origin: &Bytes,
    channel: &Bytes,
    text: &Bytes,
    verb: &'static [u8],
    emit_errors: bool,
) {
    let Some(chan) = state.channel(channel.as_ref()) else {
        if emit_errors {
            from_user.send(numeric(
                state,
                Target::UNREGISTERED,
                ReplyCode::ERR_NOSUCHCHANNEL,
                [channel.clone()],
                Some("No such channel".into()),
            ));
        }
        return;
    };
    let guard = chan.read();
    if !guard.has_member(from_user.id()) {
        let canonical = guard.name.clone();
        drop(guard);
        if emit_errors {
            from_user.send(numeric(
                state,
                Target::UNREGISTERED,
                ReplyCode::ERR_CANNOTSENDTOCHAN,
                [canonical],
                Some("Cannot send to channel".into()),
            ));
        }
        return;
    }
    let canonical = guard.name.clone();
    let recipients: Vec<_> = guard
        .members
        .keys()
        .copied()
        .filter(|uid| *uid != from_user.id())
        .collect();
    drop(guard);
    let msg = message_line(origin, verb, &canonical, text);
    broadcast(state, &recipients, &msg);

    // IRCv3 echo-message: send the message back to the sender.
    if from_user.caps().echo_message {
        let echo = maybe_add_time(msg, from_user);
        from_user.send(echo);
    }
}

fn deliver_to_user(
    state: &Arc<ServerState>,
    from_user: &Arc<User>,
    origin: &Bytes,
    target_nick: &Bytes,
    text: &Bytes,
    verb: &'static [u8],
    emit_errors: bool,
) {
    let Some(recipient) = state.user_by_nick(target_nick.as_ref()) else {
        if emit_errors {
            from_user.send(numeric(
                state,
                Target::UNREGISTERED,
                ReplyCode::ERR_NOSUCHNICK,
                [target_nick.clone()],
                Some("No such nick/channel".into()),
            ));
        }
        return;
    };
    let canonical = recipient
        .snapshot()
        .nick
        .unwrap_or_else(|| target_nick.clone());
    let mut msg = message_line(origin, verb, &canonical, text);
    // Add server-time tag if recipient has it enabled.
    if recipient.caps().server_time {
        msg.tags = server_time_tags();
    }
    recipient.send(msg);
}

fn message_line(origin: &Bytes, verb: &'static [u8], target: &Bytes, text: &Bytes) -> Message {
    let mut params = Params::new();
    params.push(target.clone());
    params.push_trailing(text.clone());
    Message {
        tags: Tags::new(),
        prefix: Some(Prefix::User {
            nick: origin.clone(),
            user: None,
            host: None,
        }),
        verb: Verb::word(Bytes::copy_from_slice(verb)),
        params,
    }
}

/// Conditionally add server-time tag when user has it enabled.
fn maybe_add_time(mut msg: Message, user: &Arc<User>) -> Message {
    if user.caps().server_time {
        msg.tags = server_time_tags();
    }
    msg
}

/// Construct a `Tags` set with only the `time` tag.
fn server_time_tags() -> Tags {
    let mut tags = Tags::new();
    tags.push(server_time_tag());
    tags
}

/// A single `time` tag with the current UTC timestamp.
pub(crate) fn server_time_tag() -> Tag {
    Tag {
        key: TagKey {
            client_only: false,
            name: Bytes::from_static(b"time"),
        },
        value: Some(Bytes::from(now_iso8601().into_bytes())),
    }
}

/// ISO 8601 timestamp with millisecond precision.
fn now_iso8601() -> String {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let millis = now.subsec_millis();
    // Manual conversion — avoids pulling in chrono just for this.
    let (year, month, day, hour, min, sec) = epoch_to_utc(secs);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}.{millis:03}Z")
}

/// Convert seconds since UNIX epoch to (year, month, day, hour, min, sec).
#[allow(clippy::similar_names)] // doe/doy from Hinnant's civil-date algorithm
fn epoch_to_utc(epoch: u64) -> (u64, u64, u64, u64, u64, u64) {
    let secs_per_day: u64 = 86400;
    let days = epoch / secs_per_day;
    let day_secs = epoch % secs_per_day;
    let hour = day_secs / 3600;
    let min = (day_secs % 3600) / 60;
    let sec = day_secs % 60;

    // Civil date from days since 1970-01-01 (Algorithm from Howard Hinnant).
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
    (y, m, d, hour, min, sec)
}
