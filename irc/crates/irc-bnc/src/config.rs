use std::net::SocketAddr;

use serde::Deserialize;

/// Top-level bouncer configuration, typically loaded from a TOML file.
#[derive(Debug, Clone, Deserialize)]
pub struct BncConfig {
    /// Address the bouncer listens on for downstream (client) connections.
    pub listen: SocketAddr,
    /// Configured users and their upstream networks.
    pub users: Vec<BncUser>,
}

/// A single bouncer user with one or more upstream networks.
#[derive(Debug, Clone, Deserialize)]
pub struct BncUser {
    /// Username used for PASS authentication (`user/network:password`).
    pub name: String,
    /// Password (plaintext for now).
    pub password: String,
    /// Upstream IRC networks this user can attach to.
    pub networks: Vec<BncNetwork>,
}

/// Configuration for a single upstream IRC network.
#[derive(Debug, Clone, Deserialize)]
pub struct BncNetwork {
    /// Logical name used in PASS auth and admin commands.
    pub name: String,
    /// Hostname or IP of the upstream IRC server.
    pub host: String,
    /// Port of the upstream IRC server.
    pub port: u16,
    /// Whether to use TLS (not implemented yet — reserved).
    #[serde(default)]
    pub tls: bool,
    /// Nick to register with on the upstream.
    pub nick: String,
    /// USER ident to register with.
    pub user: String,
    /// Real name / GECOS field.
    pub realname: String,
}
