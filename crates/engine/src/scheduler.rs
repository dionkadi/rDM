//! Concurrency scheduling: limits total simultaneous downloads and the number of
//! connections used by any single download.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;

/// Tracks how many connection permits each active download currently holds so a
/// download can never exceed `connections_per_download`.
pub struct DownloadScheduler {
    /// Global limit on simultaneously active downloads (replaceable on settings change).
    download_slots: Mutex<Arc<Semaphore>>,
    /// Per-download connection permits (keyed by download id).
    per_download: Mutex<HashMap<String, Arc<Semaphore>>>,
    connections_per_download: Mutex<usize>,
}

impl DownloadScheduler {
    pub fn new(max_concurrent_downloads: usize, connections_per_download: usize) -> Arc<Self> {
        Arc::new(Self {
            download_slots: Mutex::new(Arc::new(Semaphore::new(max_concurrent_downloads.max(1)))),
            per_download: Mutex::new(HashMap::new()),
            connections_per_download: Mutex::new(connections_per_download.max(1)),
        })
    }

    /// Acquire a slot to begin a new download. Returns a guard that releases the
    /// slot when dropped.
    pub async fn acquire_download(&self) -> DownloadSlotGuard {
        let sem = self.download_slots.lock().unwrap().clone();
        let permit = sem.acquire_owned().await.expect("download semaphore closed");
        DownloadSlotGuard { _permit: permit }
    }

    /// Acquire one connection permit for a specific download.
    pub async fn acquire_connection(&self, id: &str) -> ConnectionGuard {
        let sem = {
            let mut map = self.per_download.lock().unwrap();
            let limit = *self.connections_per_download.lock().unwrap();
            map.entry(id.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(limit)))
                .clone()
        };
        let permit = sem.acquire_owned().await.expect("connection semaphore closed");
        ConnectionGuard { _permit: permit }
    }

    /// Drop per-download state once a download fully finishes.
    pub fn release_download_state(&self, id: &str) {
        self.per_download.lock().unwrap().remove(id);
    }

    /// Update limits. New downloads use the new caps; in-flight ones keep their
    /// existing permits (acceptable for a live settings change).
    pub fn set_limits(&self, max_concurrent_downloads: usize, connections_per_download: usize) {
        *self.download_slots.lock().unwrap() =
            Arc::new(Semaphore::new(max_concurrent_downloads.max(1)));
        *self.connections_per_download.lock().unwrap() = connections_per_download.max(1);
        self.per_download.lock().unwrap().clear();
    }
}

/// Released when a download slot is dropped.
pub struct DownloadSlotGuard {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

/// Released when a connection permit is dropped.
pub struct ConnectionGuard {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn limits_downloads() {
        let s = DownloadScheduler::new(2, 2);
        let _a = s.acquire_download().await;
        let _b = s.acquire_download().await;
        let third = tokio::time::timeout(Duration::from_millis(100), s.acquire_download()).await;
        assert!(third.is_err(), "third download should be blocked");
    }

    #[tokio::test]
    async fn limits_connections_per_download() {
        let s = DownloadScheduler::new(5, 2);
        let _c1 = s.acquire_connection("x").await;
        let _c2 = s.acquire_connection("x").await;
        let third = tokio::time::timeout(Duration::from_millis(100), s.acquire_connection("x")).await;
        assert!(third.is_err(), "third connection should be blocked");
    }
}
