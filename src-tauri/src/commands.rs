//! Tauri command surface. Each command delegates to the `DownloadManager`
//! held in managed state.

use dm_engine::manager::DownloadManager;
use dm_engine::model::{ChecksumSpec, Download, DownloadStatus, Settings};
use dm_engine::protocol;
use tauri::State;
use tauri_plugin_notification::NotificationExt;

/// Health-check command used by the scaffold to verify the Rust<->JS bridge.
#[tauri::command]
pub fn ping() -> String {
    format!("pong @ {}", chrono::Utc::now().to_rfc3339())
}

/// Enqueue a new download. Returns the created `Download` (status `Queued`).
#[tauri::command]
pub fn add_download(
    state: State<'_, DownloadManager>,
    url: String,
    category: Option<String>,
    filename: Option<String>,
    speed_limit: Option<u64>,
    checksum: Option<ChecksumSpec>,
) -> Download {
    let settings = state.settings();
    let dir = state.save_dir_for(category.as_deref());
    let fname = filename.unwrap_or_else(|| protocol::suggest_filename(&url, None, "download.bin"));
    let save_path = dir.join(&fname);

    let mut d = Download::new(url);
    d.category = category;
    d.filename = fname;
    d.save_path = save_path;
    d.speed_limit = speed_limit;
    d.checksum = checksum;
    d.status = DownloadStatus::Queued;

    state.add(d.clone());
    d
}

/// Snapshot of all in-memory downloads.
#[tauri::command]
pub fn list_downloads(state: State<'_, DownloadManager>) -> Vec<Download> {
    state.list()
}

/// Look up a single download by id.
#[tauri::command]
pub fn get_download(state: State<'_, DownloadManager>, id: String) -> Option<Download> {
    state.get(&id)
}

#[tauri::command]
pub fn pause_download(state: State<'_, DownloadManager>, id: String) {
    state.pause(&id);
}

#[tauri::command]
pub fn resume_download(state: State<'_, DownloadManager>, id: String) {
    state.resume(&id);
}

#[tauri::command]
pub fn cancel_download(state: State<'_, DownloadManager>, id: String) {
    state.cancel(&id);
}

#[tauri::command]
pub fn remove_download(state: State<'_, DownloadManager>, id: String) {
    state.remove(&id);
}

/// Set or clear a per-download speed cap (bytes/sec).
#[tauri::command]
pub fn set_speed_limit(state: State<'_, DownloadManager>, id: String, limit: Option<u64>) {
    state.set_speed_limit(&id, limit);
}

/// Set or clear the global speed cap (bytes/sec).
#[tauri::command]
pub fn set_global_speed_limit(state: State<'_, DownloadManager>, limit: Option<u64>) {
    state.set_global_speed_limit(limit);
}

#[tauri::command]
pub fn get_settings(state: State<'_, DownloadManager>) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn update_settings(state: State<'_, DownloadManager>, settings: Settings) {
    state.update_settings(settings);
}

/// Resolve the on-disk directory for a (optional) category.
#[tauri::command]
pub fn save_dir_for(state: State<'_, DownloadManager>, category: Option<String>) -> String {
    state
        .save_dir_for(category.as_deref())
        .to_string_lossy()
        .to_string()
}

/// Show a desktop notification (used when a download completes).
#[tauri::command]
pub fn notify_on_complete(app: tauri::AppHandle, title: String, body: String) {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}
