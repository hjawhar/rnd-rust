//! Channel state.
//!
//! A `Channel` is a hub for a set of member users plus a small bag of
//! metadata (name, topic, modes). Membership mutation goes through
//! [`Channel::add_member`] / [`Channel::remove_member`] — callers must
//! hold the `RwLock<Channel>` for writing to call them.

use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;

use crate::state::UserId;

/// Per-member metadata within a channel.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MemberMode {
    /// `+o` (channel operator) — prefixes nicks with `@`.
    pub op: bool,
    /// `+v` (voice) — prefixes nicks with `+`.
    pub voice: bool,
}

impl MemberMode {
    /// Best-effort prefix byte for this member (op wins over voice).
    #[must_use]
    pub const fn prefix_byte(self) -> Option<u8> {
        if self.op {
            Some(b'@')
        } else if self.voice {
            Some(b'+')
        } else {
            None
        }
    }
}

/// Topic metadata captured when TOPIC is set.
#[derive(Debug, Clone)]
pub struct Topic {
    /// Topic text.
    pub text: Bytes,
    /// Nick of the setter (kept in wire form for TOPIC reply).
    pub setter: Bytes,
    /// Unix-epoch seconds when the topic was set.
    pub set_at: u64,
}

/// A single channel.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // IRC channel modes are a flag bag
pub struct Channel {
    /// Original-case channel name (e.g. `#Rust`).
    pub name: Bytes,
    /// Current topic, if any.
    pub topic: Option<Topic>,
    /// Members plus per-member mode bits. Sorted by [`UserId`] so
    /// iteration order is stable across snapshots.
    pub members: BTreeMap<UserId, MemberMode>,
    /// `+n` — no external messages (default on).
    pub mode_n: bool,
    /// `+t` — topic lock (default on).
    pub mode_t: bool,
    /// `+m` — moderated (default off).
    pub mode_m: bool,
    /// `+i` — invite-only (default off).
    pub mode_i: bool,
    /// `+k` — channel key.
    pub mode_k: Option<Bytes>,
    /// `+l` — member limit.
    pub mode_l: Option<u32>,
}

impl Channel {
    /// Construct an empty channel with `name`.
    #[must_use]
    pub fn new(name: Bytes) -> Self {
        Self {
            name,
            topic: None,
            members: BTreeMap::new(),
            mode_n: true,
            mode_t: true,
            mode_m: false,
            mode_i: false,
            mode_k: None,
            mode_l: None,
        }
    }

    /// Add `user` with default membership mode. Returns `true` when
    /// the user was newly added, `false` if they were already a
    /// member.
    pub fn add_member(&mut self, user: UserId, mode: MemberMode) -> bool {
        use std::collections::btree_map::Entry;
        match self.members.entry(user) {
            Entry::Occupied(_) => false,
            Entry::Vacant(slot) => {
                slot.insert(mode);
                true
            }
        }
    }

    /// Remove `user` from the channel. Returns the per-member mode
    /// they held, if any.
    pub fn remove_member(&mut self, user: UserId) -> Option<MemberMode> {
        self.members.remove(&user)
    }

    /// Is `user` a member?
    #[must_use]
    pub fn has_member(&self, user: UserId) -> bool {
        self.members.contains_key(&user)
    }

    /// Snapshot of the member list in iteration order.
    #[must_use]
    pub fn member_ids(&self) -> Vec<UserId> {
        self.members.keys().copied().collect()
    }

    /// Is any op set? Useful for first-joiner auto-op.
    #[must_use]
    pub fn has_op(&self) -> bool {
        self.members.values().any(|m| m.op)
    }

    /// Gather all nicks as a sorted set. Used to assemble
    /// `RPL_NAMREPLY`.
    #[must_use]
    pub fn nick_set(&self) -> BTreeSet<UserId> {
        self.members.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Channel, MemberMode};
    use crate::state::UserId;
    use bytes::Bytes;

    #[test]
    fn add_and_remove_members() {
        let mut c = Channel::new(Bytes::from_static(b"#rust"));
        let a = UserId::new(1);
        let b = UserId::new(2);
        assert!(c.add_member(
            a,
            MemberMode {
                op: true,
                voice: false
            }
        ));
        assert!(c.add_member(b, MemberMode::default()));
        assert!(
            !c.add_member(a, MemberMode::default()),
            "adding twice is a no-op"
        );
        assert!(c.has_member(a));
        assert!(c.has_op());

        let removed = c.remove_member(a).unwrap();
        assert!(removed.op);
        assert!(!c.has_op());
    }

    #[test]
    fn prefix_byte_priority() {
        assert_eq!(
            MemberMode {
                op: true,
                voice: false
            }
            .prefix_byte(),
            Some(b'@')
        );
        assert_eq!(
            MemberMode {
                op: false,
                voice: true
            }
            .prefix_byte(),
            Some(b'+')
        );
        assert_eq!(
            MemberMode {
                op: true,
                voice: true
            }
            .prefix_byte(),
            Some(b'@')
        );
        assert_eq!(MemberMode::default().prefix_byte(), None);
    }
}
