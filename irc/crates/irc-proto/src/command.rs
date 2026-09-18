//! Typed command surface.
//!
//! Sits on top of the wire [`crate::Message`] as a semantic layer:
//!
//! - [`Command::parse`] maps a [`Message`] to one of the typed variants
//!   (or [`Command::Unknown`] / [`Command::Numeric`] for things we
//!   haven't modelled yet).
//! - [`Command::to_message`] goes the other way, producing a wire
//!   message suitable for serialization.
//!
//! Phase 1 covers the RFC 2812 baseline plus the IRCv3 commands needed
//! for SASL and CAP negotiation. Additional variants land per phase.

use bytes::{BufMut, Bytes, BytesMut};
use thiserror::Error;

use crate::message::Message;
use crate::params::Params;
use crate::tags::Tags;
use crate::verb::Verb;

/// Errors returned when promoting a wire [`Message`] to a typed
/// [`Command`].
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// Not enough params for the command to be parsed semantically.
    #[error("command `{command}` requires at least {required} param(s), got {got}")]
    MissingParam {
        /// The command word that demanded the param.
        command: &'static str,
        /// Minimum param count.
        required: usize,
        /// Actual param count on the wire.
        got: usize,
    },
}

/// Typed IRC command surface.
///
/// Unmodelled commands fall through to [`Command::Unknown`]; unmodelled
/// numerics fall through to [`Command::Numeric`]. Callers that want to
/// dispatch on every possible command should pattern-match the typed
/// variants and fall through the catch-alls on the wildcard arm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Command {
    /// `CAP` subcommand exchange (IRCv3).
    Cap {
        /// Subcommand: `LS`, `LIST`, `REQ`, `ACK`, `NAK`, `NEW`, `DEL`, `END`, or other.
        subcommand: CapSub,
        /// Further args as they appear on the wire.
        args: Vec<Bytes>,
    },
    /// `NICK <nickname>` — request the given nickname.
    Nick {
        /// Requested nickname.
        nick: Bytes,
    },
    /// `USER <user> <mode> <*> :<realname>`.
    User {
        /// `user` parameter (historically the local user name).
        user: Bytes,
        /// Mode flags (0-9 bitmask) submitted at registration. RFC 2812
        /// specifies this as a numeric; we preserve the raw bytes.
        mode: Bytes,
        /// Real name (trailing).
        realname: Bytes,
    },
    /// `PASS <password>`.
    Pass {
        /// Connection password.
        password: Bytes,
    },
    /// `AUTHENTICATE <payload>` — SASL step.
    Authenticate {
        /// Mechanism name (first step) or base64-encoded payload.
        payload: Bytes,
    },
    /// `QUIT [:reason]`.
    Quit {
        /// Optional reason.
        reason: Option<Bytes>,
    },
    /// `PING <token> [server]`.
    Ping {
        /// Token echoed by the peer's PONG.
        token: Bytes,
        /// Optional target server.
        server: Option<Bytes>,
    },
    /// `PONG <token> [server]`.
    Pong {
        /// Token matching the prior PING.
        token: Bytes,
        /// Optional target server.
        server: Option<Bytes>,
    },
    /// `JOIN <channels> [keys]`.
    Join {
        /// Comma-separated channels, parsed.
        channels: Vec<Bytes>,
        /// Comma-separated keys in the same order as `channels`. May be
        /// shorter than `channels`; trailing channels have no key.
        keys: Vec<Bytes>,
    },
    /// `PART <channels> [:reason]`.
    Part {
        /// Comma-separated channels, parsed.
        channels: Vec<Bytes>,
        /// Optional part message.
        reason: Option<Bytes>,
    },
    /// `PRIVMSG <targets> :<text>`.
    Privmsg {
        /// Comma-separated targets (nicks and/or channels).
        targets: Vec<Bytes>,
        /// Message body (trailing).
        text: Bytes,
    },
    /// `NOTICE <targets> :<text>`.
    Notice {
        /// Comma-separated targets.
        targets: Vec<Bytes>,
        /// Message body.
        text: Bytes,
    },
    /// `TOPIC <channel> [:<topic>]`.
    Topic {
        /// Target channel.
        channel: Bytes,
        /// New topic (write) or absent (read).
        topic: Option<Bytes>,
    },
    /// `MODE <target> [changes] [args ...]`.
    Mode {
        /// Target channel or nick.
        target: Bytes,
        /// Mode change string (e.g. `+o-v`). Absent → query mode.
        changes: Option<Bytes>,
        /// Mode arguments (nick/mask/key/limit).
        args: Vec<Bytes>,
    },
    /// `LIST [<channels>] [<server>]`.
    List {
        /// Optional list of channels to query (empty = all).
        channels: Vec<Bytes>,
        /// Optional target server.
        server: Option<Bytes>,
    },
    /// Three-digit numeric reply (parsed `u16`, params preserved raw).
    Numeric {
        /// Three-digit reply code.
        code: u16,
        /// Raw params, including the trailing if present.
        params: Params,
    },
    /// Verb we haven't modelled (or an application-defined extension).
    Unknown {
        /// Verb bytes.
        verb: Bytes,
        /// Raw params.
        params: Params,
    },
}

/// Subcommands of `CAP`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapSub {
    /// `LS` — list available caps.
    Ls,
    /// `LIST` — list enabled caps.
    List,
    /// `REQ` — request caps.
    Req,
    /// `ACK` — server acknowledges.
    Ack,
    /// `NAK` — server rejects.
    Nak,
    /// `NEW` — server announces a new cap available (IRCv3 `cap-notify`).
    New,
    /// `DEL` — server announces a cap being removed.
    Del,
    /// `END` — client ends cap negotiation.
    End,
    /// Anything else, preserved verbatim.
    Other(Bytes),
}

impl CapSub {
    fn parse(bytes: &[u8]) -> Self {
        // CAP subcommands are case-insensitive per spec.
        match bytes.to_ascii_uppercase().as_slice() {
            b"LS" => Self::Ls,
            b"LIST" => Self::List,
            b"REQ" => Self::Req,
            b"ACK" => Self::Ack,
            b"NAK" => Self::Nak,
            b"NEW" => Self::New,
            b"DEL" => Self::Del,
            b"END" => Self::End,
            _ => Self::Other(Bytes::copy_from_slice(bytes)),
        }
    }

    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Ls => b"LS",
            Self::List => b"LIST",
            Self::Req => b"REQ",
            Self::Ack => b"ACK",
            Self::Nak => b"NAK",
            Self::New => b"NEW",
            Self::Del => b"DEL",
            Self::End => b"END",
            Self::Other(b) => b.as_ref(),
        }
    }
}

impl Command {
    /// Promote a wire [`Message`] into a typed [`Command`].
    ///
    /// Unrecognised verbs surface as [`Command::Unknown`]; insufficient
    /// params for a modelled verb surface as [`CommandError::MissingParam`].
    pub fn parse(msg: &Message) -> Result<Self, CommandError> {
        match &msg.verb {
            Verb::Numeric(code) => Ok(Self::Numeric {
                code: *code,
                params: msg.params.clone(),
            }),
            Verb::Word(word) => parse_word(word, &msg.params),
        }
    }

    /// Serialize this command into a wire [`Message`]. Tags and prefix
    /// default to empty / `None`; callers that need to attach either
    /// build the [`Message`] directly.
    #[must_use]
    pub fn to_message(&self) -> Message {
        let (verb, params) = self.to_verb_params();
        Message {
            tags: Tags::new(),
            prefix: None,
            verb,
            params,
        }
    }

    #[allow(clippy::too_many_lines)] // exhaustive match over all Command variants
    fn to_verb_params(&self) -> (Verb, Params) {
        match self {
            Self::Cap { subcommand, args } => {
                let mut p = Params::new();
                p.push(Bytes::copy_from_slice(subcommand.as_bytes()));
                for a in args {
                    p.push(a.clone());
                }
                // Last arg is frequently a space-separated list; mark
                // it trailing if needed so it survives round-trip.
                promote_last_to_trailing_if_needed(&mut p);
                (Verb::word(Bytes::from_static(b"CAP")), p)
            }
            Self::Nick { nick } => {
                let mut p = Params::new();
                p.push(nick.clone());
                (Verb::word(Bytes::from_static(b"NICK")), p)
            }
            Self::User {
                user,
                mode,
                realname,
            } => {
                let mut p = Params::new();
                p.push(user.clone());
                p.push(mode.clone());
                p.push(Bytes::from_static(b"*"));
                p.push_trailing(realname.clone());
                (Verb::word(Bytes::from_static(b"USER")), p)
            }
            Self::Pass { password } => {
                let mut p = Params::new();
                p.push(password.clone());
                (Verb::word(Bytes::from_static(b"PASS")), p)
            }
            Self::Authenticate { payload } => {
                let mut p = Params::new();
                p.push(payload.clone());
                (Verb::word(Bytes::from_static(b"AUTHENTICATE")), p)
            }
            Self::Quit { reason } => {
                let mut p = Params::new();
                if let Some(r) = reason {
                    p.push_trailing(r.clone());
                }
                (Verb::word(Bytes::from_static(b"QUIT")), p)
            }
            Self::Ping { token, server } => build_token_server(b"PING", token, server.as_ref()),
            Self::Pong { token, server } => build_token_server(b"PONG", token, server.as_ref()),
            Self::Join { channels, keys } => {
                let mut p = Params::new();
                p.push(join_commas(channels));
                if !keys.is_empty() {
                    p.push(join_commas(keys));
                }
                (Verb::word(Bytes::from_static(b"JOIN")), p)
            }
            Self::Part { channels, reason } => {
                let mut p = Params::new();
                p.push(join_commas(channels));
                if let Some(r) = reason {
                    p.push_trailing(r.clone());
                }
                (Verb::word(Bytes::from_static(b"PART")), p)
            }
            Self::Privmsg { targets, text } => {
                let mut p = Params::new();
                p.push(join_commas(targets));
                p.push_trailing(text.clone());
                (Verb::word(Bytes::from_static(b"PRIVMSG")), p)
            }
            Self::Notice { targets, text } => {
                let mut p = Params::new();
                p.push(join_commas(targets));
                p.push_trailing(text.clone());
                (Verb::word(Bytes::from_static(b"NOTICE")), p)
            }
            Self::Topic { channel, topic } => {
                let mut p = Params::new();
                p.push(channel.clone());
                if let Some(t) = topic {
                    p.push_trailing(t.clone());
                }
                (Verb::word(Bytes::from_static(b"TOPIC")), p)
            }
            Self::Mode {
                target,
                changes,
                args,
            } => {
                let mut p = Params::new();
                p.push(target.clone());
                if let Some(c) = changes {
                    p.push(c.clone());
                }
                for a in args {
                    p.push(a.clone());
                }
                (Verb::word(Bytes::from_static(b"MODE")), p)
            }
            Self::List { channels, server } => {
                let mut p = Params::new();
                if !channels.is_empty() {
                    p.push(join_commas(channels));
                }
                if let Some(s) = server {
                    p.push(s.clone());
                }
                (Verb::word(Bytes::from_static(b"LIST")), p)
            }
            Self::Numeric { code, params } => (Verb::Numeric(*code), params.clone()),
            Self::Unknown { verb, params } => (Verb::Word(verb.clone()), params.clone()),
        }
    }
}

fn build_token_server(
    verb: &'static [u8],
    token: &Bytes,
    server: Option<&Bytes>,
) -> (Verb, Params) {
    let mut p = Params::new();
    p.push(token.clone());
    if let Some(s) = server {
        p.push(s.clone());
    }
    // If the token contains a space, auto-promote to trailing is
    // handled by Params::write.
    (Verb::Word(Bytes::copy_from_slice(verb)), p)
}

fn parse_word(word: &Bytes, params: &Params) -> Result<Command, CommandError> {
    // ASCII-uppercase-fold the verb for matching. Command verbs are
    // ASCII per the grammar so to_ascii_uppercase never widens a byte.
    let upper = word.to_ascii_uppercase();
    match upper.as_slice() {
        b"CAP" => parse_cap(params),
        b"NICK" => parse_one(params, "NICK").map(|nick| Command::Nick { nick }),
        b"USER" => parse_user(params),
        b"PASS" => parse_one(params, "PASS").map(|password| Command::Pass { password }),
        b"AUTHENTICATE" => {
            parse_one(params, "AUTHENTICATE").map(|payload| Command::Authenticate { payload })
        }
        b"QUIT" => Ok(Command::Quit {
            reason: params.get(0).cloned(),
        }),
        b"PING" => parse_ping_or_pong(params, "PING", true),
        b"PONG" => parse_ping_or_pong(params, "PONG", false),
        b"JOIN" => parse_join(params),
        b"PART" => parse_part(params),
        b"PRIVMSG" => parse_privmsg(params, false),
        b"NOTICE" => parse_privmsg(params, true),
        b"TOPIC" => parse_topic(params),
        b"MODE" => parse_mode(params),
        b"LIST" => Ok(Command::List {
            channels: params.get(0).map(split_commas).unwrap_or_default(),
            server: params.get(1).cloned(),
        }),
        _ => Ok(Command::Unknown {
            verb: word.clone(),
            params: params.clone(),
        }),
    }
}

fn parse_one(params: &Params, name: &'static str) -> Result<Bytes, CommandError> {
    params.get(0).cloned().ok_or(CommandError::MissingParam {
        command: name,
        required: 1,
        got: 0,
    })
}

fn parse_cap(params: &Params) -> Result<Command, CommandError> {
    let Some(sub_bytes) = params.get(0) else {
        return Err(CommandError::MissingParam {
            command: "CAP",
            required: 1,
            got: 0,
        });
    };
    let subcommand = CapSub::parse(sub_bytes);
    let args = params.iter().skip(1).cloned().collect();
    Ok(Command::Cap { subcommand, args })
}

fn parse_user(params: &Params) -> Result<Command, CommandError> {
    if params.len() < 4 {
        return Err(CommandError::MissingParam {
            command: "USER",
            required: 4,
            got: params.len(),
        });
    }
    Ok(Command::User {
        user: params[0].clone(),
        mode: params[1].clone(),
        realname: params[3].clone(),
    })
}

fn parse_ping_or_pong(
    params: &Params,
    name: &'static str,
    is_ping: bool,
) -> Result<Command, CommandError> {
    let token = params.get(0).cloned().ok_or(CommandError::MissingParam {
        command: name,
        required: 1,
        got: 0,
    })?;
    let server = params.get(1).cloned();
    Ok(if is_ping {
        Command::Ping { token, server }
    } else {
        Command::Pong { token, server }
    })
}

fn parse_join(params: &Params) -> Result<Command, CommandError> {
    let chans = params.get(0).ok_or(CommandError::MissingParam {
        command: "JOIN",
        required: 1,
        got: 0,
    })?;
    Ok(Command::Join {
        channels: split_commas(chans),
        keys: params.get(1).map(split_commas).unwrap_or_default(),
    })
}

fn parse_part(params: &Params) -> Result<Command, CommandError> {
    let chans = params.get(0).ok_or(CommandError::MissingParam {
        command: "PART",
        required: 1,
        got: 0,
    })?;
    Ok(Command::Part {
        channels: split_commas(chans),
        reason: params.get(1).cloned(),
    })
}

fn parse_privmsg(params: &Params, notice: bool) -> Result<Command, CommandError> {
    let command = if notice { "NOTICE" } else { "PRIVMSG" };
    if params.len() < 2 {
        return Err(CommandError::MissingParam {
            command,
            required: 2,
            got: params.len(),
        });
    }
    let targets = split_commas(&params[0]);
    let text = params[1].clone();
    Ok(if notice {
        Command::Notice { targets, text }
    } else {
        Command::Privmsg { targets, text }
    })
}

fn parse_topic(params: &Params) -> Result<Command, CommandError> {
    let channel = params.get(0).cloned().ok_or(CommandError::MissingParam {
        command: "TOPIC",
        required: 1,
        got: 0,
    })?;
    Ok(Command::Topic {
        channel,
        topic: params.get(1).cloned(),
    })
}

fn parse_mode(params: &Params) -> Result<Command, CommandError> {
    let target = params.get(0).cloned().ok_or(CommandError::MissingParam {
        command: "MODE",
        required: 1,
        got: 0,
    })?;
    let changes = params.get(1).cloned();
    let args = params.iter().skip(2).cloned().collect();
    Ok(Command::Mode {
        target,
        changes,
        args,
    })
}

fn split_commas(src: &Bytes) -> Vec<Bytes> {
    let bytes: &[u8] = src.as_ref();
    // Pre-size by the number of commas; micro-opt for the common case.
    #[allow(clippy::naive_bytecount)] // pulling in bytecount for one call site isn't worth a dep
    let mut out = Vec::with_capacity(bytes.iter().filter(|b| **b == b',').count() + 1);
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b',' {
            out.push(src.slice(start..i));
            start = i + 1;
        }
    }
    out.push(src.slice(start..bytes.len()));
    out
}

fn join_commas(items: &[Bytes]) -> Bytes {
    if items.is_empty() {
        return Bytes::new();
    }
    let total: usize = items.iter().map(bytes::Bytes::len).sum::<usize>() + items.len() - 1;
    let mut out = BytesMut::with_capacity(total);
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.put_u8(b',');
        }
        out.extend_from_slice(item.as_ref());
    }
    out.freeze()
}

fn promote_last_to_trailing_if_needed(params: &mut Params) {
    // CAP LS-style lists are space-separated and must be sent as trailing.
    if let Some(last) = params.last() {
        if last.contains(&b' ') {
            // Replace the last value with a push_trailing to set the flag.
            let last = last.clone();
            // Rebuild: drain everything except last, push_trailing(last).
            let len = params.len();
            let mut rebuilt = Params::new();
            for item in params.iter().take(len - 1) {
                rebuilt.push(item.clone());
            }
            rebuilt.push_trailing(last);
            *params = rebuilt;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CapSub, Command};
    use crate::Message;
    use bytes::Bytes;

    fn parse(s: &str) -> Command {
        let msg = Message::parse_slice(s.as_bytes()).expect("wire parse");
        Command::parse(&msg).expect("command parse")
    }

    fn round_trip(s: &str) -> String {
        let msg = Message::parse_slice(s.as_bytes()).expect("wire parse");
        let cmd = Command::parse(&msg).expect("command parse");
        let rebuilt = cmd.to_message();
        String::from_utf8(rebuilt.to_bytes().to_vec()).unwrap()
    }

    #[test]
    fn parses_ping_with_token() {
        match parse("PING :abc") {
            Command::Ping { token, server } => {
                assert_eq!(&*token, b"abc");
                assert!(server.is_none());
            }
            other => panic!("expected Ping, got {other:?}"),
        }
    }

    #[test]
    fn parses_nick_command() {
        match parse("NICK alice") {
            Command::Nick { nick } => assert_eq!(&*nick, b"alice"),
            other => panic!("expected Nick, got {other:?}"),
        }
    }

    #[test]
    fn parses_user_four_params() {
        match parse("USER alice 0 * :Alice Example") {
            Command::User {
                user,
                mode,
                realname,
            } => {
                assert_eq!(&*user, b"alice");
                assert_eq!(&*mode, b"0");
                assert_eq!(&*realname, b"Alice Example");
            }
            other => panic!("expected User, got {other:?}"),
        }
    }

    #[test]
    fn user_rejects_missing_params() {
        let msg = Message::parse_slice(b"USER alice 0 *").unwrap();
        assert!(Command::parse(&msg).is_err());
    }

    #[test]
    fn parses_join_with_multiple_channels_and_keys() {
        match parse("JOIN #a,#b k1,k2") {
            Command::Join { channels, keys } => {
                assert_eq!(channels.len(), 2);
                assert_eq!(&*channels[0], b"#a");
                assert_eq!(&*channels[1], b"#b");
                assert_eq!(keys.len(), 2);
                assert_eq!(&*keys[0], b"k1");
                assert_eq!(&*keys[1], b"k2");
            }
            other => panic!("expected Join, got {other:?}"),
        }
    }

    #[test]
    fn parses_privmsg_with_multi_targets() {
        match parse("PRIVMSG #a,alice :hello") {
            Command::Privmsg { targets, text } => {
                assert_eq!(targets.len(), 2);
                assert_eq!(&*targets[0], b"#a");
                assert_eq!(&*targets[1], b"alice");
                assert_eq!(&*text, b"hello");
            }
            other => panic!("expected Privmsg, got {other:?}"),
        }
    }

    #[test]
    fn parses_cap_subcommand() {
        match parse("CAP LS 302") {
            Command::Cap { subcommand, args } => {
                assert_eq!(subcommand, CapSub::Ls);
                assert_eq!(args.len(), 1);
                assert_eq!(&*args[0], b"302");
            }
            other => panic!("expected Cap, got {other:?}"),
        }
    }

    #[test]
    fn parses_mode_query_no_changes() {
        match parse("MODE #rust") {
            Command::Mode {
                target,
                changes,
                args,
            } => {
                assert_eq!(&*target, b"#rust");
                assert!(changes.is_none());
                assert!(args.is_empty());
            }
            other => panic!("expected Mode, got {other:?}"),
        }
    }

    #[test]
    fn parses_mode_with_args() {
        match parse("MODE #rust +o alice") {
            Command::Mode {
                target,
                changes,
                args,
            } => {
                assert_eq!(&*target, b"#rust");
                assert_eq!(changes.as_deref(), Some(&b"+o"[..]));
                assert_eq!(args.len(), 1);
                assert_eq!(&*args[0], b"alice");
            }
            other => panic!("expected Mode, got {other:?}"),
        }
    }

    #[test]
    fn unknown_verb_preserved_with_raw_params() {
        match parse("FOOBAR 1 2 3") {
            Command::Unknown { verb, params } => {
                assert_eq!(&*verb, b"FOOBAR");
                assert_eq!(params.len(), 3);
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn numeric_kept_with_params() {
        match parse(":server 001 alice :Welcome") {
            Command::Numeric { code, params } => {
                assert_eq!(code, 1);
                assert_eq!(params.len(), 2);
                assert_eq!(&*params[0], b"alice");
                assert_eq!(&*params[1], b"Welcome");
            }
            other => panic!("expected Numeric, got {other:?}"),
        }
    }

    #[test]
    fn round_trip_nick() {
        assert_eq!(round_trip("NICK alice"), "NICK alice");
    }

    #[test]
    fn round_trip_user() {
        assert_eq!(
            round_trip("USER alice 0 * :Alice Example"),
            "USER alice 0 * :Alice Example"
        );
    }

    #[test]
    fn round_trip_privmsg_multi_target() {
        assert_eq!(
            round_trip("PRIVMSG #a,alice :hello world"),
            "PRIVMSG #a,alice :hello world"
        );
    }

    #[test]
    fn round_trip_join_with_keys() {
        assert_eq!(round_trip("JOIN #a,#b k1,k2"), "JOIN #a,#b k1,k2");
    }

    #[test]
    fn round_trip_cap_ls() {
        assert_eq!(round_trip("CAP LS 302"), "CAP LS 302");
    }

    #[test]
    fn construct_and_write_part_with_reason() {
        let cmd = Command::Part {
            channels: vec![Bytes::from_static(b"#rust")],
            reason: Some(Bytes::from_static(b"bye")),
        };
        let wire = cmd.to_message().to_bytes();
        assert_eq!(wire.as_ref(), b"PART #rust :bye");
    }

    #[test]
    fn quit_without_reason_writes_no_params() {
        let cmd = Command::Quit { reason: None };
        assert_eq!(cmd.to_message().to_bytes().as_ref(), b"QUIT");
    }
}
