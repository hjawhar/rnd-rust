//! Shared runtime state.
//!
//! The registries (users, nicks, channels) live behind their own
//! concurrency primitives — `DashMap` for outer indexing plus a per-
//! channel `RwLock` — so no mutation ever takes a server-wide lock.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use dashmap::DashMap;
use irc_proto::Casemap;

use crate::cloak::CloakEngine;
use crate::config::Config;
use crate::oper::glob_match;
use crate::store::AnyStore;

pub mod channel;
pub mod user;

pub use channel::{Channel, MemberMode, Topic};
pub use user::{User, UserHandle, UserId, UserRegInfo};

/// Errors returned when mutating the [`ServerState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    /// A nickname collision: the requested nick is already owned by a
    /// different [`UserId`].
    NickInUse,
    /// The caller referenced a [`UserId`] that is no longer present.
    UnknownUser,
}

/// A server-wide ban entry.
#[derive(Debug, Clone)]
pub struct Kline {
    /// Host mask pattern.
    pub mask: String,
    /// Reason shown to banned users.
    pub reason: String,
    /// Nick or identifier of the oper who set this.
    pub set_by: String,
    /// Expiry as seconds since UNIX epoch; `None` for permanent.
    pub expires: Option<u64>,
}

/// Shared server-wide state. Cloned via `Arc` into every connection
/// task.
#[derive(Debug)]
pub struct ServerState {
    config: Arc<Config>,
    cloak: CloakEngine,
    store: Arc<AnyStore>,
    users: DashMap<UserId, Arc<User>>,
    /// Casemap-folded nickname → owning [`UserId`].
    nicks: DashMap<Bytes, UserId>,
    /// Casemap-folded channel name → [`Channel`] state.
    channels: DashMap<Bytes, Arc<parking_lot::RwLock<Channel>>>,
    casemap: Casemap,
    next_user_id: AtomicU64,
    /// Active K-line bans.
    klines: parking_lot::RwLock<Vec<Kline>>,
}

impl ServerState {
    /// Construct a fresh state snapshot from a validated [`Config`].
    #[must_use]
    pub fn new(config: Arc<Config>, store: Arc<AnyStore>) -> Self {
        let cloak_secret = config.cloak_secret.as_deref().map_or_else(
            || {
                use rand::Rng;
                tracing::warn!("no cloak_secret configured; generating a random one");
                let mut buf = [0u8; 32];
                rand::rng().fill(&mut buf);
                buf.to_vec()
            },
            |s| s.as_bytes().to_vec(),
        );
        Self {
            config,
            cloak: CloakEngine::new(&cloak_secret),
            store,
            users: DashMap::new(),
            nicks: DashMap::new(),
            channels: DashMap::new(),
            casemap: Casemap::Rfc1459,
            next_user_id: AtomicU64::new(1),
            klines: parking_lot::RwLock::new(Vec::new()),
        }
    }

    /// Access the active configuration.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Access the cloaking engine.
    #[must_use]
    pub fn cloak(&self) -> &CloakEngine {
        &self.cloak
    }

    /// Access the backing store.
    #[must_use]
    pub fn store(&self) -> &AnyStore {
        &self.store
    }
    /// Active casemap for nickname and channel comparisons.
    #[must_use]
    pub const fn casemap(&self) -> Casemap {
        self.casemap
    }

    /// Mint the next [`UserId`]. Zero is reserved and never issued.
    pub fn next_user_id(&self) -> UserId {
        UserId::new(self.next_user_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Insert a freshly-accepted (pre-registration) user.
    pub fn insert_user(&self, user: Arc<User>) {
        self.users.insert(user.id(), user);
    }

    /// Remove a user and release any nickname it held. Used when a
    /// connection closes. Idempotent.
    pub fn remove_user(&self, id: UserId) -> Option<Arc<User>> {
        let user = self.users.remove(&id).map(|(_, v)| v)?;
        if let Some(nick) = user.snapshot().nick {
            let folded = self.casemap.fold(nick.as_ref());
            // Only evict the nick entry if it still points at this user
            // (a NICK change could have already claimed the slot).
            if let Some(entry) = self.nicks.get_mut(&folded) {
                if *entry.value() == id {
                    drop(entry);
                    self.nicks.remove(&folded);
                }
            }
        }
        Some(user)
    }

    /// Look up a live [`User`] by id.
    #[must_use]
    pub fn user(&self, id: UserId) -> Option<Arc<User>> {
        self.users.get(&id).map(|r| r.value().clone())
    }

    /// Look up a live [`User`] by nickname using the active casemap.
    #[must_use]
    pub fn user_by_nick(&self, nick: &[u8]) -> Option<Arc<User>> {
        let folded = self.casemap.fold(nick);
        let id = *self.nicks.get(&folded)?.value();
        self.user(id)
    }

    /// Reserve `nick` for `user_id`. Fails if the nickname is already
    /// owned by a different user. Releases any prior nickname the user
    /// held.
    pub fn claim_nick(&self, user_id: UserId, nick: &[u8]) -> Result<(), StateError> {
        let folded = self.casemap.fold(nick);
        // If the slot exists and belongs to somebody else, refuse.
        if let Some(entry) = self.nicks.get(&folded) {
            if *entry.value() != user_id {
                return Err(StateError::NickInUse);
            }
            // Same user re-claiming its own nick — no-op.
            return Ok(());
        }
        // Release the prior nickname, if any.
        let prior = self
            .user(user_id)
            .ok_or(StateError::UnknownUser)?
            .snapshot()
            .nick;
        if let Some(old) = prior {
            let old_folded = self.casemap.fold(old.as_ref());
            if let Some(entry) = self.nicks.get(&old_folded) {
                if *entry.value() == user_id {
                    drop(entry);
                    self.nicks.remove(&old_folded);
                }
            }
        }
        self.nicks.insert(folded, user_id);
        Ok(())
    }

    /// Count currently-registered users. O(n); call sparingly.
    pub fn registered_count(&self) -> usize {
        self.users
            .iter()
            .filter(|e| e.value().is_registered())
            .count()
    }

    /// Iterate live [`UserHandle`]s. Cheap: each handle is a clone of
    /// an `Arc<User>`.
    pub fn users(&self) -> Vec<UserHandle> {
        self.users
            .iter()
            .map(|e| UserHandle(e.value().clone()))
            .collect()
    }

    // --- channel operations ---

    /// Look up a channel by name (case-folded).
    #[must_use]
    pub fn channel(&self, name: &[u8]) -> Option<Arc<parking_lot::RwLock<Channel>>> {
        let folded = self.casemap.fold(name);
        self.channels.get(&folded).map(|r| r.value().clone())
    }

    /// Get the channel, or create it with `name` as the original-case
    /// canonical name if absent.
    pub fn channel_or_create(&self, name: &[u8]) -> Arc<parking_lot::RwLock<Channel>> {
        let folded = self.casemap.fold(name);
        self.channels
            .entry(folded)
            .or_insert_with(|| {
                Arc::new(parking_lot::RwLock::new(Channel::new(
                    Bytes::copy_from_slice(name),
                )))
            })
            .value()
            .clone()
    }

    /// Remove a channel once it has no members left.
    pub fn remove_empty_channel(&self, name: &[u8]) {
        let folded = self.casemap.fold(name);
        if let Some(entry) = self.channels.get(&folded) {
            if !entry.value().read().members.is_empty() {
                return;
            }
            drop(entry);
            self.channels.remove(&folded);
        }
    }

    /// Every user ID that shares at least one channel with `user_id`
    /// (excluding `user_id` itself). Used for QUIT broadcasts.
    #[must_use]
    pub fn channel_peers(&self, user_id: UserId) -> Vec<UserId> {
        let mut peers = std::collections::BTreeSet::new();
        for entry in &self.channels {
            let guard = entry.value().read();
            if guard.has_member(user_id) {
                for uid in guard.members.keys() {
                    if *uid != user_id {
                        peers.insert(*uid);
                    }
                }
            }
        }
        peers.into_iter().collect()
    }

    /// Walk every channel the user is a member of, removing them.
    /// Returns the list of channel names (original case) they were
    /// in.
    pub fn purge_user_from_channels(&self, user_id: UserId) -> Vec<Bytes> {
        let mut emptied: Vec<Bytes> = Vec::new();
        let mut left: Vec<Bytes> = Vec::new();
        for entry in &self.channels {
            let chan = entry.value();
            let mut guard = chan.write();
            if guard.remove_member(user_id).is_some() {
                left.push(guard.name.clone());
                if guard.members.is_empty() {
                    emptied.push(guard.name.clone());
                }
            }
        }
        for name in &emptied {
            self.remove_empty_channel(name.as_ref());
        }
        left
    }

    // --- kline operations ---

    /// Add a K-line ban.
    pub fn add_kline(&self, kl: Kline) {
        self.klines.write().push(kl);
    }

    /// Remove the first K-line whose mask matches exactly.
    pub fn remove_kline(&self, mask: &str) -> bool {
        let mut klines = self.klines.write();
        let before = klines.len();
        klines.retain(|k| k.mask != mask);
        klines.len() < before
    }

    #[allow(clippy::significant_drop_tightening)] // lock held for iteration
    /// Check if a host is banned. Expired entries are pruned lazily.
    pub fn is_klined(&self, host: &str) -> Option<Kline> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let klines = self.klines.read();
        for kl in klines.iter() {
            if let Some(exp) = kl.expires {
                if exp <= now {
                    continue;
                }
            }
            // Match the mask against *!*@host
            let full = format!("*!*@{host}");
            // The mask may be host-only or full user@host
            let mask_host = if let Some((_prefix, h)) = kl.mask.rsplit_once('@') {
                h
            } else {
                &kl.mask
            };
            if glob_match(mask_host, host) || glob_match(&kl.mask, &full) {
                return Some(kl.clone());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{ServerState, StateError, User, UserId};
    use crate::config::Config;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn mk_user(id: UserId) -> Arc<User> {
        let (tx, _rx) = mpsc::channel(32);
        User::new(id, SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1), tx).into()
    }

    fn mk_state() -> ServerState {
        ServerState::new(
            Arc::new(Config::builder().build().unwrap()),
            Arc::new(crate::store::AnyStore::InMemory(
                crate::store::InMemoryStore::new(),
            )),
        )
    }

    #[test]
    fn user_ids_are_monotonically_unique() {
        let state = mk_state();
        let a = state.next_user_id();
        let b = state.next_user_id();
        assert_ne!(a, b);
        assert_eq!(a.get() + 1, b.get());
    }

    #[test]
    fn claim_then_release_nick() {
        let state = mk_state();
        let id = state.next_user_id();
        state.insert_user(mk_user(id));
        state.claim_nick(id, b"alice").unwrap();
        assert!(state.user_by_nick(b"ALICE").is_some(), "casemap-aware");
        state.remove_user(id);
        assert!(state.user_by_nick(b"alice").is_none());
    }

    #[test]
    fn nick_collision_rejected() {
        let state = mk_state();
        let a = state.next_user_id();
        let b = state.next_user_id();
        state.insert_user(mk_user(a));
        state.insert_user(mk_user(b));
        state.claim_nick(a, b"alice").unwrap();
        assert_eq!(state.claim_nick(b, b"Alice"), Err(StateError::NickInUse));
    }

    #[test]
    fn reclaiming_own_nick_is_noop() {
        let state = mk_state();
        let id = state.next_user_id();
        state.insert_user(mk_user(id));
        state.claim_nick(id, b"alice").unwrap();
        state.claim_nick(id, b"alice").unwrap();
    }

    #[test]
    fn nick_change_releases_prior_slot() {
        let state = mk_state();
        let id = state.next_user_id();
        let user = mk_user(id);
        state.insert_user(user.clone());
        state.claim_nick(id, b"alice").unwrap();
        user.set_nick(bytes::Bytes::from_static(b"alice"));
        state.claim_nick(id, b"alice2").unwrap();
        user.set_nick(bytes::Bytes::from_static(b"alice2"));
        // Another user can now take "alice".
        let other = state.next_user_id();
        state.insert_user(mk_user(other));
        state.claim_nick(other, b"alice").unwrap();
    }

    #[test]
    fn kline_add_match_remove() {
        let state = mk_state();
        state.add_kline(super::Kline {
            mask: "192.168.*".into(),
            reason: "test ban".into(),
            set_by: "admin".into(),
            expires: None,
        });
        assert!(state.is_klined("192.168.1.1").is_some());
        assert!(state.is_klined("10.0.0.1").is_none());
        // Remove and verify.
        assert!(state.remove_kline("192.168.*"));
        assert!(state.is_klined("192.168.1.1").is_none());
    }

    #[test]
    fn kline_expired_not_matched() {
        let state = mk_state();
        state.add_kline(super::Kline {
            mask: "10.0.*".into(),
            reason: "expired".into(),
            set_by: "admin".into(),
            expires: Some(1), // epoch 1 — already expired
        });
        assert!(state.is_klined("10.0.0.1").is_none());
    }
}
