//! Oper command handlers: OPER, KILL, KLINE, UNKLINE, SHOWHOST.

use std::sync::Arc;

use bytes::Bytes;
use irc_proto::{Message, Params, Prefix, ReplyCode, Tags, Verb};
use tracing::info;

use crate::handler::Outcome;
use crate::numeric::{Target, numeric_one, numeric_text, server_name_bytes};
use crate::oper::{Privilege, glob_match, verify_oper_password};
use crate::state::{ServerState, User};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn nick_or_star(user: &User) -> Bytes {
    user.snapshot()
        .nick
        .unwrap_or_else(|| Bytes::from_static(b"*"))
}

fn send_no_privileges(state: &Arc<ServerState>, user: &Arc<User>) {
    let nick = nick_or_star(user);
    user.send(numeric_text(
        state,
        Target(&nick),
        ReplyCode::ERR_NOPRIVILEGES,
        "Permission Denied- You're not an IRC operator",
    ));
}

fn require_registered(state: &Arc<ServerState>, user: &Arc<User>) -> bool {
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

fn require_privilege(state: &Arc<ServerState>, user: &Arc<User>, priv_: Privilege) -> bool {
    if user.has_privilege(state, priv_) {
        return true;
    }
    send_no_privileges(state, user);
    false
}

fn send_notice(state: &Arc<ServerState>, user: &Arc<User>, text: &str) {
    let nick = nick_or_star(user);
    let mut params = Params::new();
    params.push(nick);
    params.push_trailing(Bytes::copy_from_slice(text.as_bytes()));
    user.send(Message {
        tags: Tags::new(),
        prefix: Some(Prefix::Server(server_name_bytes(state))),
        verb: Verb::word(Bytes::from_static(b"NOTICE")),
        params,
    });
}

// ---------------------------------------------------------------------------
// OPER
// ---------------------------------------------------------------------------

/// Handle `OPER <name> <password>`.
pub fn handle_oper(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    params: &irc_proto::Params,
) -> Outcome {
    if !require_registered(state, user) {
        return Outcome::Continue;
    }

    if params.len() < 2 {
        let nick = nick_or_star(user);
        user.send(numeric_one(
            state,
            Target(&nick),
            ReplyCode::ERR_NEEDMOREPARAMS,
            Bytes::from_static(b"OPER"),
            "Not enough parameters",
        ));
        return Outcome::Continue;
    }

    let oper_name = String::from_utf8_lossy(&params[0]);
    let password = String::from_utf8_lossy(&params[1]);
    let nick = nick_or_star(user);

    let config = state.config();
    let block = config.opers.iter().find(|b| b.name == *oper_name);

    let Some(block) = block else {
        user.send(numeric_text(
            state,
            Target(&nick),
            ReplyCode::ERR_NOOPERHOST,
            "No O-lines for your host",
        ));
        return Outcome::Continue;
    };

    // Check password first — wrong password gets ERR_PASSWDMISMATCH
    if !verify_oper_password(block, &password) {
        user.send(numeric_text(
            state,
            Target(&nick),
            ReplyCode::ERR_PASSWDMISMATCH,
            "Password incorrect",
        ));
        return Outcome::Continue;
    }

    // Check allowed_hosts against real host (not cloak)
    if !block.allowed_hosts.is_empty() {
        let snap = user.snapshot();
        let real_host = String::from_utf8_lossy(&snap.host);
        let allowed = block
            .allowed_hosts
            .iter()
            .any(|mask| glob_match(mask, &real_host));
        if !allowed {
            user.send(numeric_text(
                state,
                Target(&nick),
                ReplyCode::ERR_NOOPERHOST,
                "No O-lines for your host",
            ));
            return Outcome::Continue;
        }
    }

    // Check require_account
    if let Some(ref required_account) = block.require_account {
        let snap = user.snapshot();
        let has_account = snap
            .account
            .as_deref()
            .is_some_and(|a| a == required_account);
        if !has_account {
            user.send(numeric_text(
                state,
                Target(&nick),
                ReplyCode::ERR_NOOPERHOST,
                "No O-lines for your host",
            ));
            return Outcome::Continue;
        }
    }

    // Success
    user.set_oper(block.class.clone());

    user.send(numeric_text(
        state,
        Target(&nick),
        ReplyCode::RPL_YOUREOPER,
        "You are now an IRC operator",
    ));

    // Set user mode +o (send MODE message)
    let origin = server_name_bytes(state);
    let mut mode_params = Params::new();
    mode_params.push(nick.clone());
    mode_params.push(Bytes::from_static(b"+o"));
    user.send(Message {
        tags: Tags::new(),
        prefix: Some(Prefix::Server(origin)),
        verb: Verb::word(Bytes::from_static(b"MODE")),
        params: mode_params,
    });

    info!(
        target: "audit",
        oper = %oper_name,
        action = "oper",
        target_nick = %String::from_utf8_lossy(&nick),
        "oper-up successful"
    );

    Outcome::Continue
}

// ---------------------------------------------------------------------------
// KILL
// ---------------------------------------------------------------------------

/// Handle `KILL <nick> <reason>`.
pub fn handle_kill(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    params: &irc_proto::Params,
) -> Outcome {
    if !require_registered(state, user) {
        return Outcome::Continue;
    }
    if !require_privilege(state, user, Privilege::Kill) {
        return Outcome::Continue;
    }

    if params.len() < 2 {
        let nick = nick_or_star(user);
        user.send(numeric_one(
            state,
            Target(&nick),
            ReplyCode::ERR_NEEDMOREPARAMS,
            Bytes::from_static(b"KILL"),
            "Not enough parameters",
        ));
        return Outcome::Continue;
    }

    let target_nick = &params[0];
    let reason = &params[1];

    let Some(target_user) = state.user_by_nick(target_nick) else {
        let nick = nick_or_star(user);
        user.send(numeric_one(
            state,
            Target(&nick),
            ReplyCode::ERR_NOSUCHNICK,
            target_nick.clone(),
            "No such nick/channel",
        ));
        return Outcome::Continue;
    };

    let oper_nick = nick_or_star(user);

    // Send QUIT to channel peers
    let target_id = target_user.id();
    let peers = state.channel_peers(target_id);
    let kill_reason = format!(
        "Killed ({}: {})",
        String::from_utf8_lossy(&oper_nick),
        String::from_utf8_lossy(reason)
    );

    // Send ERROR to the target
    let mut error_params = Params::new();
    error_params.push_trailing(Bytes::copy_from_slice(
        format!("Closing Link: {kill_reason}").as_bytes(),
    ));
    target_user.send(Message {
        tags: Tags::new(),
        prefix: None,
        verb: Verb::word(Bytes::from_static(b"ERROR")),
        params: error_params,
    });

    // Broadcast QUIT to channel peers
    let origin = target_user.origin_prefix();
    let mut quit_params = Params::new();
    quit_params.push_trailing(Bytes::copy_from_slice(kill_reason.as_bytes()));
    let quit_msg = Message {
        tags: Tags::new(),
        prefix: Some(Prefix::User {
            nick: origin,
            user: None,
            host: None,
        }),
        verb: Verb::word(Bytes::from_static(b"QUIT")),
        params: quit_params,
    };
    for uid in &peers {
        if let Some(u) = state.user(*uid) {
            u.send(quit_msg.clone());
        }
    }

    // Clean up: remove from channels, remove from state
    let _ = state.purge_user_from_channels(target_id);
    state.remove_user(target_id);

    info!(
        target: "audit",
        oper = %String::from_utf8_lossy(&oper_nick),
        action = "kill",
        target_nick = %String::from_utf8_lossy(target_nick),
        reason = %String::from_utf8_lossy(reason),
        "user killed"
    );

    Outcome::Continue
}

// ---------------------------------------------------------------------------
// KLINE / UNKLINE
// ---------------------------------------------------------------------------

/// Handle `KLINE <mask> <duration> :<reason>`.
pub fn handle_kline(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    params: &irc_proto::Params,
) -> Outcome {
    if !require_registered(state, user) {
        return Outcome::Continue;
    }
    if !require_privilege(state, user, Privilege::Kline) {
        return Outcome::Continue;
    }

    if params.len() < 3 {
        let nick = nick_or_star(user);
        user.send(numeric_one(
            state,
            Target(&nick),
            ReplyCode::ERR_NEEDMOREPARAMS,
            Bytes::from_static(b"KLINE"),
            "Not enough parameters",
        ));
        return Outcome::Continue;
    }

    let mask = String::from_utf8_lossy(&params[0]).into_owned();
    let duration_str = String::from_utf8_lossy(&params[1]);
    let reason = String::from_utf8_lossy(&params[2]).into_owned();

    let duration_secs: u64 = duration_str.parse().unwrap_or(0);
    let expires = if duration_secs == 0 {
        None
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Some(now + duration_secs)
    };

    let oper_nick = nick_or_star(user);
    let set_by = String::from_utf8_lossy(&oper_nick).into_owned();

    let kl = crate::state::Kline {
        mask: mask.clone(),
        reason: reason.clone(),
        set_by: set_by.clone(),
        expires,
    };
    state.add_kline(kl);

    send_notice(state, user, &format!("K-Line added for {mask}"));

    info!(
        target: "audit",
        oper = %set_by,
        action = "kline",
        mask = %mask,
        duration = duration_secs,
        reason = %reason,
        "kline added"
    );

    Outcome::Continue
}

/// Handle `UNKLINE <mask>`.
pub fn handle_unkline(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    params: &irc_proto::Params,
) -> Outcome {
    if !require_registered(state, user) {
        return Outcome::Continue;
    }
    if !require_privilege(state, user, Privilege::Kline) {
        return Outcome::Continue;
    }

    if params.is_empty() {
        let nick = nick_or_star(user);
        user.send(numeric_one(
            state,
            Target(&nick),
            ReplyCode::ERR_NEEDMOREPARAMS,
            Bytes::from_static(b"UNKLINE"),
            "Not enough parameters",
        ));
        return Outcome::Continue;
    }

    let mask = String::from_utf8_lossy(&params[0]);
    let removed = state.remove_kline(&mask);

    let oper_nick = nick_or_star(user);
    if removed {
        send_notice(state, user, &format!("K-Line removed for {mask}"));
    } else {
        send_notice(state, user, &format!("No K-Line found for {mask}"));
    }

    info!(
        target: "audit",
        oper = %String::from_utf8_lossy(&oper_nick),
        action = "unkline",
        mask = %mask,
        removed = removed,
        "unkline"
    );

    Outcome::Continue
}

// ---------------------------------------------------------------------------
// SHOWHOST
// ---------------------------------------------------------------------------

/// Handle `SHOWHOST <nick>` — reveal a user's real host.
pub fn handle_showhost(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    params: &irc_proto::Params,
) -> Outcome {
    if !require_registered(state, user) {
        return Outcome::Continue;
    }
    if !require_privilege(state, user, Privilege::SeeRealhost) {
        return Outcome::Continue;
    }

    if params.is_empty() {
        let nick = nick_or_star(user);
        user.send(numeric_one(
            state,
            Target(&nick),
            ReplyCode::ERR_NEEDMOREPARAMS,
            Bytes::from_static(b"SHOWHOST"),
            "Not enough parameters",
        ));
        return Outcome::Continue;
    }

    let target_nick = &params[0];
    let Some(target_user) = state.user_by_nick(target_nick) else {
        let nick = nick_or_star(user);
        user.send(numeric_one(
            state,
            Target(&nick),
            ReplyCode::ERR_NOSUCHNICK,
            target_nick.clone(),
            "No such nick/channel",
        ));
        return Outcome::Continue;
    };

    let snap = target_user.snapshot();
    let real_host = String::from_utf8_lossy(&snap.host);
    let oper_nick = nick_or_star(user);

    send_notice(
        state,
        user,
        &format!(
            "Real host of {} is {}",
            String::from_utf8_lossy(target_nick),
            real_host
        ),
    );

    info!(
        target: "audit",
        oper = %String::from_utf8_lossy(&oper_nick),
        action = "showhost",
        target_nick = %String::from_utf8_lossy(target_nick),
        reason = "showhost",
        "real host revealed"
    );

    Outcome::Continue
}
