//! Single-node IRC daemon.
//!
//! The `irc-server` binary is a thin wrapper around the types and
//! entrypoints re-exported from this library: [`Config`], [`Server`],
//! and [`ServerError`]. Integration tests — both in this crate and in
//! `irc-testkit` downstream — drive the server via these types so the
//! test harness and the `main.rs` binary share the same code path.

#![deny(missing_docs)]

pub mod account;
pub mod caps;
pub mod cloak;
pub mod config;
pub mod connection;
pub mod error;
pub mod flood;
pub mod handler;
pub mod limiter;
pub mod metrics;
pub mod numeric;
pub mod oper;
pub mod proxy_proto;
pub mod runtime;
pub mod state;
pub mod store;
pub mod tls;

pub use config::{Config, Limits, ListenerConfig};
pub use error::ServerError;
pub use runtime::{Server, ShutdownHandle};
pub use state::{ServerState, User, UserHandle, UserId};
