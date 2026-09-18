//! `irc-server` daemon entrypoint.
//!
//! Parses CLI arguments, loads the TOML config, configures `tracing`,
//! binds the server, and drives the accept loop until a shutdown
//! signal (SIGINT / SIGTERM) is received.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use irc_server::{Config, Server};
use tokio::signal;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "irc-server", version, about)]
struct Args {
    /// Path to the TOML configuration file.
    #[arg(long, short = 'c', env = "IRC_SERVER_CONFIG")]
    config: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    init_tracing();
    let cfg = Config::from_toml_path(&args.config)
        .with_context(|| format!("loading config from {}", args.config.display()))?;
    info!(server_name = %cfg.server_name, "starting");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let (server, shutdown) = Server::bind(cfg).await.context("binding listeners")?;
        let addrs = server.local_addrs();
        info!(?addrs, "serving");
        let serve_task = tokio::spawn(server.serve());

        wait_for_shutdown().await;
        info!("shutdown signal received");
        shutdown.signal();

        serve_task
            .await
            .context("serve task panicked")?
            .context("serve task returned error")
    })
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}

async fn wait_for_shutdown() {
    let ctrl_c = async {
        let _ = signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        let mut sig = signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("installable SIGTERM handler");
        let _ = sig.recv().await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
    }
}
