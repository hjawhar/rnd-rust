//! Persistence abstraction.
//!
//! Production wires this to a SQLite-backed implementation in `irc-server`
//! and `irc-bnc`. Tests use [`InMemoryStore`] (or capture proxies built on
//! top of it) so they neither touch disk nor depend on test ordering.
//!
//! The trait is intentionally minimal in Phase 0. Methods grow per phase
//! as features land (accounts in Phase 3, k-lines in Phase 4, chathistory
//! in Phase 5, BNC buffers in Phase 11).

use std::error::Error;

/// Errors returned by [`Store`] operations.
pub type StoreError = Box<dyn Error + Send + Sync>;

/// Persistence contract.
pub trait Store: Send + Sync + 'static {
    /// Health check; returns `Ok(())` when the backend is reachable.
    fn ping(&self) -> Result<(), StoreError>;
}

/// In-memory placeholder store.
#[derive(Debug, Default, Clone, Copy)]
pub struct InMemoryStore;

impl Store for InMemoryStore {
    fn ping(&self) -> Result<(), StoreError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{InMemoryStore, Store};

    #[test]
    fn in_memory_store_pings_ok() {
        InMemoryStore.ping().unwrap();
    }
}
