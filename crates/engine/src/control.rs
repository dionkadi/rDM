//! Per-download runtime control (pause / resume / cancel) shared between the task
//! and its chunk workers.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

/// Shared control flags for one download. Chunks poll these between writes.
#[derive(Debug)]
pub struct DownloadControl {
    paused: AtomicBool,
    cancel: AtomicBool,
    notify: Notify,
}

pub type SharedControl = Arc<DownloadControl>;

impl DownloadControl {
    pub fn new() -> SharedControl {
        Arc::new(Self {
            paused: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            notify: Notify::new(),
        })
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.paused.store(false, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_canceled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Block while paused. Returns `true` if cancellation was requested.
    pub async fn wait_while_paused(&self) -> bool {
        if self.cancel.load(Ordering::SeqCst) {
            return true;
        }
        if !self.paused.load(Ordering::SeqCst) {
            return false;
        }
        self.notify.notified().await;
        self.cancel.load(Ordering::SeqCst)
    }
}
