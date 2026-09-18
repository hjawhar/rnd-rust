//! Token-bucket rate limiter for post-registration message flow.

use tokio::time::Instant;

/// Token-bucket rate limiter for post-registration message flow.
///
/// Lives inside the per-connection read loop — not `Send`/`Sync`.
/// Tokens refill continuously at `refill_rate` tokens per second,
/// capped at `max_tokens`.
pub struct FloodBucket {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64,
    last_refill: Instant,
}

impl FloodBucket {
    /// Create a new bucket that allows `burst` messages immediately
    /// and refills at `rate_per_sec` tokens per second.
    pub fn new(rate_per_sec: u32, burst: u32) -> Self {
        Self {
            tokens: f64::from(burst),
            max_tokens: f64::from(burst),
            refill_rate: f64::from(rate_per_sec),
            last_refill: Instant::now(),
        }
    }

    /// Try to consume one token. Returns `true` if a token was
    /// available, `false` if the bucket is empty (flood).
    pub fn try_consume(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.last_refill = now;

        self.tokens = elapsed
            .mul_add(self.refill_rate, self.tokens)
            .min(self.max_tokens);

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{self, Duration};

    #[tokio::test]
    async fn burst_then_throttle_then_refill() {
        time::pause();

        let mut bucket = FloodBucket::new(2, 3);

        // Burst: 3 messages pass immediately.
        assert!(bucket.try_consume());
        assert!(bucket.try_consume());
        assert!(bucket.try_consume());

        // 4th message is rejected — bucket is empty.
        assert!(!bucket.try_consume());

        // Advance 1 second → 2 tokens refilled (rate = 2/s).
        time::advance(Duration::from_secs(1)).await;
        assert!(bucket.try_consume());
        assert!(bucket.try_consume());
        assert!(!bucket.try_consume());

        // Advance a long time — tokens must cap at max_tokens (3).
        time::advance(Duration::from_secs(100)).await;
        assert!(bucket.try_consume());
        assert!(bucket.try_consume());
        assert!(bucket.try_consume());
        assert!(!bucket.try_consume());
    }
}
