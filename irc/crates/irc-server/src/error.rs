//! Error types used by the server runtime.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Top-level error returned by the server runtime.
#[derive(Debug, Error)]
pub enum ServerError {
    /// Configuration failed to load or validate.
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    /// A listener socket failed to bind or accept.
    #[error("listener {addr}: {source}")]
    Listener {
        /// Intended bind address.
        addr: String,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// Generic I/O failure not otherwise categorised.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// Protocol-level codec failure surfaced out to the runtime.
    #[error(transparent)]
    Codec(#[from] irc_proto::CodecError),
}

/// Errors raised while loading or validating [`crate::Config`].
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Config file could not be read.
    #[error("cannot read config at {path}: {source}")]
    Read {
        /// Path the runtime tried to read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// Config file contained invalid TOML syntax or schema.
    #[error("invalid TOML at {path}: {source}")]
    Parse {
        /// Path the parser was reading.
        path: PathBuf,
        /// Parser diagnostic.
        #[source]
        source: toml::de::Error,
    },
    /// A required field was missing or internally inconsistent.
    #[error("invalid config: {0}")]
    Invalid(String),
}
