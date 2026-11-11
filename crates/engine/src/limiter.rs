//! Token-bucket rate limiting for download speed caps (global + per-download).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
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
///
/// Both buckets are held behind `Arc` so a per-download limiter can share the
/// manager's *single* global bucket: without sharing, every download gets its
/// own full-rate bucket and N concurrent downloads reach N × the configured
/// global cap.
#[derive(Default)]
pub struct CombinedLimiter {
    global: Arc<TokenBucket>,
    per_download: Arc<TokenBucket>,
}

impl CombinedLimiter {
    pub fn new(global_rate: u64, per_download_rate: u64) -> Self {
        CombinedLimiter {
            global: Arc::new(TokenBucket::new(global_rate)),
            per_download: Arc::new(TokenBucket::new(per_download_rate)),
        }
    }

    /// Build a per-download limiter that **shares** the manager's global
    /// bucket and owns a fresh per-download bucket. All downloads created
    /// from the same `global` therefore draw from one shared allowance.
    pub fn from_shared(global: Arc<TokenBucket>, per_download_rate: u64) -> Self {
        CombinedLimiter {
            global,
            per_download: Arc::new(TokenBucket::new(per_download_rate)),
        }
    }

    /// The shared global bucket (for constructing shared limiters).
    pub fn global_bucket(&self) -> Arc<TokenBucket> {
        Arc::clone(&self.global)
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

    /// Pin the shared-global-bucket semantics: two limiters built from the
    /// same `global_bucket()` must draw from ONE allowance. With separate
    /// buckets, two concurrent 100-byte acquires at 100 B/s both finish in
    /// ~0.8 s (2 × burst 20 + 80 wait each, in parallel). Shared, they must
    /// serialise: 200 bytes total at 100 B/s minus 40 burst ≈ 1.6 s.
    #[tokio::test]
    async fn shared_global_bucket_is_shared() {
        let g = Arc::new(TokenBucket::new(100));
        let a = CombinedLimiter::from_shared(Arc::clone(&g), 0);
        let b = CombinedLimiter::from_shared(Arc::clone(&g), 0);
        let start = Instant::now();
        tokio::join!(a.acquire(100), b.acquire(100));
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(1100),
            "shared bucket must serialise the two acquires, took {elapsed:?}"
        );
    }
}
