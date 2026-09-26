//! Listens for URLs forwarded by the browser native-messaging host
//! (`dm-native-host`) and surfaces them to the user as a
//! confirmation dialog (`CaptureDialog` in the Svelte SPA).
//!
//! The previous implementation called `DownloadManager::add()`
//! directly when a URL arrived, which is why every click on a
//! download link immediately started transferring — the user had
//! no chance to confirm category, path, or filename. IDM, FDM and
//! similar managers pop a dialog for every captured URL; we now
//! match that flow:
//!
//!   1. The browser extension forwards a URL to DM's local TCP
//!      listener (`127.0.0.1:9157`).
//!   2. The listener emits a `Captured` event on the
//!      `download-event` channel. The payload includes the URL, a
//!      suggested filename (extracted from the URL path or
//!      `Content-Disposition` via `protocol::suggest_filename`),
//!      and the current default save directory.
//!   3. The frontend shows a modal (`CaptureDialog.svelte`) with
//!      editable filename, a category dropdown, and a save-path
//!      preview. Only when the user clicks "Download" does the
//!      frontend call the `add_download` Tauri command, which is
//!      the only path that actually creates the engine `Download`
//!      and starts the chunk workers.
//!
//! This module is intentionally lean: it does **not** hold an
//! `Arc<DownloadManager>` anymore. Engine state is touched only
//! through the `add_download` command (which is what the frontend
//! calls after the user confirms).

use dm_engine::protocol;
use crate::events::{CapturedUrl, FrontendEvent, EVENT_CHANNEL};
use crate::ws;
use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Emitter};

/// Port the native host connects to. Keep in sync with
/// `crates/native-host/src/main.rs::DEFAULT_PORT`.
pub const DEFAULT_PORT: u16 = 9157;

/// Shared state the frontend can poll via `probe_native_host` to see whether
/// the listener is up and when it last saw traffic.
#[derive(Clone)]
pub struct NativeHostStatus {
    pub bound: Arc<AtomicBool>,
    pub last_event_unix: Arc<AtomicU64>,
}

impl Default for NativeHostStatus {
    fn default() -> Self {
        NativeHostStatus {
            bound: Arc::new(AtomicBool::new(false)),
            last_event_unix: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl NativeHostStatus {
    pub fn snapshot(&self) -> (bool, u64) {
        (
            self.bound.load(Ordering::Relaxed),
            self.last_event_unix.load(Ordering::Relaxed),
        )
    }
    fn touch(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.last_event_unix.store(now, Ordering::Relaxed);
    }
}

/// Bind the listener on a background OS thread. Returns immediately; if the
/// port is unavailable the thread logs and exits without affecting the app.
///
/// `app` is the Tauri `AppHandle` used to emit `Captured` events to the
/// frontend. `default_save_dir` is read once at startup (the user's default
/// download location) and used as the suggested save directory for every
/// capture; the frontend can override it once the user picks a category.
pub fn start_native_host_listener(
    app: AppHandle,
    port: u16,
    default_save_dir: String,
    status: NativeHostStatus,
) {
    thread::spawn(move || {
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("dm: native-host listener bind failed on {port}: {e}");
                return;
            }
        };
        // Make accept() non-blocking so we can update `bound` promptly and
        // exit cleanly if the user later closes the app.
        listener.set_nonblocking(true).ok();
        status.bound.store(true, Ordering::Relaxed);
        eprintln!("dm: native-host listener on 127.0.0.1:{port}");
        loop {
            match listener.accept() {
                Ok((s, _)) => {
                    let app = app.clone();
                    let st = status.clone();
                    let dir = default_save_dir.clone();
                    // Peek the first byte to decide whether this
                    // is a WebSocket upgrade request. We wrap the
                    // raw socket in a BufReader first because the
                    // handshake path needs `BufRead`, and BufRead's
                    // `peek` is the cheap buffered way to look at
                    // incoming bytes without consuming them. A `GET`
                    // request starts with `G`; the legacy
                    // `dm-native-host` binary sends a raw JSON
                    // object (`{`).
                    let mut reader = BufReader::new(s);
                    let is_ws = match reader.fill_buf() {
                        Ok(buf) => buf.first().copied() == Some(b'G')
                            || buf.first().copied() == Some(b'g'),
                        Err(_) => false,
                    };
                    // `fill_buf` advances the internal cursor but
                    // not the underlying socket, so the bytes
                    // remain available to the reader we hand off.
                    if is_ws {
                        thread::spawn(move || handle_ws_conn(reader, app, dir, st));
                    } else {
                        thread::spawn(move || handle_conn(reader, app, dir, st));
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Idle; sleep briefly then re-check.
                    thread::sleep(Duration::from_millis(250));
                }
                Err(_) => {
                    // Transient error: brief backoff, then keep accepting.
                    thread::sleep(Duration::from_millis(500));
                }
            }
        }
    });
}

fn handle_conn(
    mut reader: BufReader<TcpStream>,
    app: AppHandle,
    default_save_dir: String,
    status: NativeHostStatus,
) {
    // Read line-delimited JSON until the client closes. The
    // legacy `dm-native-host` binary (Firefox path) and any
    // direct local-process integration use this format; the
    // WebSocket path is for browser extensions.
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            process_payload(&app, &status, &default_save_dir, &v);
        }
    }
}

fn handle_ws_conn(
    mut reader: BufReader<TcpStream>,
    app: AppHandle,
    default_save_dir: String,
    status: NativeHostStatus,
) {
    // The handshake is read+write on the same TCP stream. We
    // can write back through the BufReader because the inner
    // TcpStream implements Write; BufReader just delegates.
    // After the handshake we split into independent read/write
    // halves so the server-to-client ack frames don't
    // interfere with the in-flight frame read.
    let inner = reader.get_ref().try_clone();
    let mut writer = match inner {
        Ok(s) => s,
        Err(e) => {
            log::warn!("ws: failed to clone stream: {e}");
            return;
        }
    };
    match ws::handshake(&mut reader, &mut writer) {
        Ok(ws::HandshakeOutcome::WebSocket) => {
            // Handshake complete; switch to frame mode.
        }
        Ok(ws::HandshakeOutcome::NotWebSocket) => {
            // Not a WS upgrade (e.g. raw TCP probe that
            // started with `G` for some other reason). Drop
            // the connection; the caller already decided to
            // dispatch to the WS path so the legacy line
            // reader is not an option here.
            return;
        }
        Ok(ws::HandshakeOutcome::Invalid) | Err(_) => {
            // Malformed request. Drop the connection; the
            // client will see ECONNRESET.
            return;
        }
    }
    log::info!("ws: client connected");
    loop {
        match ws::read_text_frame(&mut reader) {
            Ok(Some(payload)) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&payload) {
                    process_payload(&app, &status, &default_save_dir, &v);
                }
                // Send a small ack so the browser knows the
                // payload was accepted. The client doesn't
                // strictly need it (it just retries on
                // disconnect), but a quick ack keeps the
                // connection warm through any idle window.
                let _ = ws::write_text_frame(&mut writer, r#"{"ok":true}"#);
            }
            Ok(None) => {
                // Clean close from the client.
                let _ = ws::write_close_frame(&mut writer);
                break;
            }
            Err(e) => {
                log::info!("ws: client disconnected: {e}");
                break;
            }
        }
    }
}

/// Turn the URLs a grab batch carries into the capture events the UI
/// should show — at most ONE dialog per grab (the IDM contract: one
/// grab, one download).
///
/// The extension already curates the list down to the one thing the
/// user is watching (see `collectMedia` in content.js), so what arrives
/// here is at most two URLs. When the batch carries `pair: true`, the
/// extension has already grouped the two halves by content id — honour
/// that directly instead of [`dm_engine::media::plan`], whose
/// same-origin check used to un-pair grabs whose halves came from
/// different mirrors (bilibili does this routinely: video from one
/// `upos-sz-*` host, audio from another) and reduced the grab to a lone
/// half. Otherwise `media::plan` decides:
///
///   * `Pair`     → one event carrying both halves in `pair_second`;
///   * `Single` / `Manifest` → one event for that URL;
///   * `Empty`    → nothing (only fragments / unknowns remained; the
///     extension's note already explains why nothing was offered).
///
/// Returns `(url, pair_second)` tuples.
fn plan_captures(urls: &[String], pair_flag: bool) -> Vec<(String, Option<String>)> {
    if pair_flag {
        if urls.len() >= 2 {
            return vec![(urls[0].clone(), Some(urls[1].clone()))];
        }
        if urls.len() == 1 {
            return vec![(urls[0].clone(), None)];
        }
        return Vec::new();
    }
    match dm_engine::media::plan(urls) {
        dm_engine::media::MediaPlan::Pair { first, second } => vec![(first, Some(second))],
        dm_engine::media::MediaPlan::Single { url, .. } | dm_engine::media::MediaPlan::Manifest { url } => {
            vec![(url, None)]
        }
        dm_engine::media::MediaPlan::Empty => Vec::new(),
    }
}

/// Parse one captured-URL payload (the same shape for both
/// the WS path and the legacy line-delimited path) and emit
/// the appropriate `Captured` frontend event(s).
fn process_payload(
    app: &AppHandle,
    status: &NativeHostStatus,
    default_save_dir: &str,
    v: &serde_json::Value,
) {
    // Three shapes we accept:
    //   {"url":"…","type":"download"|"save-as"|"grab"|"click"}
    //     – the dm-native-host binary's forward_url path;
    //   {"url":"…"}
    //     – the legacy / direct-socket form (kept for back-compat);
    //   {"urls":["…","…"]}
    //     – grab batches; grouped into at most one capture each via
    //       `media::plan` (a DASH pair becomes ONE dialog, not two).
    let kind = v
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("download");
    let source = match kind {
        "click" | "download-click" => "browser-click",
        "save-as" => "browser-save-as",
        "grab" | "capture" => "browser-grab",
        _ => "native-host",
    };
    let referer = v.get("referer").and_then(|r| r.as_str());
    let ua = v.get("userAgent").and_then(|r| r.as_str());
    let pair_flag = v.get("pair").and_then(|p| p.as_bool()).unwrap_or(false);
    let meta = v.get("meta");
    let meta_title = page_meta_field(meta, "title");
    let meta_date = page_meta_field(meta, "date");
    let meta_artist = page_meta_field(meta, "artist");
    if let Some(arr) = v.get("urls").and_then(|u| u.as_array()) {
        let urls: Vec<String> = arr
            .iter()
            .filter_map(|u| u.as_str())
            .map(|s| s.to_string())
            .collect();
        for (url, pair_second) in plan_captures(&urls, pair_flag) {
            status.touch();
            emit_captured(
                app,
                source,
                &url,
                pair_second,
                default_save_dir,
                referer,
                ua,
                meta_title.clone(),
                meta_date.clone(),
                meta_artist.clone(),
            );
        }
    } else if let Some(url) = v.get("url").and_then(|u| u.as_str()) {
        status.touch();
        emit_captured(
            app,
            source,
            url,
            None,
            default_save_dir,
            referer,
            ua,
            meta_title,
            meta_date,
            meta_artist,
        );
    }
}

/// Read a string field out of the batch's `meta` object (the page
/// metadata the extension harvested). `None` for anything missing or
/// non-string.
fn page_meta_field(meta: Option<&serde_json::Value>, field: &str) -> Option<String> {
    meta?
        .get(field)?
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// For a pair capture, the dialog's filename describes the MERGED file,
/// not the `.m4s` halves — replace the segment extension with the
/// container the remux produces.
fn pair_suggested_filename(first_url: &str) -> String {
    let raw = protocol::suggest_filename(first_url, None, "download.bin");
    let with_ext = std::path::Path::new(&raw)
        .with_extension("mkv")
        .to_string_lossy()
        .to_string();
    if with_ext.is_empty() || with_ext == ".mkv" {
        "video.mkv".to_string()
    } else {
        with_ext
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_captured(
    app: &AppHandle,
    source: &str,
    url: &str,
    pair_second: Option<String>,
    default_save_dir: &str,
    referer: Option<&str>,
    user_agent: Option<&str>,
    meta_title: Option<String>,
    meta_date: Option<String>,
    meta_artist: Option<String>,
) {
    // Best-effort filename extraction. The engine has a richer
    // `protocol::suggest_filename` that prefers `Content-Disposition`
    // over the URL path, but we don't have the response headers
    // here (the URL was forwarded by the native host, not
    // downloaded by us). Use the URL-path variant for now; the
    // `add_download` Tauri command can refine it later if the
    // user clicks "Download" — the frontend can also re-suggest
    // a filename once the engine returns the real Content-Type.
    let suggested_filename = match &pair_second {
        Some(_) => pair_suggested_filename(url),
        None => protocol::suggest_filename(url, None, "download.bin"),
    };
    let nonce = format!("{}-{}", Instant::now().elapsed().as_nanos(), url);
    let payload = CapturedUrl {
        source: source.to_string(),
        url: url.to_string(),
        pair_second,
        meta_title: meta_title.filter(|s| !s.is_empty()),
        meta_date: meta_date.filter(|s| !s.is_empty()),
        meta_artist: meta_artist.filter(|s| !s.is_empty()),
        suggested_filename,
        default_save_dir: default_save_dir.to_string(),
        // The browser extension can forward the source page's
        // Referer and the browser's current User-Agent string.
        // We surface them as `Option`s so the frontend can
        // pre-fill the per-download headers (or skip the row
        // entirely if the value is empty / absent). This is
        // the Tier-1 "Referer / user-agent per download" hook:
        // a user clicking a download link on a paywalled page
        // usually has the page's Referer set, and the server
        // rejects requests without it. Capturing it at the
        // browser level is the cleanest path.
        referer: referer.map(|s| s.to_string()).filter(|s| !s.is_empty()),
        user_agent: user_agent.map(|s| s.to_string()).filter(|s| !s.is_empty()),
        nonce,
    };
    let event: FrontendEvent = FrontendEvent::Captured(payload);
    if let Err(e) = app.emit(EVENT_CHANNEL, &event) {
        eprintln!("dm: failed to emit Captured event: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::plan_captures;

    /// A DASH grab batch (the two stream bases) becomes ONE capture
    /// carrying both halves — one dialog, one logical download.
    #[test]
    fn dash_pair_becomes_a_single_paired_capture() {
        let urls = vec![
            "https://upos.example.test/12/34/56/123456-1-30280.m4s".to_string(),
            "https://upos.example.test/12/34/56/123456-1-30232.m4s".to_string(),
        ];
        assert_eq!(
            plan_captures(&urls, false),
            vec![(urls[0].clone(), Some(urls[1].clone()))]
        );
    }

    /// The extension groups halves by content id and flags the batch.
    /// The flag must win over `media::plan`'s same-origin heuristic —
    /// bilibili serves the two halves from different mirrors, and
    /// trusting the heuristic reduced grabs to a lone half.
    #[test]
    fn pair_flag_pairs_across_mirrors() {
        let urls = vec![
            "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-100022.m4s?upsign=v".to_string(),
            "https://upos-sz-mirrorkb.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-30216.m4s?upsign=a".to_string(),
        ];
        assert_eq!(
            plan_captures(&urls, true),
            vec![(urls[0].clone(), Some(urls[1].clone()))]
        );
    }

    /// A direct media file stays a single unpaired capture.
    #[test]
    fn single_file_stays_unpaired() {
        let urls = vec!["https://cdn.example.test/clip.mp4".to_string()];
        assert_eq!(plan_captures(&urls, false), vec![(urls[0].clone(), None)]);
    }

    /// Fragments and unknowns are not offered — the extension's note
    /// explains the decision, DM's queue stays clean.
    #[test]
    fn fragments_and_unknowns_offer_nothing() {
        assert!(
            plan_captures(&["https://cdn.example.test/hls/0.ts".to_string()], false).is_empty()
        );
        assert!(plan_captures(&["https://cdn.example.test/page".to_string()], false).is_empty());
        assert!(plan_captures(&[], false).is_empty());
    }

    /// A pair capture's suggested filename describes the merged file,
    /// not the `.m4s` half it was derived from.
    #[test]
    fn pair_filename_names_the_merged_file() {
        let name = super::pair_suggested_filename(
            "https://upos.example.test/12/34/56/123456-1-30280.m4s",
        );
        assert_eq!(name, "123456-1-30280.mkv");
    }
}
