use clap::Parser;

use client::Session;
use log::info;
use models::{Docker, Pm2};
use std::error::Error;
use table::{display_docker, display_pm2};

pub mod client;
pub mod models;
pub mod table;

#[derive(Parser, Debug)]
struct Args {
    host: String,
    username: String,
    port: u16,
    pk_path: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    let pk_path = args.pk_path;
    let host = args.host;
    let username = args.username;
    let port = args.port;

    info!("Connecting to {}:{}", host, port);
    info!("Key path: {:?}", pk_path);

    let mut ssh = Session::connect(pk_path, username, (host, port)).await?;
    info!("Connected");

    let _install_jq = ssh
        .call(
            r##"if ! command -v jq 2>&1 >/dev/null
    then
        apt install jq -y
    fi"##,
        )
        .await?;

    let _install_nvm = ssh
        .call(
            r##"if ! command -v nvm 2>&1 >/dev/null
    then
        curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash -y \
        export NVM_DIR="$([ -z "${XDG_CONFIG_HOME-}" ] && printf %s "${HOME}/.nvm" || printf %s "${XDG_CONFIG_HOME}/nvm")"
[ -s "$NVM_DIR/nvm.sh" ] && \. "$NVM_DIR/nvm.sh" # This loads nvm
    fi"##,
        )
        .await?;

    let pm2_command = ssh.call("pm2 jlist").await?;
    let pm2_response: Vec<Pm2> = serde_json::from_str::<Vec<Pm2>>(&pm2_command).unwrap_or(vec![]);
    display_pm2(&pm2_response);

    let docker_command = ssh
        .call(r##"docker ps --all --no-trunc --format="{{json . }}" | jq --tab . -s"##)
        .await?;
    let docker_response: Vec<Docker> = serde_json::from_str::<Vec<Docker>>(&docker_command)?;
    display_docker(&docker_response);

    ssh.close().await?;
    Ok(())
}
