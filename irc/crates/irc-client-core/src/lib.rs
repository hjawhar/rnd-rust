//! Headless IRC client library: connection management, event/command
//! channels, and client-side state tracking.
//!
//! The primary entry point is [`Client`], which manages multiple network
//! connections and exposes a single event stream to the frontend.

#![deny(missing_docs)]

pub mod client;
pub mod command;
pub mod dcc;
pub mod event;
pub mod network;
pub mod scripting;
pub mod state;

pub use client::Client;
pub use command::ClientCommand;
pub use event::{ClientEvent, NetworkId};
