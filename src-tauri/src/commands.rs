//! Tauri command surface. Each command delegates to the `DownloadManager`
//! held in managed state.

use dm_engine::manager::DownloadManager;
use dm_engine::model::{AuthSpec, ChecksumSpec, Download, DownloadStatus, ProxyMode, Settings};
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

/// Reorder a subset of downloads to the order given by `ids`.
///
/// The frontend calls this after a drag-and-drop or
/// Alt+↑/↓ keyboard reorder. Each id in `ids` is assigned a
/// fresh `sort_key` spaced by 1000, so subsequent interleaved
/// reorders still have integer room to land. Rows that are
/// not in `ids` are left alone. Per-row `StatusChanged` events
/// are emitted so the list re-renders without a full refresh.
#[tauri::command]
pub fn reorder_downloads(state: State<'_, DownloadManager>, ids: Vec<String>) {
    state.reorder(&ids);
}

/// Set the per-download priority.
///
/// `priority` is clamped to the `[0, 2]` range by the engine,
/// where `0` = low, `1` = normal (default), `2` = high. Higher
/// priority downloads run before lower priority when the
/// scheduler picks the next transfer.
#[tauri::command]
pub fn set_download_priority(
    state: State<'_, DownloadManager>,
    id: String,
    priority: u8,
) {
    state.set_priority(&id, priority);
}

/// Set per-download HTTP headers and optional `Authorization`
/// credentials. Both live in memory only for v1 — a restart
/// clears them. Auth secrets don't sit in plain-text SQLite
/// by design; the use case is "I'm downloading one file from
/// a site that needs login", not "every future download uses
/// these credentials".
///
/// `headers` is a JSON object `{ "Header-Name": "value" }`. The
/// engine filters restricted headers (`Host`, `Content-Length`,
/// `Accept-Encoding`) and logs + drops them. `auth` is one of
/// `{ kind: "basic", username, password }`,
/// `{ kind: "bearer", token }`, or
/// `{ kind: "digest", username, password }` (downgraded to basic).
/// Pass `null` to clear.
#[tauri::command]
pub fn set_download_auth(
    state: State<'_, DownloadManager>,
    id: String,
    headers: std::collections::BTreeMap<String, String>,
    auth: Option<AuthSpec>,
) {
    state.set_headers_auth(&id, headers, auth);
}

/// Set the mirror list for a download. The engine tries `url`
/// first; on transient failure (4xx/5xx/timeout) it walks
/// `mirrors` in order and replaces the primary URL with the
/// first mirror that returns a successful probe. The new
/// primary is persisted and a `StatusChanged` event is
/// emitted so the UI updates without a full refresh.
///
/// Pass an empty vec to clear the mirror list.
#[tauri::command]
pub fn set_download_mirrors(
    state: State<'_, DownloadManager>,
    id: String,
    mirrors: Vec<String>,
) {
    state.set_mirrors(&id, mirrors);
}

/// Switch the global proxy policy. `mode` is `"none" | "system" | "manual"`.
/// `url` is only consulted when `mode == "manual"`.
#[tauri::command]
pub fn set_proxy(
    state: State<'_, DownloadManager>,
    mode: String,
    url: Option<String>,
) -> Result<Settings, String> {
    let parsed = match mode.to_ascii_lowercase().as_str() {
        "none" | "direct" | "off" => ProxyMode::None,
        "system" | "env" => ProxyMode::System,
        "manual" | "custom" => ProxyMode::Manual,
        other => return Err(format!("invalid proxy mode: {other}")),
    };
    state.set_global_proxy(parsed, url);
    Ok(state.settings())
}

#[tauri::command]
pub fn get_settings(state: State<'_, DownloadManager>) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn update_settings(
    state: State<'_, DownloadManager>,
    close_to_tray: State<'_, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    settings: Settings,
) {
    // Mirror `closeToTray` into the shared atomic that the
    // WindowEvent::CloseRequested handler reads. This is the
    // hook that makes the SettingsTabs toggle take effect
    // immediately for the *next* close attempt — without it, the
    // user would have to restart DM after toggling the checkbox.
    close_to_tray.store(settings.close_to_tray, std::sync::atomic::Ordering::Relaxed);
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
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}

/// Snapshot of the native-messaging host listener, used by the Settings →
/// Extensions tab to render a live status pill.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeHostProbe {
    pub bound: bool,
    pub port: u16,
    pub last_event_unix: u64,
}

#[tauri::command]
pub fn probe_native_host(
    state: tauri::State<'_, crate::native_host::NativeHostStatus>,
) -> NativeHostProbe {
    let (bound, last_event_unix) = state.snapshot();
    NativeHostProbe {
        bound,
        port: crate::native_host::DEFAULT_PORT,
        last_event_unix,
    }
}

/// Open the **containing folder** of `path` in the OS file manager.
///
/// Used by the "Open folder" entry in the download row's dropdown
/// menu. The path is the `Download.save_path` from the engine; the
/// file may not exist yet (the transfer is still in progress), so we
/// open the *parent* directory rather than the file itself. If
/// `select_file` is true **and** the file exists, the OS file manager
/// is asked to highlight the file (Finder → reveal, Explorer →
/// `/select,`, Nautilus → no equivalent, falls back to opening the
/// parent).
///
/// We deliberately do not pass the raw path through the `opener`
/// plugin's URL-parsing code path: it's a local filesystem path, not
/// a URL, and we want to fail loudly on missing parents rather than
/// silently open a remote URL by mistake.
#[tauri::command]
pub async fn open_folder(
    app: tauri::AppHandle,
    path: String,
    select_file: Option<bool>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    let p = std::path::PathBuf::from(&path);
    if !p.exists() {
        // Fall back to the parent so a half-written file (download
        // still in flight) still produces a useful open — the user's
        // folder will appear, even if the file isn't there yet.
        if let Some(parent) = p.parent() {
            if parent.as_os_str().is_empty() {
                return Err(format!("path does not exist and has no parent: {path}"));
            }
            if !parent.exists() {
                return Err(format!(
                    "neither the file nor its parent directory exists: {path}"
                ));
            }
            let parent_str = parent.to_string_lossy().into_owned();
            log::info!("open_folder: file {path} missing, opening parent {parent_str}");
            app.opener()
                .open_path(parent_str, None::<&str>)
                .map_err(|e| format!("opener failed: {e}"))?;
            return Ok(());
        }
        return Err(format!("path does not exist and has no parent: {path}"));
    }

    // The path itself is a directory: open it directly.
    if p.is_dir() {
        let s = p.to_string_lossy().into_owned();
        log::info!("open_folder: opening directory {s}");
        app.opener()
            .open_path(s, None::<&str>)
            .map_err(|e| format!("opener failed: {e}"))?;
        return Ok(());
    }

    // The path is an existing file. Either reveal it (select_file=true)
    // or open its parent directory.
    let want_select = select_file.unwrap_or(false);
    if want_select {
        let s = p.to_string_lossy().into_owned();
        log::info!("open_folder: revealing file {s}");
        // `reveal_item_in_dir` is the cross-platform "highlight this
        // file" call. On Linux without Nautilus' contract it falls
        // back to opening the parent, which is still useful.
        app.opener()
            .reveal_item_in_dir(s)
            .map_err(|e| format!("reveal failed: {e}"))?;
    } else if let Some(parent) = p.parent() {
        let parent_str = parent.to_string_lossy().into_owned();
        log::info!("open_folder: opening parent {parent_str} of {path}");
        app.opener()
            .open_path(parent_str, None::<&str>)
            .map_err(|e| format!("opener failed: {e}"))?;
    } else {
        // No parent? Open the file directly (shouldn't happen for
        // absolute paths, but harmless).
        let s = p.to_string_lossy().into_owned();
        app.opener()
            .open_path(s, None::<&str>)
            .map_err(|e| format!("opener failed: {e}"))?;
    }
    Ok(())
}
