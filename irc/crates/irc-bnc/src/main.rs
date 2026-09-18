//! `irc-bnc` — multi-user IRC bouncer entrypoint.

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use irc_bnc::Bouncer;
use irc_bnc::config::BncConfig;

/// Multi-user IRC bouncer with persistent upstreams and IRCv3 server-time replay.
#[derive(Parser, Debug)]
#[command(name = "irc-bnc", version, about)]
struct Cli {
    /// Path to the TOML configuration file.
    #[arg(short, long, default_value = "bnc.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let config_str = std::fs::read_to_string(&cli.config)
        .with_context(|| format!("failed to read config file: {}", cli.config.display()))?;
    let config: BncConfig =
        toml::from_str(&config_str).context("failed to parse bouncer config")?;

    let bouncer = Bouncer::new(config);
    bouncer.run().await
}
