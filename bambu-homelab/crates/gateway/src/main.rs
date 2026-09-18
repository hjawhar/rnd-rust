mod telemetry;

use anyhow::Context;
use tracing::info;

use bambu_shared::config::{self, GatewayConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,bambu_gateway=debug".parse().unwrap()),
        )
        .with_target(true)
        .init();

    info!("bambu-gateway starting");

    let cfg: GatewayConfig = config::load("BAMBU_GATEWAY")
        .context("failed to load gateway configuration")?;

    info!(nats_url = %cfg.nats.url, "connecting to NATS");

    let ca_cert = std::path::Path::new(&cfg.nats.ca_cert);
    let client = async_nats::ConnectOptions::new()
        .nkey(cfg.nats.nkey_seed.clone())
        .add_root_certificates(ca_cert.into())
        .require_tls(true)
        .connect(&cfg.nats.url)
        .await
        .context("failed to connect to NATS")?;

    info!("connected to NATS");

    let js = async_nats::jetstream::new(client);

    // Run telemetry subscriber until shutdown signal.
    let shutdown = tokio::signal::ctrl_c();
    tokio::select! {
        result = telemetry::subscribe(&js) => result,
        _ = shutdown => {
            info!("shutting down");
            Ok(())
        }
    }
}
