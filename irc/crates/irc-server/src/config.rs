//! Server configuration.
//!
//! Config is TOML-driven with sane defaults for every knob. Tests
//! construct configs programmatically via [`Config::builder`]; the
//! `main.rs` binary loads a config file via [`Config::from_toml_path`].

use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::ConfigError;
use crate::oper::{OperBlock, OperClass};

/// Top-level server configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// This server's public name (`irc.example.net`).
    pub server_name: String,
    /// Network-wide name advertised in ISUPPORT `NETWORK=` and 002/003.
    #[serde(default = "default_network_name")]
    pub network_name: String,
    /// Inline MOTD text. Mutually exclusive with `motd_path`.
    #[serde(default)]
    pub motd: Option<String>,
    /// Path to a MOTD file on disk. Mutually exclusive with `motd`.
    #[serde(default)]
    pub motd_path: Option<PathBuf>,
    /// Bind addresses + TLS + PROXY-protocol toggles.
    #[serde(default = "default_listeners")]
    pub listeners: Vec<ListenerConfig>,
    /// HMAC secret used for cloaking hosts. Inline form.
    #[serde(default)]
    pub cloak_secret: Option<String>,
    /// HMAC secret file. Mutually exclusive with `cloak_secret`.
    #[serde(default)]
    pub cloak_secret_file: Option<PathBuf>,
    /// Abuse-control limits.
    #[serde(default)]
    pub limits: Limits,
    /// Storage backend configuration.
    #[serde(default)]
    pub storage: StorageConfig,
    /// Server operator blocks.
    #[serde(default, rename = "oper")]
    pub opers: Vec<OperBlock>,
    /// Oper class definitions.
    #[serde(default, rename = "oper_class")]
    pub oper_classes: HashMap<String, OperClass>,
    /// Prometheus metrics configuration.
    #[serde(default)]
    pub metrics: MetricsConfig,
}

/// Storage configuration.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    /// Path to the SQLite database file. When absent, an in-memory
    /// store is used.
    pub sqlite_path: Option<String>,
}

/// Prometheus metrics exporter configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsConfig {
    /// Socket address the metrics HTTP exporter binds to.
    #[serde(default = "default_metrics_bind")]
    pub bind: SocketAddr,
    /// Whether the metrics exporter is enabled.
    #[serde(default)]
    pub enabled: bool,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            bind: default_metrics_bind(),
            enabled: false,
        }
    }
}

/// Listener configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerConfig {
    /// Bind address (IPv4 or IPv6, port may be 0 for ephemeral).
    pub bind: SocketAddr,
    /// Whether to accept TLS client-hellos.
    #[serde(default)]
    pub tls: bool,
    /// TLS certificate chain (PEM) — required when `tls=true`.
    #[serde(default)]
    pub cert: Option<PathBuf>,
    /// TLS private key (PEM) — required when `tls=true`.
    #[serde(default)]
    pub key: Option<PathBuf>,
    /// Expect PROXY protocol v2 headers on accept.
    #[serde(default)]
    pub proxy_protocol: bool,
}

/// Abuse-control limits.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Maximum concurrent connections per source IP.
    #[serde(default = "default_per_ip_max")]
    pub per_ip_max_connections: u32,
    /// Accept-time connection attempts per minute per source IP.
    #[serde(default = "default_per_ip_rate")]
    pub per_ip_connect_rate_per_minute: u32,
    /// Seconds a connection has to complete NICK + USER before being dropped.
    #[serde(default = "default_registration_deadline")]
    pub registration_deadline_seconds: u64,
    /// Token-bucket refill rate for post-registration messages.
    #[serde(default = "default_messages_per_second")]
    pub messages_per_second: u32,
    /// Initial burst size for the message token bucket.
    #[serde(default = "default_messages_burst")]
    pub messages_burst: u32,
    /// Hard ceiling on total concurrent connections.
    #[serde(default = "default_global_max")]
    pub global_max_connections: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            per_ip_max_connections: default_per_ip_max(),
            per_ip_connect_rate_per_minute: default_per_ip_rate(),
            registration_deadline_seconds: default_registration_deadline(),
            messages_per_second: default_messages_per_second(),
            messages_burst: default_messages_burst(),
            global_max_connections: default_global_max(),
        }
    }
}

fn default_network_name() -> String {
    "IRC".into()
}

fn default_listeners() -> Vec<ListenerConfig> {
    vec![ListenerConfig {
        bind: "127.0.0.1:6667".parse().expect("valid default addr"),
        tls: false,
        cert: None,
        key: None,
        proxy_protocol: false,
    }]
}

const fn default_per_ip_max() -> u32 {
    5
}
const fn default_per_ip_rate() -> u32 {
    3
}
const fn default_registration_deadline() -> u64 {
    10
}
const fn default_messages_per_second() -> u32 {
    2
}
const fn default_messages_burst() -> u32 {
    6
}
const fn default_global_max() -> u32 {
    10_000
}

fn default_metrics_bind() -> SocketAddr {
    "127.0.0.1:9772"
        .parse()
        .expect("valid default metrics addr")
}

impl Config {
    /// Construct a minimal config suitable for tests.
    ///
    /// Binds a single plain listener to `127.0.0.1:0` so tests can
    /// discover the actual port via [`crate::Server::local_addrs`].
    #[must_use]
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }

    /// Load a [`Config`] from a TOML file on disk.
    pub fn from_toml_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|e| ConfigError::Read {
            path: path.to_path_buf(),
            source: e,
        })?;
        let cfg: Self = toml::from_str(&text).map_err(|e| ConfigError::Parse {
            path: path.to_path_buf(),
            source: e,
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Parse a config from a TOML string. Mostly useful in tests.
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let cfg: Self = toml::from_str(text).map_err(|e| ConfigError::Parse {
            path: PathBuf::from("<memory>"),
            source: e,
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Runtime validation beyond what `serde` catches syntactically.
    fn validate(&self) -> Result<(), ConfigError> {
        if self.listeners.is_empty() {
            return Err(ConfigError::Invalid(
                "at least one listener is required".into(),
            ));
        }
        if self.motd.is_some() && self.motd_path.is_some() {
            return Err(ConfigError::Invalid(
                "set either `motd` or `motd_path`, not both".into(),
            ));
        }
        if self.cloak_secret.is_some() && self.cloak_secret_file.is_some() {
            return Err(ConfigError::Invalid(
                "set either `cloak_secret` or `cloak_secret_file`, not both".into(),
            ));
        }
        for listener in &self.listeners {
            if listener.tls && (listener.cert.is_none() || listener.key.is_none()) {
                return Err(ConfigError::Invalid(format!(
                    "TLS listener on {} requires both cert and key",
                    listener.bind
                )));
            }
        }
        Ok(())
    }
}

/// Fluent builder for [`Config`] used by tests and programmatic
/// launches. Not `Deserialize` — the file path is the declarative
/// surface; this is the imperative one.
#[derive(Debug, Clone)]
pub struct ConfigBuilder {
    inner: Config,
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self {
            inner: Config {
                server_name: "irc.local".into(),
                network_name: "Local".into(),
                motd: Some("Welcome to the test network.".into()),
                motd_path: None,
                listeners: vec![ListenerConfig {
                    bind: "127.0.0.1:0".parse().expect("valid test addr"),
                    tls: false,
                    cert: None,
                    key: None,
                    proxy_protocol: false,
                }],
                cloak_secret: Some(
                    "test-cloak-secret-32bytes-long-xxxx"
                        .chars()
                        .take(32)
                        .collect::<String>(),
                ),
                cloak_secret_file: None,
                limits: Limits::default(),
                storage: StorageConfig::default(),
                opers: Vec::new(),
                oper_classes: HashMap::new(),
                metrics: MetricsConfig::default(),
            },
        }
    }
}

impl ConfigBuilder {
    /// Override the server name.
    #[must_use]
    pub fn server_name(mut self, name: impl Into<String>) -> Self {
        self.inner.server_name = name.into();
        self
    }

    /// Replace the listener set.
    #[must_use]
    pub fn listeners(mut self, listeners: Vec<ListenerConfig>) -> Self {
        self.inner.listeners = listeners;
        self
    }

    /// Override limit defaults.
    #[must_use]
    pub fn limits(mut self, limits: Limits) -> Self {
        self.inner.limits = limits;
        self
    }

    /// Materialise the [`Config`].
    pub fn build(self) -> Result<Config, ConfigError> {
        self.inner.validate()?;
        Ok(self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, ConfigError};

    #[test]
    fn minimal_toml_parses() {
        let cfg = Config::from_toml_str(
            r#"
            server_name = "irc.example.net"

            [[listeners]]
            bind = "0.0.0.0:6667"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.server_name, "irc.example.net");
        assert_eq!(cfg.listeners.len(), 1);
        assert_eq!(cfg.limits.per_ip_max_connections, 5);
    }

    #[test]
    fn rejects_no_listeners() {
        let err = Config::from_toml_str(
            r#"
            server_name = "irc"
            listeners = []
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn rejects_conflicting_motd_sources() {
        let err = Config::from_toml_str(
            r#"
            server_name = "irc"
            motd = "hello"
            motd_path = "/var/irc/motd.txt"

            [[listeners]]
            bind = "127.0.0.1:6667"
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn rejects_tls_without_cert_or_key() {
        let err = Config::from_toml_str(
            r#"
            server_name = "irc"

            [[listeners]]
            bind = "127.0.0.1:6697"
            tls = true
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Invalid(_)));
    }

    #[test]
    fn unknown_field_fails() {
        let err = Config::from_toml_str(
            r#"
            server_name = "irc"
            oopsie = 1

            [[listeners]]
            bind = "127.0.0.1:6667"
            "#,
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }));
    }

    #[test]
    fn builder_produces_valid_default_config() {
        let cfg = Config::builder().build().unwrap();
        assert_eq!(cfg.listeners[0].bind.port(), 0, "ephemeral port for tests");
    }
}
