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
    /// Set when the user *removed* the download. Distinct from
    /// `cancel`, which is also used for the user-visible Cancel
    /// action (where the row must stay, showing status = Canceled).
    ///
    /// The distinction matters at finalize time: a cancelled download
    /// should persist `Canceled` and emit a `StatusChanged`, but a
    /// removed one must do neither — its row was just deleted, and
    /// `Storage::save_download` is an UPSERT, so persisting would
    /// *re-insert* the row the user just removed (and it would come
    /// back on the next launch), while the `StatusChanged` would
    /// resurrect the row in a frontend that has already dropped it.
    removed: AtomicBool,
    notify: Notify,
}

pub type SharedControl = Arc<DownloadControl>;

impl DownloadControl {
    pub fn new() -> SharedControl {
        Arc::new(Self {
            paused: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            removed: AtomicBool::new(false),
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

    /// Stop this download permanently because its row is being deleted.
    /// Implies `cancel()`, and additionally marks the download so the
    /// finalize path skips persisting and emitting.
    pub fn remove(&self) {
        self.removed.store(true, Ordering::SeqCst);
        self.cancel();
    }

    /// True once `remove()` has been called: the download no longer
    /// exists, so it must not write to storage or emit events.
    pub fn is_removed(&self) -> bool {
        self.removed.load(Ordering::SeqCst)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `remove()` has to imply `cancel()`: the chunk workers only look
    /// at `is_canceled()` to decide to unwind, and `run_download`'s
    /// finalize path is what we are trying to reach (so it can skip
    /// persisting). If removal didn't cancel, a removed download would
    /// keep transferring.
    #[test]
    fn remove_implies_cancel_and_is_distinguishable() {
        let c = DownloadControl::new();
        assert!(!c.is_canceled());
        assert!(!c.is_removed());

        c.remove();

        assert!(c.is_canceled(), "chunk workers poll `is_canceled` to abort");
        assert!(
            c.is_removed(),
            "the finalize path needs to tell removal apart from a plain cancel"
        );
    }

    /// The user-visible Cancel keeps the row (status = Canceled), so it
    /// must NOT set the removed flag — otherwise its finalize path would
    /// skip persisting the canceled status and the UI would never learn
    /// about it.
    #[test]
    fn cancel_does_not_look_like_removal() {
        let c = DownloadControl::new();
        c.cancel();
        assert!(c.is_canceled());
        assert!(!c.is_removed());
    }
}
