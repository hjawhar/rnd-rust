#![allow(unused)]
#![allow(clippy::all)]

use dotenv::dotenv;
use http::start_app_router;
use models::{server::Server, state::AppState};
use std::{
    error::Error,
    net::{IpAddr, Ipv4Addr},
    str::FromStr,
    sync::Arc,
};
use tokio::sync::broadcast;

use db::Database;

pub mod db;
pub mod http;
pub mod models;
pub mod utils;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    dotenv().ok();
    let file_appender = tracing_appender::rolling::daily("./logs/", "logger.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_max_level(tracing::Level::INFO)
        .with_thread_names(true)
        .with_thread_ids(true)
        .with_target(false)
        .init();
    tracing::info!("Starting app...");

    let (tx_broadcast, _) = broadcast::channel::<(String, bool)>(1000);

    let db: Database = Database::new().await;
    let axsys_external_api_key =
        std::env::var("AXSYS_EXTERNAL_API_KEY").expect("axsys API external key is required");
    let axsys_api_key = std::env::var("AXSYS_API_KEY").expect("axsys API key is required");
    let axsys_api_key_normal =
        std::env::var("AXSYS_API_KEY_NORMAL").expect("axsys API key is required");

    let axsys_ws_endpoint =
        std::env::var("AXSYS_WS_ENDPOINT").expect("axsys ws endpoint is required");
    let x_api_key = std::env::var("X_API_KEY").expect("X_API_KEY is required");
    let server_name = std::env::var("SERVER_NAME").expect("Server name is required");
    let rpc_endpoint = std::env::var("RPC_ENDPOINT").expect("RPC endpoint is required");
    let servers = std::env::var("SERVERS").expect("Servers are required");
    let servers: Vec<String> = servers.split(",").map(|x| x.to_string()).collect();
    let servers: Vec<Server> = servers
        .iter()
        .map(|server| {
            let split_server: Vec<String> = server.split("@").map(|x| x.to_string()).collect();
            let id = split_server[1].clone();
            let ip = split_server[0].clone();
            Server { id, ip }
        })
        .collect();

    let block_leaders = std::env::var("BLOCK_LEADERS").expect("Block leaders is required");
    let block_leaders: Vec<String> = block_leaders.split(",").map(|x| x.to_string()).collect();

    let shared_state = Arc::new(AppState {
        db,
        tx_broadcast,
        x_api_key,
        rpc_endpoint,
        axsys_external_api_key,
        axsys_api_key,
        axsys_api_key_normal,
        axsys_ws_endpoint,
        server_name,
        servers,
        block_leaders,
    });

    let db = shared_state.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let _ = db.get_db().run_pending_migrations();
    })
    .await;
    start_app_router(shared_state.clone()).await;
    Ok(())
}

async fn get_ip() -> Result<IpAddr, Box<dyn Error + Send + Sync>> {
    let client = reqwest::Client::builder()
        .local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
        .build()?;

    let response = client.get("https://ifconfig.me/ip").send().await?;
    let public_ip = IpAddr::from_str(&response.text().await?).unwrap();
    tracing::info!("Retrieved public ip: {public_ip:?}");

    Ok(public_ip)
}
