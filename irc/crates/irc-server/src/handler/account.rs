//! `REGISTER` and `VERIFY` command handlers.

use std::sync::Arc;

use bytes::Bytes;
use irc_proto::{Message, Params, Prefix, ReplyCode, Tags, Verb};

use crate::handler::Outcome;
use crate::numeric::Target;
use crate::state::{ServerState, User};
use crate::store::{Store, StoreError};
/// Handle the `REGISTER` command.
///
/// Params: `REGISTER <account> <email> <password>`
pub async fn handle_register(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    account_name: &[u8],
    email: &[u8],
    password: &[u8],
) -> Outcome {
    let target = user_target(user);

    let (name, email_str, password_str) =
        match validate_register_params(account_name, email, password) {
            Ok(v) => v,
            Err(msg) => {
                send_err(state, &target, user, ReplyCode::ERR_REG_INVALID, msg);
                return Outcome::Continue;
            }
        };

    let Ok(hash) = crate::account::hash_password(password_str) else {
        send_err(
            state,
            &target,
            user,
            ReplyCode::ERR_REG_INVALID,
            "Internal hashing error",
        );
        return Outcome::Continue;
    };

    let token = crate::account::generate_verify_token();

    match state
        .store()
        .account_create(name, email_str, &hash, &token)
        .await
    {
        Ok(()) => {
            let msg = crate::numeric::numeric_text(
                state,
                target,
                ReplyCode::RPL_REG_VERIFICATION_REQUIRED,
                Bytes::from(format!("{name} :Verification required. Check your email.")),
            );
            user.send(msg);
        }
        Err(StoreError::AlreadyExists) => {
            send_err(
                state,
                &target,
                user,
                ReplyCode::ERR_REG_ALREADY_EXISTS,
                "Account already exists",
            );
        }
        Err(_) => {
            send_err(
                state,
                &target,
                user,
                ReplyCode::ERR_REG_INVALID,
                "Registration failed",
            );
        }
    }

    Outcome::Continue
}

/// Handle the `VERIFY` command.
///
/// Params: `VERIFY <account> <token>`
pub async fn handle_verify(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    account_name: &[u8],
    token: &[u8],
) -> Outcome {
    let target = user_target(user);

    let Ok(name) = std::str::from_utf8(account_name) else {
        send_err(
            state,
            &target,
            user,
            ReplyCode::ERR_REG_INVALID,
            "Invalid account name encoding",
        );
        return Outcome::Continue;
    };
    let Ok(token_str) = std::str::from_utf8(token) else {
        send_err(
            state,
            &target,
            user,
            ReplyCode::ERR_REG_INVALID,
            "Invalid token encoding",
        );
        return Outcome::Continue;
    };

    match state.store().account_verify(name, token_str).await {
        Ok(true) => {
            user.set_account(name.to_owned());
            broadcast_account_notify(state, user, name);
            let msg = crate::numeric::numeric_text(
                state,
                target,
                ReplyCode::RPL_REG_SUCCESS,
                Bytes::from(format!("{name} :Account verified successfully")),
            );
            user.send(msg);
        }
        Ok(false) => {
            send_err(
                state,
                &target,
                user,
                ReplyCode::ERR_REG_INVALID,
                "Invalid verify token",
            );
        }
        Err(_) => {
            send_err(
                state,
                &target,
                user,
                ReplyCode::ERR_REG_INVALID,
                "Verification failed",
            );
        }
    }

    Outcome::Continue
}

/// Decode and validate the raw `REGISTER` parameters.
///
/// Returns the decoded `(account, email, password)` strings or an error message.
fn validate_register_params<'a>(
    account_name: &'a [u8],
    email: &'a [u8],
    password: &'a [u8],
) -> Result<(&'a str, &'a str, &'a str), &'static str> {
    let name = std::str::from_utf8(account_name).map_err(|_| "Invalid account name encoding")?;
    let email_str = std::str::from_utf8(email).map_err(|_| "Invalid email encoding")?;
    let password_str = std::str::from_utf8(password).map_err(|_| "Invalid password encoding")?;

    if name.is_empty() || name.len() > 32 {
        return Err("Account name must be 1-32 characters");
    }
    if email_str.is_empty() || email_str.len() > 320 {
        return Err("Invalid email address");
    }
    if password_str.len() < 5 || password_str.len() > 512 {
        return Err("Password must be 5-512 characters");
    }

    Ok((name, email_str, password_str))
}

fn user_target(user: &User) -> Target<'_> {
    let snap = user.snapshot();
    if let Some(ref nick) = snap.nick {
        // Leaking here is fine for the lifetime of a single message
        // dispatch — the nick bytes are already `Arc`-backed.
        let leaked: &'static [u8] = Vec::leak(nick.to_vec());
        Target(leaked)
    } else {
        Target::UNREGISTERED
    }
}

fn send_err(state: &ServerState, target: &Target<'_>, user: &User, code: ReplyCode, text: &str) {
    let msg = crate::numeric::numeric_text(state, *target, code, Bytes::from(text.to_owned()));
    user.send(msg);
}

/// Send `:<prefix> ACCOUNT <account>` to channel peers with account-notify.
fn broadcast_account_notify(state: &ServerState, user: &Arc<User>, account: &str) {
    let origin = user.origin_prefix();
    let peers = state.channel_peers(user.id());
    let mut params = Params::new();
    params.push(Bytes::copy_from_slice(account.as_bytes()));
    let msg = Message {
        tags: Tags::new(),
        prefix: Some(Prefix::User {
            nick: origin,
            user: None,
            host: None,
        }),
        verb: Verb::word(Bytes::from_static(b"ACCOUNT")),
        params,
    };
    for uid in peers {
        if let Some(peer) = state.user(uid) {
            if peer.caps().account_notify {
                peer.send(msg.clone());
            }
        }
    }
}
