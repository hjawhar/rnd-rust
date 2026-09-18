//! Per-IP connection limiter.
//!
//! Tracks concurrent connection counts per IP address using a lock-free
//! `DashMap<IpAddr, AtomicU32>`. The accept loop calls [`ConnectionLimiter::try_acquire`]
//! before spawning a connection task and [`ConnectionLimiter::release`] when
//! the connection closes.

use std::net::IpAddr;
use std::sync::atomic::{AtomicU32, Ordering};

use dashmap::DashMap;

/// Tracks per-IP concurrent connection counts.
#[derive(Debug, Default)]
pub struct ConnectionLimiter {
    counts: DashMap<IpAddr, AtomicU32>,
}

impl ConnectionLimiter {
    /// Create a new, empty limiter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Try to acquire a slot for `ip`. Returns `true` if the current
    /// count (after increment) is at or below `max`. Returns `false`
    /// and rolls the increment back when the limit would be exceeded.
    pub fn try_acquire(&self, ip: IpAddr, max: u32) -> bool {
        let entry = self.counts.entry(ip).or_insert_with(|| AtomicU32::new(0));
        // Optimistic CAS loop: increment only if we'd stay within max.
        loop {
            let current = entry.load(Ordering::Relaxed);
            if current >= max {
                return false;
            }
            if entry
                .compare_exchange_weak(current, current + 1, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }

    /// Release one slot for `ip`. If the count reaches zero the entry
    /// is removed to avoid unbounded map growth. Safe to call even if
    /// `try_acquire` was never called (or already released) — the count
    /// saturates at zero.
    pub fn release(&self, ip: IpAddr) {
        if let Some(entry) = self.counts.get(&ip) {
            let prev = entry.fetch_sub(1, Ordering::AcqRel);
            if prev <= 1 {
                // Drop the read guard before removing to avoid deadlock.
                drop(entry);
                // Remove if still zero. Another thread may have acquired
                // in the meantime — `remove_if` re-checks.
                self.counts
                    .remove_if(&ip, |_, v| v.load(Ordering::Relaxed) == 0);
            }
        }
        // No entry → nothing to do (idempotent).
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn acquire_up_to_limit_then_reject() {
        let limiter = ConnectionLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);

        // Acquire up to max (3).
        assert!(limiter.try_acquire(ip, 3));
        assert!(limiter.try_acquire(ip, 3));
        assert!(limiter.try_acquire(ip, 3));

        // Fourth must fail.
        assert!(!limiter.try_acquire(ip, 3));

        // Release one, then acquire succeeds again.
        limiter.release(ip);
        assert!(limiter.try_acquire(ip, 3));

        // Still at limit.
        assert!(!limiter.try_acquire(ip, 3));
    }

    #[test]
    fn release_without_acquire_is_idempotent() {
        let limiter = ConnectionLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        // Must not panic.
        limiter.release(ip);
        // Acquiring after spurious release still works.
        assert!(limiter.try_acquire(ip, 1));
    }

    #[test]
    fn ipv6_works() {
        let limiter = ConnectionLimiter::new();
        let ip = IpAddr::V6(Ipv6Addr::LOCALHOST);
        assert!(limiter.try_acquire(ip, 1));
        assert!(!limiter.try_acquire(ip, 1));
        limiter.release(ip);
        assert!(limiter.try_acquire(ip, 1));
    }

    #[test]
    fn different_ips_are_independent() {
        let limiter = ConnectionLimiter::new();
        let a = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        let b = IpAddr::V4(Ipv4Addr::new(5, 6, 7, 8));
        assert!(limiter.try_acquire(a, 1));
        assert!(limiter.try_acquire(b, 1));
        assert!(!limiter.try_acquire(a, 1));
        assert!(!limiter.try_acquire(b, 1));
    }
}
