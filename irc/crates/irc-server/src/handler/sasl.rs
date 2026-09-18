//! SASL PLAIN and EXTERNAL authentication handlers.

use std::sync::Arc;

use bytes::Bytes;
use irc_proto::{Message, Params, Prefix, ReplyCode, Tags, Verb};

use crate::handler::Outcome;
use crate::numeric::{Target, numeric_text, server_name_bytes};
use crate::state::{ServerState, User};
use crate::store::Store;
/// Handle `AUTHENTICATE <payload>`.
///
/// If `payload` is a mechanism name (`PLAIN` or `EXTERNAL`), record
/// it and send `AUTHENTICATE +`.  Otherwise, treat `payload` as
/// base64-encoded credentials and attempt authentication.
pub async fn handle_authenticate(
    state: &Arc<ServerState>,
    user: &Arc<User>,
    payload: Bytes,
) -> Outcome {
    let target = user_target(user);

    // Mechanism negotiation step.
    if payload.eq_ignore_ascii_case(b"PLAIN") || payload.eq_ignore_ascii_case(b"EXTERNAL") {
        {
            let mut inner = user.inner_write();
            inner.sasl_mechanism = Some(String::from_utf8_lossy(&payload).to_ascii_uppercase());
        }
        // Reply with `AUTHENTICATE +`
        let msg = Message {
            tags: Tags::new(),
            prefix: Some(Prefix::Server(server_name_bytes(state))),
            verb: Verb::word(Bytes::from_static(b"AUTHENTICATE")),
            params: {
                let mut p = Params::new();
                p.push(Bytes::from_static(b"+"));
                p
            },
        };
        user.send(msg);
        return Outcome::Continue;
    }

    // Credential step — decode and dispatch based on stored mechanism.
    let mechanism = user.snapshot().sasl_mechanism.clone();
    match mechanism.as_deref() {
        Some("PLAIN") => handle_plain(state, user, &target, &payload).await,
        Some("EXTERNAL") => handle_external(state, user, &target).await,
        _ => {
            send_sasl_fail(state, user, &target, "No SASL mechanism negotiated");
        }
    }

    Outcome::Continue
}

async fn handle_plain(state: &ServerState, user: &Arc<User>, target: &Target<'_>, payload: &[u8]) {
    use base64::Engine;
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(payload) else {
        send_sasl_fail(state, user, target, "Invalid base64");
        return;
    };

    // Format: authzid\0authcid\0passwd
    let parts: Vec<&[u8]> = decoded.splitn(3, |&b| b == 0).collect();
    if parts.len() != 3 {
        send_sasl_fail(state, user, target, "Malformed PLAIN payload");
        return;
    }

    let Ok(authcid) = std::str::from_utf8(parts[1]) else {
        send_sasl_fail(state, user, target, "Invalid UTF-8 in authcid");
        return;
    };
    let Ok(passwd) = std::str::from_utf8(parts[2]) else {
        send_sasl_fail(state, user, target, "Invalid UTF-8 in password");
        return;
    };

    let Ok(Some(acct)) = state.store().account_get(authcid).await else {
        send_sasl_fail(state, user, target, "Authentication failed");
        return;
    };

    if !acct.verified {
        send_sasl_fail(state, user, target, "Account not verified");
        return;
    }

    match crate::account::verify_password(&acct.password_hash, passwd) {
        Ok(true) => {
            user.set_account(acct.name.clone());
            broadcast_account_notify(state, user, &acct.name);
            send_logged_in(state, user, target, &acct.name);
        }
        _ => {
            send_sasl_fail(state, user, target, "Authentication failed");
        }
    }
}

async fn handle_external(state: &ServerState, user: &Arc<User>, target: &Target<'_>) {
    let fp = user.snapshot().cert_fingerprint.clone();
    let Some(fp) = fp else {
        send_sasl_fail(state, user, target, "No client certificate");
        return;
    };

    let Ok(Some(acct)) = state.store().account_by_fingerprint(&fp).await else {
        send_sasl_fail(state, user, target, "No account for certificate");
        return;
    };

    if !acct.verified {
        send_sasl_fail(state, user, target, "Account not verified");
        return;
    }

    user.set_account(acct.name.clone());
    broadcast_account_notify(state, user, &acct.name);
    send_logged_in(state, user, target, &acct.name);
}

fn send_logged_in(state: &ServerState, user: &User, target: &Target<'_>, account: &str) {
    // RPL_LOGGEDIN (900) <nick> <prefix> <account> :You are now logged in as <account>
    let prefix = user.origin_prefix();
    let msg = crate::numeric::numeric(
        state,
        *target,
        ReplyCode::RPL_LOGGEDIN,
        [prefix, Bytes::copy_from_slice(account.as_bytes())],
        Some(Bytes::from(format!("You are now logged in as {account}"))),
    );
    user.send(msg);

    // RPL_SASLSUCCESS (903)
    let msg = numeric_text(
        state,
        *target,
        ReplyCode::RPL_SASLSUCCESS,
        "SASL authentication successful",
    );
    user.send(msg);

    // Clear mechanism.
    user.inner_write().sasl_mechanism = None;
}

fn send_sasl_fail(state: &ServerState, user: &User, target: &Target<'_>, text: &str) {
    let msg = numeric_text(
        state,
        *target,
        ReplyCode::ERR_SASLFAIL,
        Bytes::from(text.to_owned()),
    );
    user.send(msg);
    // Clear mechanism on failure.
    user.inner_write().sasl_mechanism = None;
}

fn user_target(user: &User) -> Target<'static> {
    let snap = user.snapshot();
    if let Some(ref nick) = snap.nick {
        let leaked: &'static [u8] = Vec::leak(nick.to_vec());
        Target(leaked)
    } else {
        Target::UNREGISTERED
    }
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
