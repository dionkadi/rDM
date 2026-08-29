//! The download manager: owns all tasks, enforces scheduling/limits, persists state
//! and broadcasts lifecycle events to the (Tauri) frontend.

use crate::config;
use crate::limiter::CombinedLimiter;
use crate::model::{Download, DownloadStatus, Settings};
use crate::scheduler::DownloadScheduler;
use crate::storage::Storage;
use crate::task::{run_download, TaskState};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// Lifecycle events emitted to the frontend.
#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Added(Download),
    Progress(Download),
    StatusChanged(Download),
    Completed(Download),
    Error(Download),
    Removed(String),
}

/// Callback invoked for every event (the Tauri layer emits these to the webview).
pub type EventSink = Arc<dyn Fn(DownloadEvent) + Send + Sync>;

/// Shared, immutable-per-run context handed to each download task.
#[derive(Clone)]
pub struct RunContext {
    pub scheduler: Arc<DownloadScheduler>,
    pub global_limiter: Arc<CombinedLimiter>,
    pub storage: Storage,
    pub max_connections: usize,
    pub global_proxy: Option<String>,
    pub event_sink: EventSink,
}

impl RunContext {
    pub fn emit(&self, e: DownloadEvent) {
        (self.event_sink)(e);
    }
}

struct TaskEntry {
    state: Arc<TaskState>,
    running: AtomicBool,
}

impl Clone for TaskEntry {
    fn clone(&self) -> Self {
        TaskEntry {
            state: Arc::clone(&self.state),
            running: AtomicBool::new(self.running.load(Ordering::SeqCst)),
        }
    }
}

/// The download manager. Cheap to clone (wraps an `Arc`).
#[derive(Clone)]
pub struct DownloadManager {
    inner: Arc<Inner>,
}

struct Inner {
    settings: RwLock<Settings>,
    tasks: Mutex<HashMap<String, TaskEntry>>,
    ctx: Mutex<Arc<RunContext>>,
    storage: Storage,
}

impl DownloadManager {
    /// Construct from storage; loads/initialises settings and resumes active downloads.
    pub fn new(storage: Storage) -> Self {
        let settings = config::load_or_default(&storage);
        Self::with_settings(storage, settings)
    }

    /// Construct from an explicit settings value.
    pub fn with_settings(storage: Storage, settings: Settings) -> Self {
        let scheduler = DownloadScheduler::new(
            settings.max_concurrent_downloads,
            settings.connections_per_download,
        );
        let global_limiter = Arc::new(CombinedLimiter::new(settings.speed_limit_global.unwrap_or(0), 0));
        let ctx = Arc::new(RunContext {
            scheduler,
            global_limiter,
            storage: storage.clone(),
            max_connections: settings.connections_per_download,
            global_proxy: settings.proxy.clone(),
            event_sink: Arc::new(|_| {}),
        });
        let inner = Inner {
            settings: RwLock::new(settings),
            tasks: Mutex::new(HashMap::new()),
            ctx: Mutex::new(ctx),
            storage,
        };
        DownloadManager {
            inner: Arc::new(inner),
        }
    }

    /// Replace the event sink (e.g. wire it to Tauri once the app is up).
    pub fn set_event_sink(&self, sink: EventSink) {
        let mut ctx = self.inner.ctx.lock().unwrap();
        let mut new_ctx = (**ctx).clone();
        new_ctx.event_sink = sink;
        *ctx = Arc::new(new_ctx);
    }

    /// Load active downloads from storage and resume queued ones.
    pub async fn start(&self) {
        let in_window = self.in_schedule_window();
        let active = self.inner.storage.load_active().unwrap_or_default();
        for mut d in active {
            let was_queued = matches!(
                d.status,
                DownloadStatus::Queued | DownloadStatus::Connecting | DownloadStatus::Downloading
            );
            let resumed = d.status == DownloadStatus::Paused || d.status == DownloadStatus::Scheduled;
            d.status = if resumed {
                DownloadStatus::Paused
            } else if !in_window {
                DownloadStatus::Scheduled
            } else {
                DownloadStatus::Queued
            };
            let state = Arc::new(TaskState {
                id: d.id.clone(),
                download: Arc::new(Mutex::new(d)),
                control: crate::control::DownloadControl::new(),
            });
            self.inner
                .tasks
                .lock()
                .unwrap()
                .insert(state.id.clone(), TaskEntry {
                    state: Arc::clone(&state),
                    running: AtomicBool::new(false),
                });
            if was_queued && in_window {
                self.spawn(state.id.clone());
            }
        }
    }

    /// Enqueue a new download (status should be `Queued`, `Paused` or `Scheduled`).
    pub fn add(&self, mut download: Download) {
        // Respect the schedule window: if it is currently closed, park the
        // download as `Scheduled` instead of starting it immediately.
        if matches!(
            download.status,
            DownloadStatus::Queued | DownloadStatus::Connecting | DownloadStatus::Downloading
        ) && !self.in_schedule_window()
        {
            download.status = DownloadStatus::Scheduled;
        }
        let state = Arc::new(TaskState {
            id: download.id.clone(),
            download: Arc::new(Mutex::new(download.clone())),
            control: crate::control::DownloadControl::new(),
        });
        let id = state.id.clone();
        let should_spawn = matches!(
            download.status,
            DownloadStatus::Queued | DownloadStatus::Connecting | DownloadStatus::Downloading
        );
        self.inner
            .tasks
            .lock()
            .unwrap()
            .insert(id.clone(), TaskEntry {
                state,
                running: AtomicBool::new(false),
            });
        let _ = self.inner.storage.save_download(&download);
        self.emit(DownloadEvent::Added(download));
        if should_spawn {
            self.spawn(id);
        }
    }

    /// `true` when downloads may currently run (schedule window open or disabled).
    fn in_schedule_window(&self) -> bool {
        self.inner
            .settings
            .read()
            .unwrap()
            .in_schedule_window(chrono::Utc::now())
    }

    /// Spawn a background task that opens/closes the schedule window, pausing
    /// active downloads when it closes and promoting `Scheduled` downloads when it
    /// reopens. Call once after `start`.
    pub fn run_schedule_loop(&self) {
        let mgr = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(15));
            let _ = ticker.tick().await; // discard the immediate first tick
            let mut was_open = mgr.in_schedule_window();
            loop {
                ticker.tick().await;
                let open = mgr.in_schedule_window();
                if open == was_open {
                    continue;
                }
                if open {
                    // Window opened: promote Scheduled -> Queued and start them.
                    let ids: Vec<String> = {
                        let tasks = mgr.inner.tasks.lock().unwrap();
                        tasks
                            .iter()
                            .filter_map(|(id, e)| {
                                let d = e.state.download.lock().unwrap();
                                (d.status == DownloadStatus::Scheduled).then(|| id.clone())
                            })
                            .collect()
                    };
                    for id in ids {
                        if let Some(e) = mgr.inner.tasks.lock().unwrap().get(&id) {
                            let mut d = e.state.download.lock().unwrap();
                            d.status = DownloadStatus::Queued;
                            let _ = mgr.inner.storage.save_download(&d);
                            mgr.emit(DownloadEvent::StatusChanged(d.clone()));
                        }
                        mgr.spawn(id);
                    }
                } else {
                    // Window closed: pause active downloads and mark them Scheduled.
                    let ids: Vec<String> = {
                        let tasks = mgr.inner.tasks.lock().unwrap();
                        tasks
                            .iter()
                            .filter_map(|(id, e)| {
                                let d = e.state.download.lock().unwrap();
                                matches!(
                                    d.status,
                                    DownloadStatus::Downloading
                                        | DownloadStatus::Queued
                                        | DownloadStatus::Connecting
                                )
                                .then(|| id.clone())
                            })
                            .collect()
                    };
                    for id in ids {
                        if let Some(e) = mgr.inner.tasks.lock().unwrap().get(&id) {
                            e.state.control.pause();
                            let mut d = e.state.download.lock().unwrap();
                            d.status = DownloadStatus::Scheduled;
                            let _ = mgr.inner.storage.save_download(&d);
                            mgr.emit(DownloadEvent::StatusChanged(d.clone()));
                        }
                    }
                }
                was_open = open;
            }
        });
    }

    /// Spawn the run task for a download if it is not already running.
    fn spawn(&self, id: String) {
        let entry = {
            let tasks = self.inner.tasks.lock().unwrap();
            tasks.get(&id).cloned()
        };
        let Some(entry) = entry else { return };
        if entry.running.swap(true, Ordering::SeqCst) {
            return; // already running
        }
        let state = entry.state.clone();
        let inner = self.inner.clone();
        let ctx = self.inner.ctx.lock().unwrap().clone();
        tokio::spawn(async move {
            run_download(ctx, state).await;
            if let Some(e) = inner.tasks.lock().unwrap().get(&id) {
                e.running.store(false, Ordering::SeqCst);
            }
        });
    }

    pub fn pause(&self, id: &str) {
        if let Some(e) = self.inner.tasks.lock().unwrap().get(id) {
            e.state.control.pause();
            let mut d = e.state.download.lock().unwrap();
            if d.status == DownloadStatus::Downloading || d.status == DownloadStatus::Queued {
                d.status = DownloadStatus::Paused;
                let _ = self.inner.storage.save_download(&d);
                self.emit(DownloadEvent::StatusChanged(d.clone()));
            }
        }
    }

    pub fn resume(&self, id: &str) {
        if let Some(e) = self.inner.tasks.lock().unwrap().get(id) {
            e.state.control.resume();
            let mut d = e.state.download.lock().unwrap();
            if d.status == DownloadStatus::Paused {
                d.status = DownloadStatus::Queued;
                let _ = self.inner.storage.save_download(&d);
                self.emit(DownloadEvent::StatusChanged(d.clone()));
            }
        }
        // NOTE: the `tasks` guard must be released before `spawn`, which re-locks it.
        self.spawn(id.to_string());
    }

    pub fn cancel(&self, id: &str) {
        if let Some(e) = self.inner.tasks.lock().unwrap().get(id) {
            e.state.control.cancel();
            let mut d = e.state.download.lock().unwrap();
            d.status = DownloadStatus::Canceled;
            let _ = self.inner.storage.save_download(&d);
            self.emit(DownloadEvent::StatusChanged(d.clone()));
        }
    }

    pub fn remove(&self, id: &str) {
        self.cancel(id);
        self.inner.tasks.lock().unwrap().remove(id);
        self.emit(DownloadEvent::Removed(id.to_string()));
    }

    /// Set or clear a per-download speed limit (bytes/sec).
    pub fn set_speed_limit(&self, id: &str, limit: Option<u64>) {
        if let Some(e) = self.inner.tasks.lock().unwrap().get(id) {
            let mut d = e.state.download.lock().unwrap();
            d.speed_limit = limit;
            let _ = self.inner.storage.save_download(&d);
        }
    }

    /// Set or clear a per-download proxy.
    pub fn set_proxy(&self, id: &str, proxy: Option<String>) {
        if let Some(e) = self.inner.tasks.lock().unwrap().get(id) {
            let mut d = e.state.download.lock().unwrap();
            d.proxy = proxy;
            let _ = self.inner.storage.save_download(&d);
        }
    }

    pub fn set_global_speed_limit(&self, limit: Option<u64>) {
        {
            let mut s = self.inner.settings.write().unwrap();
            s.speed_limit_global = limit;
            let _ = self.inner.storage.save_settings(&s);
        }
        let ctx = self.inner.ctx.lock().unwrap();
        ctx.global_limiter.set_global_rate(limit.unwrap_or(0));
    }

    pub fn update_settings(&self, settings: Settings) {
        {
            let mut s = self.inner.settings.write().unwrap();
            *s = settings.clone();
            let _ = self.inner.storage.save_settings(&settings);
            let _ = self.inner.storage.save_categories(&settings.categories);
        }
        let ctx = self.inner.ctx.lock().unwrap();
        ctx.scheduler
            .set_limits(settings.max_concurrent_downloads, settings.connections_per_download);
        ctx.global_limiter.set_global_rate(settings.speed_limit_global.unwrap_or(0));
        // Rebuild context with new proxy/connections.
        let mut new_ctx = (**ctx).clone();
        new_ctx.global_proxy = settings.proxy.clone();
        new_ctx.max_connections = settings.connections_per_download;
        drop(ctx);
        *self.inner.ctx.lock().unwrap() = Arc::new(new_ctx);
    }

    pub fn settings(&self) -> Settings {
        self.inner.settings.read().unwrap().clone()
    }

    pub fn list(&self) -> Vec<Download> {
        let tasks = self.inner.tasks.lock().unwrap();
        tasks
            .values()
            .map(|e| e.state.download.lock().unwrap().clone())
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<Download> {
        self.inner
            .tasks
            .lock()
            .unwrap()
            .get(id)
            .map(|e| e.state.download.lock().unwrap().clone())
    }

    /// Compute the save directory for a category (used by the add-URL dialog).
    pub fn save_dir_for(&self, category: Option<&str>) -> PathBuf {
        let settings = self.inner.settings.read().unwrap();
        match category {
            Some(cat) => settings
                .categories
                .iter()
                .find(|c| c.id == cat)
                .map(|c| c.directory.clone())
                .unwrap_or_else(|| settings.default_directory.clone()),
            None => settings.default_directory.clone(),
        }
    }

    fn emit(&self, e: DownloadEvent) {
        let ctx = self.inner.ctx.lock().unwrap();
        ctx.emit(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Download;

    fn new_download(id: &str, url: &str) -> Download {
        Download {
            id: id.into(),
            url: url.into(),
            filename: String::new(),
            save_path: PathBuf::from(format!("/tmp/{id}")),
            total_size: None,
            downloaded: 0,
            status: DownloadStatus::Queued,
            category: None,
            content_type: None,
            chunks: vec![],
            speed_limit: None,
            proxy: None,
            checksum: None,
            error: None,
            created_at: chrono::Utc::now(),
            finished_at: None,
            can_resume: false,
        }
    }

    #[tokio::test]
    async fn add_and_list() {
        let storage = Storage::open_memory().unwrap();
        let mgr = DownloadManager::new(storage);
        mgr.add(new_download("a", "https://example.com/a.bin"));
        let list = mgr.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "a");
    }

    #[tokio::test]
    async fn pause_and_resume() {
        let storage = Storage::open_memory().unwrap();
        let mgr = DownloadManager::new(storage);
        mgr.add(new_download("a", "https://example.com/a.bin"));
        mgr.pause("a");
        assert_eq!(mgr.get("a").unwrap().status, DownloadStatus::Paused);
        mgr.resume("a");
        // Should be queued (will try to spawn a real download; harmless for test).
        assert_eq!(mgr.get("a").unwrap().status, DownloadStatus::Queued);
    }

    #[tokio::test]
    async fn global_speed_limit_updates() {
        let storage = Storage::open_memory().unwrap();
        let mgr = DownloadManager::new(storage);
        mgr.set_global_speed_limit(Some(1234));
        assert_eq!(mgr.settings().speed_limit_global, Some(1234));
    }
}
