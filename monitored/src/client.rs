use core::str;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use log::info;
use russh::keys::*;
use russh::*;
use std::error::Error;
use tokio::net::ToSocketAddrs;

pub struct Client {}

#[async_trait]
impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

pub struct Session {
    session: client::Handle<Client>,
}

impl Session {
    pub async fn connect<P: AsRef<Path>, A: ToSocketAddrs>(
        key_path: P,
        user: impl Into<String>,
        addrs: A,
    ) -> Result<Self, Box<dyn Error>> {
        let key_pair = load_secret_key(key_path, None)?;
        let config = client::Config {
            inactivity_timeout: Some(Duration::from_secs(5)),
            ..<_>::default()
        };

        let config = Arc::new(config);
        let sh = Client {};

        let mut session = client::connect(config, addrs, sh).await?;
        let auth_res = session
            .authenticate_publickey(user, Arc::new(key_pair))
            .await?;

        if !auth_res {
            panic!("Authentication failed");
        }

        Ok(Self { session })
    }

    pub async fn call(&mut self, command: &str) -> Result<String, Box<dyn Error>> {
        let mut channel = self.session.channel_open_session().await?;
        channel.exec(true, command).await?;

        let mut response: Vec<CryptoVec> = vec![];
        // let mut stdout = tokio::io::stdout();

        loop {
            // There's an event available on the session channel
            let Some(msg) = channel.wait().await else {
                break;
            };
            match msg {
                ChannelMsg::Data { ref data } => {
                    // stdout.write_all(data).await?;
                    // stdout.flush().await?;

                    response.push(data.clone());
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    info!("Exited with code {exit_status}");
                }
                _ => {}
            }
        }

        let b: Vec<_> = response
            .iter()
            .map(|x| x.bytes().map(|x| x.unwrap()))
            .flatten()
            .collect();
        let appended = str::from_utf8(&b).unwrap().to_string();
        Ok(appended)
    }

    pub async fn close(&mut self) -> Result<(), Box<dyn Error>> {
        self.session
            .disconnect(Disconnect::ByApplication, "", "English")
            .await?;
        Ok(())
    }
}
