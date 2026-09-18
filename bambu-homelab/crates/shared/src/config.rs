use serde::Deserialize;

/// NATS connection configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct NatsConfig {
    /// NATS server URL (e.g. "tls://127.0.0.1:4222").
    pub url: String,
    /// NKey seed for authentication (starts with "SU").
    pub nkey_seed: String,
    /// Path to CA certificate for TLS verification.
    pub ca_cert: String,
}

/// Gateway service configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct GatewayConfig {
    pub nats: NatsConfig,
    /// Address for the health/metrics HTTP listener.
    pub listen_addr: String,
}

/// Mock printer configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct MockPrinterConfig {
    pub nats: NatsConfig,
    /// Printer ID this mock instance pretends to be.
    pub printer_id: String,
    /// Interval in seconds between telemetry publishes.
    #[serde(default = "default_telemetry_interval")]
    pub telemetry_interval_secs: u64,
}

/// Bridge service configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct BridgeConfig {
    pub nats: NatsConfig,
    /// Postgres connection URL.
    pub database_url: String,
}

fn default_telemetry_interval() -> u64 {
    5
}

/// API service configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct ApiConfig {
    pub nats: NatsConfig,
    /// HTTP listen address (e.g. "127.0.0.1:8081").
    pub listen_addr: String,
    /// Postgres connection URL.
    pub database_url: String,
    /// JWT signing secret (min 32 bytes).
    pub jwt_secret: String,
}

/// Load configuration from environment variables.
/// Environment variables are prefixed with the given prefix and use `__` as the
/// nesting separator (e.g. `BAMBU_GATEWAY__NATS__URL`).
pub fn load<T: serde::de::DeserializeOwned>(prefix: &str) -> Result<T, config::ConfigError> {
    config::Config::builder()
        .add_source(
            config::Environment::with_prefix(prefix)
                .separator("__")
                .try_parsing(true),
        )
        .build()?
        .try_deserialize()
}
