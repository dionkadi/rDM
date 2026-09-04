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

/// Type of the async spawner the engine uses to launch download tasks and the
/// schedule loop. Takes a boxed future and is expected to drive it on whatever
/// async runtime the host provides. The Tauri shell passes its own runtime
/// handle so the engine never has to know about Tauri and never has to assume
/// it is being called from inside a Tokio context.
pub type Spawner = Arc<dyn Fn(BoxFuture<'static, ()>) + Send + Sync>;

/// Convenience alias for boxed `'static` futures. Equivalent to the standard
/// `std::pin::Pin<Box<dyn Future<Output = ()> + Send + 'static>>`.
pub type BoxFuture<'a, T = ()> =
    std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

fn default_spawner() -> Spawner {
    // If we are already inside a Tokio runtime, hand the future off to it.
    // If we are not, fall back to `tokio::spawn` which will panic — but
    // that is the same behaviour the engine had before, and the Tauri shell
    // is expected to provide a real spawner before the first download.
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        Arc::new(move |fut| {
            handle.spawn(fut);
        })
    } else {
        Arc::new(|fut| {
            tokio::spawn(fut);
        })
    }
}

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
    spawn: Spawner,
}

impl DownloadManager {
    /// Construct from storage; loads/initialises settings and resumes active downloads.
    pub fn new(storage: Storage) -> Self {
        let settings = config::load_or_default(&storage);
        Self::with_settings(storage, settings)
    }

    /// Construct from an explicit settings value.
    pub fn with_settings(storage: Storage, settings: Settings) -> Self {
        Self::with_settings_and_spawner(storage, settings, default_spawner())
    }

    /// Construct from an explicit settings value AND an async spawner. The Tauri
    /// shell uses this to inject `tauri::async_runtime::spawn` so that commands
    /// dispatched on the IPC thread can safely launch download tasks without
    /// requiring a Tokio runtime to be active on the calling thread.
    pub fn with_settings_and_spawner(
        storage: Storage,
        settings: Settings,
        spawn: Spawner,
    ) -> Self {
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
            global_proxy: settings.effective_proxy_url(),
            event_sink: Arc::new(|_| {}),
        });
        let inner = Inner {
            settings: RwLock::new(settings),
            tasks: Mutex::new(HashMap::new()),
            ctx: Mutex::new(ctx),
            storage,
            spawn,
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
            // On-disk verification: before spawning the task,
            // check the actual `.part` file size against the
            // chunk layout. If the file is smaller than what
            // the chunks claim, the disk is behind (e.g. a
            // crash mid-write, or the file was truncated by
            // an external tool). We adjust each chunk's
            // `downloaded` so the next range GET starts at the
            // correct offset. If the file is *larger* than
            // what the chunks claim (e.g. someone appended
            // bytes), we truncate it to the expected size
            // before resuming. If the `.part` file is missing,
            // we start from zero.
            //
            // Without this, a chunk that thinks it has N
            // bytes will issue a `Range: bytes=N-` request, and
            // if the file on disk is smaller, the write at
            // offset N will leave a hole of zeros up to N
            // (the user's "starts at 7% instead of 30%" bug
            // generalized to a "starts at 0 bytes" bug after a
            // crash).
            verify_chunks_against_disk(&mut d);
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
        let spawn = Arc::clone(&self.inner.spawn);
        let fut: BoxFuture<'static, ()> = Box::pin(async move {
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
        spawn(fut);
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
        let spawn = Arc::clone(&self.inner.spawn);
        let task_id = id.clone();
        let fut: BoxFuture<'static, ()> = Box::pin(async move {
            run_download(ctx, state).await;
            if let Some(e) = inner.tasks.lock().unwrap().get(&task_id) {
                e.running.store(false, Ordering::SeqCst);
            }
        });
        spawn(fut);
    }

    pub fn pause(&self, id: &str) {
        if let Some(e) = self.inner.tasks.lock().unwrap().get(id) {
            e.state.control.pause();
            let mut d = e.state.download.lock().unwrap();
            if d.status == DownloadStatus::Downloading || d.status == DownloadStatus::Queued {
                d.status = DownloadStatus::Paused;
                // Clear any stale error from a previous failed attempt so
                // the user isn't shown a 403 / connection-refused message
                // for a download they just paused manually.
                d.error = None;
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
                // Same rationale as `pause`: starting a fresh attempt
                // invalidates the previous failure's error message.
                d.error = None;
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
        // We deliberately do **not** call `self.cancel(id)` here. The
        // `cancel()` path sets `d.status = Canceled`, persists that
        // status to SQLite, and emits a `StatusChanged(d)` event with
        // the canceled download. The frontend's event listener sees
        // that StatusChanged *before* the Removed event we emit below
        // (Tauri's event channel dispatches them in order, but the
        // frontend's `removeDownload()` also calls
        // `refreshDownloads()` after the Tauri command returns — so
        // the StatusChanged is racing with the list snapshot, and
        // either one can win).
        //
        // The user-visible bug: the user clicks Remove, the list
        // briefly shows the entry with status="canceled" (because
        // StatusChanged was the last event to land before
        // `downloads.set()` from refreshDownloads gets overwritten by
        // a later StatusChanged that's still in flight), and Resume on
        // that ghost row hits the engine which has already deleted
        // the task — so the resume silently does nothing and the row
        // is then removed by the still-pending Removed event. The
        // result looks like a "canceled ghost" that the user can
        // only get rid of by reloading the view.
        //
        // The fix: skip `cancel()` entirely. Set the underlying
        // `control.cancel` flag directly so in-flight chunk workers
        // see the abort signal, drop the in-memory entry, delete the
        // SQLite row, and emit **only** `Removed(id)` — no
        // StatusChanged, no `d.status = Canceled`, no `save_download`.
        // The frontend sees a single Removed event and the row is
        // gone.
        let entry = self.inner.tasks.lock().unwrap().remove(id);
        if let Some(e) = entry {
            // Stop the chunk workers without touching the on-disk
            // row. The `DownloadControl::cancel()` flag is what
            // `chunk.rs` polls; flipping it is enough to make the
            // in-flight `run_download()` future unwind.
            e.state.control.cancel();
        }
        if let Err(e) = self.inner.storage.delete_download(id) {
            log::warn!("failed to delete download {id} from storage: {e}");
        }
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
        new_ctx.global_proxy = settings.effective_proxy_url();
        new_ctx.max_connections = settings.connections_per_download;
        drop(ctx);
        *self.inner.ctx.lock().unwrap() = Arc::new(new_ctx);
    }

    /// Update only the global proxy configuration (mode + manual URL) without
    /// touching the rest of the settings. Cheaper than `update_settings` when
    /// the UI just toggled the proxy picker.
    pub fn set_global_proxy(&self, mode: crate::model::ProxyMode, url: Option<String>) {
        {
            let mut s = self.inner.settings.write().unwrap();
            s.proxy_mode = mode;
            // Keep the manual URL even if mode != Manual so toggling back is
            // a one-click action.
            s.proxy = url.filter(|u| !u.is_empty());
            let _ = self.inner.storage.save_settings(&s);
        }
        let ctx = self.inner.ctx.lock().unwrap();
        let mut new_ctx = (**ctx).clone();
        let s = self.inner.settings.read().unwrap();
        new_ctx.global_proxy = s.effective_proxy_url();
        drop(s);
        drop(ctx);
        *self.inner.ctx.lock().unwrap() = Arc::new(new_ctx);
    }

    pub fn settings(&self) -> Settings {
        self.inner.settings.read().unwrap().clone()
    }

    pub fn list(&self) -> Vec<Download> {
        let tasks = self.inner.tasks.lock().unwrap();
        let mut out: Vec<Download> = tasks
            .values()
            .map(|e| e.state.download.lock().unwrap().clone())
            .collect();
        // Stable sort by (sort_key ASC, created_at ASC). This is the
        // user-facing order: the list view in the frontend should
        // match what we hand back here. `sort_by_key` + a tuple of
        // the same key is what gives us a stable, two-key sort
        // without pulling in `itertools`.
        out.sort_by(|a, b| {
            a.sort_key
                .cmp(&b.sort_key)
                .then(a.created_at.cmp(&b.created_at))
        });
        out
    }

    /// Reorder a subset of downloads to the order given by `ids`.
    ///
    /// Each id in `ids` gets a fresh `sort_key` spaced by `1000` so
    /// a future reorder that inserts *between* two existing rows
    /// has integer room to land (e.g. inserting at the midpoint
    /// `(a + b) / 2` works without collisions for ~10 reorder
    /// rounds). Rows that are *not* in `ids` are left alone, so
    /// the caller can reorder just the visible list (which is the
    /// common case from the drag-and-drop UI). The new keys are
    /// persisted to SQLite and a `StatusChanged` event is emitted
    /// for each affected row so the frontend can update the list
    /// order without a full refresh.
    pub fn reorder(&self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let now = chrono::Utc::now().timestamp_millis();
        // Use a unique `base` per call so a re-reorder of the same
        // set within the same millisecond still gets distinct keys.
        let base = now + (ids.len() as i64);
        for (i, id) in ids.iter().enumerate() {
            let entry = {
                let tasks = self.inner.tasks.lock().unwrap();
                tasks.get(id).map(|e| e.state.clone())
            };
            if let Some(state) = entry {
                let mut d = state.download.lock().unwrap();
                d.sort_key = base + (i as i64) * 1000;
                let _ = self.inner.storage.save_download(&d);
                self.emit(DownloadEvent::StatusChanged(d.clone()));
            }
        }
    }

    /// Set the per-download priority. Higher-priority downloads
    /// run before lower-priority ones when the scheduler needs to
    /// pick the next transfer. Persisted to SQLite, emitted as a
    /// `StatusChanged` event so the UI updates without a refresh.
    pub fn set_priority(&self, id: &str, priority: u8) {
        let entry = {
            let tasks = self.inner.tasks.lock().unwrap();
            tasks.get(id).map(|e| e.state.clone())
        };
        if let Some(state) = entry {
            let mut d = state.download.lock().unwrap();
            d.priority = priority.min(crate::model::PRIORITY_HIGH);
            let _ = self.inner.storage.save_download(&d);
            self.emit(DownloadEvent::StatusChanged(d.clone()));
        }
    }

    /// Set per-download HTTP headers and optional `Authorization`
    /// credentials. Both live in memory only — they are not
    /// persisted to SQLite, so a restart clears them. This is
    /// intentional: auth secrets shouldn't sit in a plain-text
    /// database file, and the use case is "I'm downloading one
    /// file from a site that needs login", not "I want every
    /// future download to use these credentials". The
    /// chunk worker applies the headers + auth at request
    /// build time (see `chunk.rs::apply_extra_headers`).
    pub fn set_headers_auth(
        &self,
        id: &str,
        headers: std::collections::BTreeMap<String, String>,
        auth: Option<crate::model::AuthSpec>,
    ) {
        let entry = {
            let tasks = self.inner.tasks.lock().unwrap();
            tasks.get(id).map(|e| e.state.clone())
        };
        if let Some(state) = entry {
            let mut d = state.download.lock().unwrap();
            d.headers = headers;
            d.auth = auth;
            // We deliberately do *not* call `save_download` or
            // emit a `StatusChanged` event here — headers and
            // auth are in-memory only, so a status-changed event
            // would be a lie (the on-disk row didn't change).
            // The frontend doesn't read these back, so a silent
            // update is fine.
        }
    }

    /// Replace the mirror list for a download. The engine tries
    /// `url` first; on transient failure (4xx/5xx/timeout) the
    /// task layer walks `mirrors` in order and replaces the
    /// primary URL with the first mirror that returns a
    /// successful probe. Mirrors live in memory only (same
    /// rationale as `set_headers_auth`).
    pub fn set_mirrors(&self, id: &str, mirrors: Vec<String>) {
        let entry = {
            let tasks = self.inner.tasks.lock().unwrap();
            tasks.get(id).map(|e| e.state.clone())
        };
        if let Some(state) = entry {
            let mut d = state.download.lock().unwrap();
            d.mirrors = mirrors;
        }
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

/// On-disk verification for a single download. Adjusts each
/// chunk's `downloaded` field to match the actual size of
/// the `.part` file on disk. The rules are:
///
/// 1. If the `.part` file is missing, reset all chunks to
///    `downloaded = 0` and return.
/// 2. If the file is smaller than the total claimed bytes
///    across chunks, walk the chunks in order and shrink
///    each `downloaded` proportionally so the sum equals
///    the file size. Chunks are laid out as
///    `[start, end]` byte ranges; chunk 0 starts at offset
///    `start_0`, chunk 1 at `start_1 = end_0 + 1`, etc.
/// 3. If the file is larger than the total claimed bytes
///    (e.g. an external tool appended), truncate the file
///    to the expected size before returning. The chunk
///    layout stays the same; we just discard the trailing
///    garbage.
///
/// This is called from `DownloadManager::start()` for
/// every loaded active download. It is also exposed for
/// tests via `pub(crate)` so the integration suite can
/// drive it directly without spinning up a full manager.
fn verify_chunks_against_disk(d: &mut crate::model::Download) {
    let part_path = d.part_path();
    let on_disk = match std::fs::metadata(&part_path) {
        Ok(m) => m.len(),
        Err(_) => {
            // No `.part` file: start from zero. This is the
            // "clean install" path — the user just added
            // the download and hasn't started it yet, or
            // the previous run cleaned up the partial.
            for chunk in &mut d.chunks {
                chunk.downloaded = 0;
            }
            d.downloaded = 0;
            return;
        }
    };
    if d.chunks.is_empty() {
        // Single-connection fallback (open-ended or no
        // ranges). The whole file is one chunk; just
        // trust the on-disk size.
        d.downloaded = on_disk;
        return;
    }
    let claimed: u64 = d.chunks.iter().map(|c| c.downloaded).sum();
    if on_disk == claimed {
        return; // nothing to do
    }
    if on_disk < claimed {
        // Disk is behind. For each chunk, figure out
        // how many bytes of *that chunk* are actually
        // on disk by looking at the file size relative
        // to the chunk's `[start, end]` byte range.
        //
        // Why per-chunk (not "distribute the total
        // across chunks"): a chunk's `downloaded`
        // field is the number of bytes written within
        // that chunk's byte range, not the offset into
        // the file. A chunk at `start=0, end=8MB`
        // contains at most 8MB; if the file is 10MB
        // total, chunk 0 has 8MB and chunk 1 has 2MB.
        // Distributing the total would set chunk 0 to
        // 10MB (overflow) and chunk 1 to 0, which
        // makes the chunk workers skip chunk 1
        // entirely and leave a hole of zeros from
        // 8MB to 10MB. The per-chunk approach sets
        // chunk 0 to its full size and chunk 1 to
        // (10MB - 8MB) = 2MB.
        let mut total = 0u64;
        for chunk in &mut d.chunks {
            let chunk_size = chunk.size();
            if chunk_size == u64::MAX {
                // Open-ended chunk: everything past
                // `chunk.start` is in this chunk.
                // The on-disk size minus the offset
                // gives us how many bytes were
                // written into this chunk.
                let bytes = on_disk.saturating_sub(chunk.start);
                chunk.downloaded = bytes;
            } else {
                // Closed-range chunk: at most
                // `chunk_size` bytes are in this
                // chunk, and only if the file
                // extends past `chunk.start`.
                if on_disk <= chunk.start {
                    chunk.downloaded = 0;
                } else {
                    let end_of_file = on_disk.saturating_sub(1);
                    let end_of_chunk = chunk.end;
                    let last_byte = end_of_file.min(end_of_chunk);
                    let bytes = last_byte - chunk.start + 1;
                    chunk.downloaded = bytes;
                }
            }
            total = total.saturating_add(chunk.downloaded);
        }
        d.downloaded = total;
        log::warn!(
            "on-disk verify: {} is behind (claimed={} B, on_disk={} B) — adjusted chunks",
            d.id, claimed, on_disk
        );
    } else {
        // Disk is ahead: the on-disk file is larger
        // than what `chunks[].downloaded` sums to.
        // This happens when the chunk workers have
        // written bytes that the SQLite aggregator
        // hasn't flushed yet (the flush runs every
        // 1s or 1 MiB). The SQLite `chunks[].downloaded`
        // values are STALE in this case.
        //
        // We use the on-disk file size as the source
        // of truth and re-derive each chunk's
        // `downloaded` value the same way as the
        // `on_disk < claimed` branch: figure out how
        // many bytes of *that chunk* are on disk by
        // intersecting the file's byte range with
        // the chunk's `[start, end]` range. Then, if
        // the on-disk file extends past the last
        // chunk's end (e.g. an external tool
        // appended bytes), truncate it.
        let mut total = 0u64;
        for chunk in &mut d.chunks {
            let chunk_size = chunk.size();
            if chunk_size == u64::MAX {
                let bytes = on_disk.saturating_sub(chunk.start);
                chunk.downloaded = bytes;
            } else {
                if on_disk <= chunk.start {
                    chunk.downloaded = 0;
                } else {
                    let end_of_file = on_disk.saturating_sub(1);
                    let end_of_chunk = chunk.end;
                    let last_byte = end_of_file.min(end_of_chunk);
                    let bytes = last_byte - chunk.start + 1;
                    chunk.downloaded = bytes;
                }
            }
            total = total.saturating_add(chunk.downloaded);
        }
        d.downloaded = total;
        log::warn!(
            "on-disk verify: {} is ahead (claimed={} B, on_disk={} B) — adjusted chunks from disk",
            d.id, claimed, on_disk
        );
        // Truncate the file to the expected total.
        // The chunk layout is [start, end] inclusive;
        // the last chunk's `end` is the byte offset
        // of the last byte. If the on-disk file
        // extends past that (e.g. an external tool
        // appended), truncate it.
        let last_end = d.chunks.last().map(|c| c.end).unwrap_or(0);
        if last_end != u64::MAX {
            let expected = last_end + 1;
            if on_disk > expected {
                // `std::fs::resize` doesn't exist; truncate by
                // opening the file and calling `set_len()`. This
                // is the standard Rust idiom for truncating a
                // file to a specific size — `set_len` on a
                // `File` handle is a thin wrapper around
                // `ftruncate(2)`.
                match std::fs::OpenOptions::new().write(true).open(&part_path) {
                    Ok(f) => {
                        if let Err(e) = f.set_len(expected) {
                            log::warn!(
                                "on-disk verify: failed to truncate {} from {} B to {} B: {e}",
                                d.id, on_disk, expected
                            );
                        }
                    }
                    Err(e) => log::warn!(
                        "on-disk verify: failed to open {} for truncation: {e}",
                        d.id
                    ),
                }
            }
        }
        // Open-ended chunks (`end == u64::MAX`): don't
        // truncate. The `.part` file might be longer
        // than the chunks claim because the user is
        // still downloading and the aggregator
        // hasn't caught up. Leave it alone.
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
            sort_key: 0,
            priority: 1,
            headers: std::collections::BTreeMap::new(),
            auth: None,
            mirrors: Vec::new(),
            media: None,
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

    /// Reordering changes the order `list()` returns. A subsequent
    /// reorder that interleaves between two previously-reordered
    /// rows must still produce a stable, unique ordering (i.e. no
    /// two rows end up with the same `sort_key`).
    #[tokio::test]
    async fn reorder_changes_list_order() {
        let storage = Storage::open_memory().unwrap();
        let mgr = DownloadManager::new(storage);
        mgr.add(new_download("a", "https://example.com/a.bin"));
        mgr.add(new_download("b", "https://example.com/b.bin"));
        mgr.add(new_download("c", "https://example.com/c.bin"));
        // Reorder to b, a, c.
        mgr.reorder(&["b".to_string(), "a".to_string(), "c".to_string()]);
        let list = mgr.list();
        assert_eq!(list.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), vec!["b", "a", "c"]);
        // Interleave: move c to the front.
        mgr.reorder(&["c".to_string(), "b".to_string(), "a".to_string()]);
        let list = mgr.list();
        assert_eq!(list.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), vec!["c", "b", "a"]);
        // The sort_keys must all be unique after interleaving.
        let mut keys: Vec<i64> = list.iter().map(|d| d.sort_key).collect();
        keys.sort();
        let unique: std::collections::HashSet<i64> = keys.iter().copied().collect();
        assert_eq!(keys.len(), unique.len(), "sort_key collisions after reorder: {keys:?}");
    }

    /// Setting priority on a download updates both the in-memory
    /// row and the persisted SQLite row, so a restart preserves
    /// the user's choice.
    #[tokio::test]
    async fn set_priority_persists() {
        let storage = Storage::open_memory().unwrap();
        let mgr = DownloadManager::new(storage);
        mgr.add(new_download("a", "https://example.com/a.bin"));
        mgr.set_priority("a", crate::model::PRIORITY_HIGH);
        assert_eq!(mgr.get("a").unwrap().priority, crate::model::PRIORITY_HIGH);
        // Out-of-range priorities are clamped (PRIORITY_HIGH = 2).
        mgr.set_priority("a", 99);
        assert_eq!(mgr.get("a").unwrap().priority, crate::model::PRIORITY_HIGH);
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

    /// Pausing a download should clear any stale error message from a
    /// previous failed attempt so the UI doesn't keep showing "HTTP 403"
    /// next to a row the user just paused manually. The same applies to
    /// resuming — starting a fresh attempt invalidates the old failure.
    #[tokio::test]
    async fn pause_clears_stale_error() {
        let storage = Storage::open_memory().unwrap();
        let mgr = DownloadManager::new(storage);
        mgr.add(new_download("a", "https://example.com/a.bin"));
        // Inject a stale error and force the status to Downloading so the
        // pause() branch will fire (mirrors what the engine would do on
        // a real failed attempt before the user clicks Pause).
        {
            let entry = mgr.inner.tasks.lock().unwrap().get("a").cloned().unwrap();
            let mut d = entry.state.download.lock().unwrap();
            d.status = DownloadStatus::Downloading;
            d.error = Some("HTTP 403 Forbidden".into());
        }
        mgr.pause("a");
        let after = mgr.get("a").unwrap();
        assert_eq!(after.status, DownloadStatus::Paused);
        assert!(after.error.is_none(), "pause() must clear the error field, got {:?}", after.error);
    }

    #[tokio::test]
    async fn resume_clears_stale_error() {
        let storage = Storage::open_memory().unwrap();
        let mgr = DownloadManager::new(storage);
        mgr.add(new_download("a", "https://example.com/a.bin"));
        // Force into Paused with a stale error.
        {
            let entry = mgr.inner.tasks.lock().unwrap().get("a").cloned().unwrap();
            let mut d = entry.state.download.lock().unwrap();
            d.status = DownloadStatus::Paused;
            d.error = Some("connection refused".into());
        }
        mgr.resume("a");
        let after = mgr.get("a").unwrap();
        // Status is back to Queued (the engine will try to spawn a real
        // download; harmless in a test).
        assert_eq!(after.status, DownloadStatus::Queued);
        assert!(after.error.is_none(), "resume() must clear the error field, got {:?}", after.error);
    }

    /// Regression test: `remove()` must delete the SQLite row, not just
    /// drop the in-memory entry. Otherwise the download would reappear
    /// on the next launch via `load_active()` — which is exactly the
    /// "remove doesn't persist" bug we hit.
    ///
    /// We simulate the restart by opening a fresh `Storage` against the
    /// same in-memory database (`open_in_memory` returns a new
    /// connection each time, so the underlying SQLite file is shared
    /// when the test uses `tempfile`-backed paths; here we keep it
    /// simple by reusing the same `Storage` handle and just calling
    /// `load_active()` after the remove, which is what `start()` does).
    #[tokio::test]
    async fn remove_persists_across_reload() {
        let storage = Storage::open_memory().unwrap();
        let mgr = DownloadManager::new(storage);
        mgr.add(new_download("a", "https://example.com/a.bin"));
        mgr.add(new_download("b", "https://example.com/b.bin"));
        assert_eq!(mgr.list().len(), 2);

        mgr.remove("a");

        // In-memory: removed.
        assert_eq!(mgr.list().len(), 1);
        assert_eq!(mgr.list()[0].id, "b");

        // Simulate restart: the engine's `start()` calls
        // `load_active()`, which reads from SQLite. The removed
        // download must NOT come back.
        let active = mgr.inner.storage.load_active().unwrap();
        assert_eq!(active.len(), 1, "removed download came back from storage: {active:?}");
        assert_eq!(active[0].id, "b");
    }

    /// Regression test: `remove()` must NOT emit a `StatusChanged`
    /// with status="canceled" before emitting `Removed`. Otherwise
    /// the frontend's `StatusChanged` handler merges the canceled
    /// download back into the list (because `findIndex` returns -1
    /// for a row that's already been removed in-memory), and the
    /// user sees a "canceled ghost" row that only disappears when
    /// the still-pending `Removed` event lands.
    ///
    /// The old implementation called `self.cancel(id)` from
    /// `remove()`, which saved `status='canceled'` to SQLite and
    /// emitted `StatusChanged(d)` before the `Removed(id)` event.
    /// The fix: skip `cancel()` entirely; set the underlying
    /// `control.cancel` flag, drop the in-memory entry, delete the
    /// SQLite row, and emit only `Removed(id)`.
    #[tokio::test]
    async fn remove_does_not_emit_canceled_status_change() {
        use std::sync::Mutex;
        let storage = Storage::open_memory().unwrap();
        let mgr = DownloadManager::new(storage);
        // Wire up an event sink that records every event the manager
        // emits.
        let events: std::sync::Arc<Mutex<Vec<DownloadEvent>>> =
            std::sync::Arc::new(Mutex::new(Vec::new()));
        {
            let events = events.clone();
            mgr.set_event_sink(std::sync::Arc::new(move |e| {
                events.lock().unwrap().push(e);
            }));
        }
        mgr.add(new_download("ghost", "https://example.com/ghost.zip"));
        // Snapshot the events emitted so far (the `Added` from
        // `mgr.add()`); we only want to assert on the events that
        // `remove()` itself produces.
        let before_remove = events.lock().unwrap().len();

        mgr.remove("ghost");

        let recorded: Vec<DownloadEvent> = {
            let all = events.lock().unwrap();
            all.iter().skip(before_remove).cloned().collect()
        };
        // We must see exactly one event from `remove()` — the
        // `Removed`. Anything else (especially a `StatusChanged`
        // with status=Canceled) is the bug.
        assert_eq!(
            recorded.len(),
            1,
            "remove() emitted {} events (expected 1): {:?}",
            recorded.len(),
            recorded
        );
        match &recorded[0] {
            DownloadEvent::Removed(id) => assert_eq!(id, "ghost"),
            other => panic!("expected Removed event, got {other:?}"),
        }
        // Sanity: the in-memory entry is gone and so is the SQLite
        // row.
        assert!(mgr.get("ghost").is_none());
        assert_eq!(mgr.inner.storage.load_active().unwrap().len(), 0);
    }
}
