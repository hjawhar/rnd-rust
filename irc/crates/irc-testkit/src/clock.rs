//! Time abstraction.
//!
//! Production wires this to [`SystemTime::now`]. Tests use a controllable
//! implementation so token expiry, ban expiry, flood buckets, and
//! chathistory ranges are deterministic.

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// A source of wall-clock time.
pub trait Clock: Send + Sync + 'static {
    /// Current wall-clock time.
    fn now(&self) -> SystemTime;
}

/// Production clock backed by [`SystemTime::now`].
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// Test clock that only advances when the test asks it to.
///
/// Construct with [`ManualClock::new`], read with [`Clock::now`], and step
/// forward with [`ManualClock::advance`]. Cloning gives every consumer the
/// same view — internal state is shared via [`Mutex`].
#[derive(Debug)]
pub struct ManualClock {
    inner: Mutex<SystemTime>,
}

impl Default for ManualClock {
    /// Start at the UNIX epoch — the simplest deterministic anchor for
    /// tests that don't care about absolute time.
    fn default() -> Self {
        Self::new(SystemTime::UNIX_EPOCH)
    }
}

impl ManualClock {
    /// Construct a `ManualClock` initialized to `start`.
    #[must_use]
    pub const fn new(start: SystemTime) -> Self {
        Self {
            inner: Mutex::new(start),
        }
    }

    /// Advance the clock by `delta`.
    ///
    /// # Panics
    /// Panics if the internal lock is poisoned.
    pub fn advance(&self, delta: Duration) {
        let mut guard = self.inner.lock().expect("ManualClock mutex poisoned");
        *guard += delta;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> SystemTime {
        *self.inner.lock().expect("ManualClock mutex poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, ManualClock, SystemClock};
    use std::time::{Duration, SystemTime};

    #[test]
    fn system_clock_returns_increasing_times() {
        let c = SystemClock;
        let t1 = c.now();
        let t2 = c.now();
        assert!(t2 >= t1);
    }

    #[test]
    fn manual_clock_only_advances_on_request() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let c = ManualClock::new(start);
        assert_eq!(c.now(), start);
        c.advance(Duration::from_secs(60));
        assert_eq!(c.now(), start + Duration::from_secs(60));
    }
}
