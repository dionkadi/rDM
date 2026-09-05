//! Tauri command surface. Each command delegates to the `DownloadManager`
//! held in managed state.

use dm_engine::manager::DownloadManager;
use dm_engine::model::{AuthSpec, ChecksumSpec, Download, DownloadStatus, ProxyMode, Settings};
use dm_engine::protocol;
use dm_engine::cookies;
use tauri::State;
use tauri_plugin_notification::NotificationExt;

/// Health-check command used by the scaffold to verify the Rust<->JS bridge.
#[tauri::command]
pub fn ping() -> String {
    format!("pong @ {}", chrono::Utc::now().to_rfc3339())
}

/// Static build / runtime info for the Settings → About panel.
///
/// Previously the frontend hard-coded "0.1.0" in two places, which
/// went stale as soon as the first version bump landed. The
/// `app_info` Tauri command is the single source of truth: the
/// values come from `env!("CARGO_PKG_VERSION")` at compile time
/// (so they're baked into the binary the user actually runs, not
/// read from a config file the user could edit).
///
/// All three fields are returned as `String` so the frontend can
/// render them without any conversion. `engine_version` is read
/// from `dm_engine::VERSION` (which mirrors
/// `crates/engine/Cargo.toml`) so the engine and the wrapper
/// app are guaranteed to report the same version, and a single
/// `scripts/release.sh` bump keeps them in sync.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub app_version: String,
    pub engine_version: String,
    /// Tauri runtime version. Useful for diagnosing
    /// version-specific Tauri bugs in user reports.
    pub tauri_version: String,
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        engine_version: dm_engine::VERSION.to_string(),
        tauri_version: tauri::VERSION.to_string(),
    }
}

/// Enqueue a new download. Returns the created `Download` (status `Queued`).
///
/// Optional `headers` and `auth` arguments attach per-download
/// auth data to the freshly-created row. They are in-memory
/// only (not persisted to SQLite) so a restart clears them —
/// auth secrets don't sit in a plain-text database file, and
/// the use case is "I'm downloading one file from a site that
/// needs login", not "every future download uses these
/// credentials". The `add_download` command wires the auth
/// data through the same `set_headers_auth` path that the
/// `AuthDialog` uses after creation.
#[tauri::command]
pub fn add_download(
    state: State<'_, DownloadManager>,
    url: String,
    category: Option<String>,
    filename: Option<String>,
    speed_limit: Option<u64>,
    checksum: Option<ChecksumSpec>,
    headers: Option<std::collections::BTreeMap<String, String>>,
    auth: Option<AuthSpec>,
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
    // Headers and auth are in-memory only (see the
    // `DownloadManager::set_headers_auth` doc). We attach
    // them after the row is in the map so the per-download
    // `state.set_headers_auth(id, …)` call is a simple
    // lookup, not a fresh insertion.
    if headers.is_some() || auth.is_some() {
        state.set_headers_auth(&d.id, headers.unwrap_or_default(), auth);
    }
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

/// Result of a `import_browser_cookies` Tauri command. Mirrors
/// `dm_engine::cookies::BrowserCookie` for the frontend (the
/// engine type is `pub` but the frontend shouldn't depend on
/// the engine crate). `camelCase` on the wire so it round-trips
/// with the `serde(rename_all = "camelCase")` derive on the
/// engine side.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserCookieDto {
    pub name: String,
    pub value: String,
    pub host: String,
    pub path: String,
    pub secure: bool,
    pub expires_unix: Option<i64>,
}

/// Read cookies from a browser's local SQLite database. Used by
/// the per-download auth dialog to populate a `Cookie:` header
/// from the user's existing browser session.
///
/// `kind` is `"firefox"` or `"chromium"`. The engine picks the
/// default on-disk path for the current OS; `path_override`
/// lets the user pick a custom file (e.g. a snap install or a
/// portable browser). `host` filters by suffix match — pass
/// the URL's host (e.g. `"example.com"`) to get only the
/// cookies that apply. Pass an empty string for the full list.
///
/// Errors are surfaced verbatim as a Tauri command error so
/// the frontend can show the exact reason in a toast (e.g.
/// "Chromium cookies on macOS are encrypted — see the docs").
#[tauri::command]
pub fn import_browser_cookies(
    kind: String,
    host: Option<String>,
    path_override: Option<String>,
) -> Result<CookieImportResult, String> {
    let parsed_kind = match kind.to_ascii_lowercase().as_str() {
        "firefox" | "ff" => cookies::BrowserKind::Firefox,
        "chromium" | "chrome" | "edge" | "brave" => cookies::BrowserKind::Chromium,
        other => return Err(format!("unknown browser kind: {other}")),
    };
    let host_filter = host
        .as_deref()
        .filter(|s| !s.is_empty());
    let path = path_override
        .as_deref()
        .map(std::path::Path::new);
    let cookies = cookies::read_browser_cookies(
        parsed_kind,
        host_filter,
        path,
    )
    .map_err(|e| e.to_string())?;
    let header = cookies::format_cookie_header(&cookies);
    Ok(CookieImportResult {
        count: cookies.len(),
        header,
        cookies: cookies
            .into_iter()
            .map(|c| BrowserCookieDto {
                name: c.name,
                value: c.value,
                host: c.host,
                path: c.path,
                secure: c.secure,
                expires_unix: c.expires_unix,
            })
            .collect(),
    })
}

/// What the cookie-import command returns: the full
/// `Cookie:` header value (the user can apply it as-is via the
/// per-download headers) plus the per-cookie breakdown (so the
/// frontend can show a checklist and let the user toggle each
/// cookie individually before applying).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CookieImportResult {
    /// Number of cookies that matched the filter.
    pub count: usize,
    /// Pre-formatted `Cookie:` header value, ready to paste
    /// into the per-download headers dialog.
    pub header: String,
    /// Per-cookie detail, for the "pick which cookies to send"
    /// picker UI.
    pub cookies: Vec<BrowserCookieDto>,
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

/// Move a download's on-disk file (and its `.part` sibling if any)
/// to the OS trash, then drop the in-memory entry and the SQLite
/// row.
///
/// This is the command the "Trash" entry in the download row's
/// dropdown menu calls. It is the recoverable counterpart to
/// `remove_download` (which only deletes the SQLite row — the
/// on-disk file is left where the engine put it). Using the OS
/// trash means the user can undelete via Finder / Explorer /
/// Files if they click Trash by accident; the SQLite row is the
/// irreversible half (it must be gone before the next `start()`
/// re-loads active downloads, otherwise the row reappears with
/// no file to back it).
///
/// We try to trash **both** the final filename and the `.part`
/// file (if present) in a single `trash::delete_all` call. The
/// `.part` file is the partial bytes from an interrupted
/// download; trashing it cleans up the directory as a side
/// effect. A missing `.part` is not an error (the user is
/// trashing a *completed* download and the file was already
/// renamed to its final name by `task.rs::run_download`).
///
/// On platforms where the trash crate cannot reach the OS
/// trash (rare, but the libcanberra / dbus stack on stripped
/// Linux images sometimes refuses), the trash call returns an
/// error and the in-memory state is left intact — we
/// deliberately do **not** fall back to `std::fs::remove_file`
/// because that would bypass the user's recoverable-delete
/// intent. The frontend should show a clear "trash failed"
/// toast and offer a "Force delete" follow-up.
#[tauri::command]
pub fn trash_download(
    state: State<'_, DownloadManager>,
    id: String,
) -> Result<(), String> {
    // 1. Resolve the on-disk paths from the in-memory state. We
    //    snapshot both the final filename and the `.part` path so
    //    the trash call below doesn't race with the chunk
    //    workers' writes (it shouldn't — the download is
    //    either completed or already errored, so the chunk
    //    workers are not active — but defending against a
    //    delete-while-transferring race is cheap).
    let (save_path, part_path) = {
        let Some(d) = state.get(&id) else {
            return Err(format!("download not found: {id}"));
        };
        (d.save_path.clone(), d.part_path())
    };

    // 2. Build the list of paths to trash. We include the
    //    `.part` file only if it exists (its absence is not an
    //    error — a completed download has already been renamed
    //    to its final filename by `task.rs::run_download`).
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    if save_path.exists() {
        paths.push(save_path.clone());
    }
    if part_path.exists() {
        paths.push(part_path.clone());
    }
    if paths.is_empty() {
        // Nothing on disk to trash. That's fine — we still
        // drop the in-memory + SQLite row. (The file may have
        // been deleted out from under us by another process
        // between when the user added the download and now.)
        log::info!(
            "trash_download: no on-disk file for id={id} (path={}); \
             dropping the database row only",
            save_path.display()
        );
    } else {
        // 3. Trash. `trash::delete_all` is the cross-platform
        //    "move to Recycle Bin / Trash" call: macOS Finder
        //    Trash, Linux XDG Trash, Windows Recycle Bin. It
        //    does **not** follow symlinks (the link is removed
        //    and the target is kept intact) — the right
        //    behaviour for a download manager.
        trash::delete_all(&paths).map_err(|e| {
            format!(
                "failed to move to trash: {e} (file kept on disk; \
                 engine state untouched — the user can retry or force-delete)"
            )
        })?;
        log::info!(
            "trash_download: trashed {} path(s) for id={id}: {:?}",
            paths.len(),
            paths
        );
    }

    // 4. Drop the in-memory entry and the SQLite row, and emit
    //    a `Removed` event. The same path `remove()` takes,
    //    including the deliberate skip of `cancel()` (see the
    //    long comment in `DownloadManager::remove` for why
    //    this matters).
    state.remove(&id);
    Ok(())
}

/// Open **the file itself** (not its parent) in the OS's default
/// handler.
///
/// This is the command the "Open" entry in the download row's
/// dropdown menu calls. It is distinct from `open_folder`, which
/// opens the parent directory. If the file does not exist
/// (the transfer is still in flight and the `.part` has not been
/// renamed yet), we surface a clear error so the frontend can
/// tell the user to wait for the download to finish.
#[tauri::command]
pub async fn open_file(
    app: tauri::AppHandle,
    path: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    let p = std::path::PathBuf::from(&path);
    if !p.exists() {
        return Err(format!(
            "file does not exist (still downloading?): {path}"
        ));
    }
    if p.is_dir() {
        // Defensive: opening a directory in the OS default
        // handler does open it in the file manager, but the
        // user-facing "Open" entry in the row is meant for
        // files. Surface a clear error rather than silently
        // redirecting.
        return Err(format!("path is a directory, not a file: {path}"));
    }
    let s = p.to_string_lossy().into_owned();
    log::info!("open_file: opening {s}");
    app.opener()
        .open_path(s, None::<&str>)
        .map_err(|e| format!("opener failed: {e}"))?;
    Ok(())
}

/// Copy a text string to the OS clipboard.
///
/// Used by the "Copy path" entry in the download row's dropdown
/// menu. We route through the Tauri command surface (rather than
/// calling `navigator.clipboard.writeText` directly in the
/// webview) because some webview configurations refuse clipboard
/// writes outside a user gesture, and the Tauri side does not
/// have that restriction. The text is what the frontend hands us
/// — the Rust side does not interpret it.
#[tauri::command]
pub async fn copy_text(
    app: tauri::AppHandle,
    text: String,
) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .write_text(text)
        .map_err(|e| format!("clipboard write failed: {e}"))?;
    Ok(())
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
