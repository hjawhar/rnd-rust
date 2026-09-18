//! Per-network connection session.

use std::sync::Arc;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_rustls::TlsConnector;
use tokio_util::codec::Framed;
use tracing::{debug, error, warn};

use irc_proto::command::Command;
use irc_proto::prefix::Prefix;
use irc_proto::{IrcCodec, Message, Params};

use crate::command::ClientCommand;
use crate::event::{ClientEvent, NetworkId};
use crate::state::NetworkState;

/// Handle to a running network session.
///
/// Dropping the handle does not immediately stop the session; the session
/// shuts down when its command channel is closed.
pub struct NetworkHandle {
    /// Sender for commands targeted at this network.
    pub command_tx: mpsc::Sender<ClientCommand>,
    /// Join handle for the session task.
    pub join_handle: tokio::task::JoinHandle<()>,
}

/// Configuration for a network connection.
pub struct NetworkConfig {
    /// Network identifier.
    pub id: NetworkId,
    /// Hostname or IP to connect to.
    pub host: String,
    /// Port number.
    pub port: u16,
    /// Whether to use TLS.
    pub tls: bool,
    /// Desired nick.
    pub nick: Bytes,
    /// Username for the USER command.
    pub user: Bytes,
    /// Realname for the USER command.
    pub realname: Bytes,
}

/// Run a network session to completion.
///
/// Connects to the configured host:port (with optional TLS), registers with
/// the given nick/user/realname, and then loops: reading messages from the
/// wire and translating them to [`ClientEvent`]s, and receiving
/// [`ClientCommand`]s from the frontend and translating them to wire
/// messages.
pub async fn run_network(
    config: NetworkConfig,
    event_tx: mpsc::Sender<ClientEvent>,
    mut cmd_rx: mpsc::Receiver<ClientCommand>,
) {
    let id = config.id;
    let stream = match TcpStream::connect((&*config.host, config.port)).await {
        Ok(s) => s,
        Err(e) => {
            let _ = event_tx
                .send(ClientEvent::Error {
                    network: id,
                    message: format!("connect failed: {e}"),
                })
                .await;
            return;
        }
    };

    let _ = event_tx.send(ClientEvent::Connected { network: id }).await;

    if config.tls {
        match tls_connect(&config.host, stream).await {
            Ok(tls_stream) => {
                run_session(
                    id,
                    tls_stream,
                    &config.nick,
                    &config.user,
                    &config.realname,
                    &event_tx,
                    &mut cmd_rx,
                )
                .await;
            }
            Err(e) => {
                let _ = event_tx
                    .send(ClientEvent::Error {
                        network: id,
                        message: format!("TLS handshake failed: {e}"),
                    })
                    .await;
            }
        }
    } else {
        run_session(
            id,
            stream,
            &config.nick,
            &config.user,
            &config.realname,
            &event_tx,
            &mut cmd_rx,
        )
        .await;
    }

    let _ = event_tx
        .send(ClientEvent::Disconnected {
            network: id,
            reason: "connection closed".into(),
        })
        .await;
}

async fn tls_connect(
    host: &str,
    stream: TcpStream,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, std::io::Error> {
    let mut root_store = rustls::RootCertStore::empty();
    let _ = &mut root_store;
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(danger::NoCertVerifier))
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let server_name = rustls::pki_types::ServerName::try_from(host.to_owned())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    connector.connect(server_name, stream).await
}

mod danger {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, Error, SignatureScheme};

    /// Certificate verifier that accepts any certificate.
    ///
    /// This is intentionally insecure and suitable only for development/testing.
    #[derive(Debug)]
    pub(super) struct NoCertVerifier;

    impl ServerCertVerifier for NoCertVerifier {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::ECDSA_NISTP384_SHA384,
                SignatureScheme::ECDSA_NISTP521_SHA512,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PSS_SHA512,
                SignatureScheme::ED25519,
                SignatureScheme::ED448,
            ]
        }
    }
}

#[allow(clippy::cognitive_complexity)] // IRC protocol dispatch is inherently branchy
async fn run_session<S>(
    id: NetworkId,
    stream: S,
    nick: &Bytes,
    user: &Bytes,
    realname: &Bytes,
    event_tx: &mpsc::Sender<ClientEvent>,
    cmd_rx: &mut mpsc::Receiver<ClientCommand>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut framed = Framed::new(stream, IrcCodec::new());

    // Send registration.
    let nick_cmd = Command::Nick { nick: nick.clone() };
    let user_cmd = Command::User {
        user: user.clone(),
        mode: Bytes::from_static(b"0"),
        realname: realname.clone(),
    };

    if let Err(e) = framed.send(nick_cmd.to_message()).await {
        error!("failed to send NICK: {e}");
        return;
    }
    if let Err(e) = framed.send(user_cmd.to_message()).await {
        error!("failed to send USER: {e}");
        return;
    }

    let mut state = NetworkState::new();
    state.nick = Some(nick.clone());

    loop {
        tokio::select! {
            frame = framed.next() => {
                match frame {
                    Some(Ok(msg)) => {
                        state.apply(&msg);
                        if let Some(event) = translate_message(id, &msg) {
                            let _ = event_tx.send(event).await;
                        }
                        // Auto-respond to PING.
                        if let Ok(Command::Ping { token, .. }) = Command::parse(&msg) {
                            let pong = Command::Pong { token, server: None };
                            if let Err(e) = framed.send(pong.to_message()).await {
                                warn!("failed to send PONG: {e}");
                                break;
                            }
                        }
                    }
                    Some(Err(e)) => {
                        let _ = event_tx
                            .send(ClientEvent::Error {
                                network: id,
                                message: format!("codec error: {e}"),
                            })
                            .await;
                        break;
                    }
                    None => {
                        break;
                    }
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(command) => {
                        if let Some(wire) = translate_command(&command) {
                            if let Err(e) = framed.send(wire).await {
                                warn!("failed to send command: {e}");
                                break;
                            }
                        }
                        if matches!(command, ClientCommand::Disconnect { .. } | ClientCommand::Quit { .. }) {
                            break;
                        }
                    }
                    None => {
                        break;
                    }
                }
            }
        }
    }
}

/// Translate a wire message into a client event.
fn translate_message(id: NetworkId, msg: &Message) -> Option<ClientEvent> {
    let cmd = Command::parse(msg).ok()?;
    let nick = match &msg.prefix {
        Some(Prefix::User { nick, .. }) => nick.clone(),
        Some(Prefix::Server(name)) => name.clone(),
        None => Bytes::new(),
    };

    match cmd {
        Command::Privmsg { targets, text } => {
            let target = targets.into_iter().next()?;
            Some(ClientEvent::Message {
                network: id,
                target,
                from: nick,
                text,
            })
        }
        Command::Notice { targets, text } => {
            let target = targets.into_iter().next()?;
            Some(ClientEvent::Notice {
                network: id,
                target,
                from: nick,
                text,
            })
        }
        Command::Join { channels, .. } => {
            let channel = channels.into_iter().next()?;
            Some(ClientEvent::Join {
                network: id,
                channel,
                nick,
            })
        }
        Command::Part { channels, reason } => {
            let channel = channels.into_iter().next()?;
            Some(ClientEvent::Part {
                network: id,
                channel,
                nick,
                reason,
            })
        }
        Command::Nick { nick: new_nick } => Some(ClientEvent::NickChange {
            network: id,
            old: nick,
            new_nick,
        }),
        Command::Topic {
            channel,
            topic: Some(topic),
        } => Some(ClientEvent::TopicChange {
            network: id,
            channel,
            topic,
        }),
        Command::Quit { reason } => Some(ClientEvent::Quit {
            network: id,
            nick,
            reason,
        }),
        Command::Numeric { code: 1, params } => {
            let confirmed_nick = params.get(0).cloned().unwrap_or_default();
            Some(ClientEvent::Registered {
                network: id,
                nick: confirmed_nick,
            })
        }
        // RPL_LIST: <client> <channel> <user_count> :<topic>
        Command::Numeric { code: 322, params } => {
            let channel = params.get(1).cloned().unwrap_or_default();
            let user_count = params
                .get(2)
                .and_then(|b| std::str::from_utf8(b).ok())
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            let topic = params.get(3).cloned().unwrap_or_default();
            Some(ClientEvent::ListEntry {
                network: id,
                channel,
                user_count,
                topic,
            })
        }
        // RPL_LISTEND
        Command::Numeric { code: 323, .. } => Some(ClientEvent::ListEnd { network: id }),
        Command::Numeric { code, params } => {
            let p: Vec<Bytes> = params.iter().cloned().collect();
            Some(ClientEvent::Numeric {
                network: id,
                code,
                params: p,
            })
        }
        _ => {
            debug!("unhandled command from network {}: {:?}", id.0, cmd);
            None
        }
    }
}

/// Translate a frontend command into a wire message.
fn translate_command(cmd: &ClientCommand) -> Option<Message> {
    match cmd {
        ClientCommand::SendRaw { line, .. } => Message::parse(line).ok(),
        ClientCommand::SendPrivmsg { target, text, .. } => Some(
            Command::Privmsg {
                targets: vec![target.clone()],
                text: text.clone(),
            }
            .to_message(),
        ),
        ClientCommand::SendNotice { target, text, .. } => Some(
            Command::Notice {
                targets: vec![target.clone()],
                text: text.clone(),
            }
            .to_message(),
        ),
        ClientCommand::Join { channel, .. } => Some(
            Command::Join {
                channels: vec![channel.clone()],
                keys: vec![],
            }
            .to_message(),
        ),
        ClientCommand::Part {
            channel, reason, ..
        } => Some(
            Command::Part {
                channels: vec![channel.clone()],
                reason: reason.clone(),
            }
            .to_message(),
        ),
        ClientCommand::ChangeNick { nick, .. } => {
            Some(Command::Nick { nick: nick.clone() }.to_message())
        }
        ClientCommand::SetTopic { channel, topic, .. } => Some(
            Command::Topic {
                channel: channel.clone(),
                topic: Some(topic.clone()),
            }
            .to_message(),
        ),
        ClientCommand::Quit { reason, .. } => Some(
            Command::Quit {
                reason: reason.clone(),
            }
            .to_message(),
        ),
        ClientCommand::List { .. } => Some(
            Command::Unknown {
                verb: Bytes::from_static(b"LIST"),
                params: Params::new(),
            }
            .to_message(),
        ),
        ClientCommand::Connect { .. }
        | ClientCommand::Disconnect { .. }
        | ClientCommand::DccAcceptChat { .. }
        | ClientCommand::DccAcceptSend { .. } => None,
    }
}
