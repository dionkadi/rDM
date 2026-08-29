//! Token-bucket rate limiting for download speed caps (global + per-download).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct BucketState {
    tokens: f64,
    last: Instant,
}

/// A single token bucket expressed in bytes/second.
pub struct TokenBucket {
    rate: AtomicU64,
    capacity: f64,
    state: Mutex<BucketState>,
}

impl Default for TokenBucket {
    fn default() -> Self {
        // Unlimited.
        TokenBucket::new(0)
    }
}

impl TokenBucket {
    /// `rate` of 0 means unlimited.
    pub fn new(rate: u64) -> Self {
        let rate = rate.max(0);
        let capacity = Self::capacity_for(rate);
        TokenBucket {
            rate: AtomicU64::new(rate),
            capacity,
            state: Mutex::new(BucketState {
                tokens: capacity,
                last: Instant::now(),
            }),
        }
    }

    /// Update the rate (0 = unlimited).
    pub fn set_rate(&self, rate: u64) {
        self.rate.store(rate, Ordering::Relaxed);
    }

    pub fn rate(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }

    /// Burst capacity: how many bytes may be sent instantly above the steady rate.
    fn capacity_for(rate: u64) -> f64 {
        (rate as f64) * 0.2
    }

    /// Asynchronously consume `n` bytes worth of bandwidth, sleeping as needed.
    pub async fn acquire(&self, n: u64) {
        let rate = self.rate.load(Ordering::Relaxed);
        if rate == 0 {
            return;
        }
        let rate = rate as f64;
        let mut remaining = n as f64;
        loop {
            // Lock scope ends (guard dropped) before any await, so the future stays `Send`.
            let wait = {
                let mut st = self.state.lock().unwrap();
                let now = Instant::now();
                let elapsed = now.duration_since(st.last).as_secs_f64();
                st.tokens = (st.tokens + elapsed * rate).min(self.capacity);
                st.last = now;
                if st.tokens >= remaining {
                    st.tokens -= remaining;
                    return;
                }
                // Spend whatever is available, then wait for the remainder.
                remaining -= st.tokens;
                st.tokens = 0.0;
                remaining.min(self.capacity) / rate
            };
            tokio::time::sleep(Duration::from_secs_f64(wait)).await;
        }
    }
}

/// Combines a global and a per-download bucket; the more restrictive one wins.
#[derive(Default)]
pub struct CombinedLimiter {
    global: TokenBucket,
    per_download: TokenBucket,
}

impl CombinedLimiter {
    pub fn new(global_rate: u64, per_download_rate: u64) -> Self {
        CombinedLimiter {
            global: TokenBucket::new(global_rate),
            per_download: TokenBucket::new(per_download_rate),
        }
    }

    pub fn set_global_rate(&self, rate: u64) {
        self.global.set_rate(rate);
    }

    pub fn set_per_download_rate(&self, rate: u64) {
        self.per_download.set_rate(rate);
    }

    /// The global rate in bytes/sec (0 = unlimited).
    pub fn rate(&self) -> u64 {
        self.global.rate()
    }

    /// Spend from both buckets sequentially (total wait = sum of needed waits).
    pub async fn acquire(&self, n: u64) {
        self.global.acquire(n).await;
        self.per_download.acquire(n).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn unlimited_is_instant() {
        let b = TokenBucket::new(0);
        let start = Instant::now();
        b.acquire(1_000_000).await;
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn caps_roughly() {
        // 100 bytes/sec, spend 100 bytes -> ~0.8-1.0s wait (minus 0.2s burst).
        let b = TokenBucket::new(100);
        let start = Instant::now();
        b.acquire(100).await;
        assert!(start.elapsed() >= Duration::from_millis(600), "elapsed={:?}", start.elapsed());
    }

    #[tokio::test]
    async fn combined_uses_stricter() {
        let c = CombinedLimiter::new(100, 0); // global 100 B/s, per-download unlimited
        let start = Instant::now();
        c.acquire(100).await;
        assert!(start.elapsed() >= Duration::from_millis(600));
    }
}
