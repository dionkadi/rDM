//! Core data model shared between the engine and the (Tauri) frontend.
//! All types are `Serialize` so they can be sent across the Tauri bridge.

use chrono::{DateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Stable identifier for a download (UUID v4 string).
pub type DownloadId = String;

/// Lifecycle state of a single download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    /// Waiting for a free download slot.
    Queued,
    /// Probing the remote (HEAD / range probe).
    Connecting,
    /// Actively transferring bytes.
    Downloading,
    /// User paused; can be resumed.
    Paused,
    /// Finished successfully.
    Completed,
    /// Failed (see `Download.error`).
    Error,
    /// User cancelled.
    Canceled,
    /// Waiting for a scheduled start time.
    Scheduled,
}

/// One contiguous byte range of a download. For a segmented download there are
/// `connections_per_download` chunks; a single-connection fallback has one chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkState {
    /// 0-based index within the download.
    pub index: usize,
    /// Inclusive start byte offset of this chunk's *full* range.
    pub start: u64,
    /// Inclusive end byte offset of this chunk's *full* range.
    pub end: u64,
    /// Bytes already retrieved within this chunk (drives resume).
    pub downloaded: u64,
}

impl ChunkState {
    /// Total bytes in the chunk's full range (end is inclusive).
    /// Returns `u64::MAX` for the open-ended chunk (`end == u64::MAX`)
    /// since the resource has no known total length.
    pub fn size(&self) -> u64 {
        if self.end == u64::MAX {
            return u64::MAX;
        }
        self.end.saturating_sub(self.start) + 1
    }

    /// Bytes still missing in this chunk.
    pub fn remaining(&self) -> u64 {
        self.size().saturating_sub(self.downloaded)
    }

    /// Absolute file offset where the already-downloaded portion ends.
    pub fn resume_offset(&self) -> u64 {
        self.start + self.downloaded
    }
}

/// Optional integrity check to run after the file is fully written.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChecksumSpec {
    /// Algorithm name understood by the engine (currently `sha256`).
    pub algorithm: String,
    /// Expected hex digest (lowercase).
    pub expected: String,
}

/// A single managed download.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    pub id: DownloadId,
    pub url: String,
    pub filename: String,
    pub save_path: PathBuf,
    /// `None` until the remote size is known.
    pub total_size: Option<u64>,
    /// Total bytes written across all chunks.
    pub downloaded: u64,
    pub status: DownloadStatus,
    pub category: Option<String>,
    pub content_type: Option<String>,
    pub chunks: Vec<ChunkState>,
    /// Per-download speed cap in bytes/sec (`None` = inherit global).
    pub speed_limit: Option<u64>,
    /// Per-download proxy override (`None` = use global / direct).
    pub proxy: Option<String>,
    pub checksum: Option<ChecksumSpec>,
    /// Set when `status == Error`.
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// Whether the server advertised range support (drive resume/segmentation).
    pub can_resume: bool,
    /// User-controlled queue position. Lower values run first; the
    /// scheduler picks the next-eligible download with the smallest
    /// `sort_key` (then `created_at` as a tie-breaker). Reorders
    /// space new keys by 1000 to leave room for interleaving.
    #[serde(default)]
    pub sort_key: i64,
    /// User priority: `0` = low, `1` = normal (default), `2` = high.
    /// Higher-priority downloads run before lower-priority ones when
    /// the slot budget is full and a new slot opens.
    #[serde(default = "default_priority")]
    pub priority: u8,
    /// Per-download HTTP headers to add to every request. The
    /// keys are header names (case-insensitive on the wire, but
    /// stored exactly as the user typed them), the values are
    /// the raw header values. Use this for `Referer`, custom
    /// `User-Agent`, `Authorization: Bearer …`, etc.
    ///
    /// Restricted headers (`Host`, `Content-Length`,
    /// `Accept-Encoding`) are filtered out at the engine
    /// boundary — reqwest refuses to set them anyway, and we
    /// want a clear error rather than a silent drop.
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
    /// Optional HTTP authentication. The engine uses these
    /// credentials to add an `Authorization` header to every
    /// request; if the server returns 401 and the
    /// `Authorization` header was already present, the request
    /// is retried with the `Bearer` scheme (i.e. it converts
    /// to a bearer token without the user having to do
    /// anything).
    #[serde(default)]
    pub auth: Option<AuthSpec>,
    /// Mirror URLs to fall back to when the primary URL fails.
    /// The engine tries `url` first; on transient failure
    /// (4xx/5xx/timeout), it moves to `mirrors[0]`, then
    /// `mirrors[1]`, and so on. The first mirror to return a
    /// successful probe becomes the new "primary" for the
    /// remainder of the transfer; subsequent chunks reuse it.
    /// Empty by default.
    #[serde(default)]
    pub mirrors: Vec<String>,
    /// Optional media manifest hint. When the URL points at an
    /// HLS `.m3u8` or DASH `.mpd` manifest, the engine's task
    /// layer can pick the right downloader. `None` means
    /// "plain HTTP" (the default path). For v1 this is a
    /// marker only — a full HLS/DASH implementation (segment
    /// fetching, retry, optional ffmpeg remux) is a separate
    /// piece of work tracked in TODO.md.
    #[serde(default)]
    pub media: Option<MediaKind>,
}

/// HLS / DASH manifest kinds. The presence of `Some(kind)`
/// tells the engine to dispatch to a media-specific downloader
/// instead of the plain HTTP path. For v1 we only mark the
/// kind; actual segment fetching is a follow-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    /// HTTP Live Streaming manifest (`.m3u8`).
    Hls,
    /// MPEG-DASH manifest (`.mpd`).
    Dash,
}

/// Per-download authentication.
///
/// `Basic` sends `Authorization: Basic base64(user:pass)`.
/// `Bearer` sends `Authorization: Bearer <token>`.
/// `Digest` is a future-proofing placeholder; the engine
/// currently downgrades to `Basic` and surfaces a clear
/// error in the download's `error` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AuthSpec {
    Basic { username: String, password: String },
    Bearer { token: String },
    Digest { username: String, password: String },
}

fn default_priority() -> u8 {
    1
}

/// Symbolic priority constants. The wire format stays a small
/// integer so adding new tiers doesn't require a protocol bump, but
/// the constants are public so the rest of the engine (scheduler,
/// Tauri command surface, frontend) refers to a name.
pub const PRIORITY_LOW: u8 = 0;
pub const PRIORITY_NORMAL: u8 = 1;
pub const PRIORITY_HIGH: u8 = 2;

impl Download {
    /// Create a fresh, queued download for `url` with a generated id.
    pub fn new(url: impl Into<String>) -> Self {
        Download {
            id: Uuid::new_v4().to_string(),
            url: url.into(),
            filename: String::new(),
            save_path: PathBuf::from("."),
            total_size: None,
            downloaded: 0,
            status: DownloadStatus::Queued,
            category: None,
            content_type: None,
            chunks: Vec::new(),
            speed_limit: None,
            proxy: None,
            checksum: None,
            error: None,
            created_at: Utc::now(),
            finished_at: None,
            can_resume: false,
            sort_key: 0,
            priority: default_priority(),
            headers: std::collections::BTreeMap::new(),
            auth: None,
            mirrors: Vec::new(),
            media: None,
        }
    }

    /// Completion fraction in `[0.0, 1.0]`; `0.0` when size unknown.
    pub fn fraction(&self) -> f64 {
        match self.total_size {
            Some(total) if total > 0 => (self.downloaded as f64 / total as f64).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    /// `true` once every byte is accounted for.
    pub fn is_complete(&self) -> bool {
        matches!(self.total_size, Some(t) if t > 0 && t == self.downloaded)
    }
}

/// Category that auto-routes downloads by file extension to a folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub name: String,
    /// Lower-case extensions without the dot, e.g. `["mp4", "mkv"]`.
    pub extensions: Vec<String>,
    pub directory: PathBuf,
}

/// Proxy policy for downloads. Serialised as a camelCase string on the wire
/// so the frontend can drive it directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProxyMode {
    /// Direct connection — no proxy, regardless of environment variables.
    None,
    /// Honour the standard `HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY` env vars
    /// (lower-case accepted on Windows). reqwest reads these directly.
    System,
    /// Use the `proxy` URL configured in settings (or per-download override).
    Manual,
}

impl Default for ProxyMode {
    fn default() -> Self {
        ProxyMode::System
    }
}

/// Global + per-download engine settings (persisted to disk).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Max simultaneous downloads.
    pub max_concurrent_downloads: usize,
    /// Connections used per download (segmentation factor).
    pub connections_per_download: usize,
    /// Default save directory.
    pub default_directory: PathBuf,
    /// Global speed cap in bytes/sec (`None` = unlimited).
    pub speed_limit_global: Option<u64>,
    pub categories: Vec<Category>,
    /// Proxy policy (None / System / Manual). `None` here means "no proxy ever";
    /// this is independent of the legacy `proxy: Option<String>` field, which
    /// carries the Manual URL.
    #[serde(default)]
    pub proxy_mode: ProxyMode,
    /// Manual proxy URL (`http://…`, `https://…`, `socks5://…`). Used when
    /// `proxy_mode == Manual`, and also kept for backwards compatibility so
    /// older clients / configs that only set this still get a proxy.
    pub proxy: Option<String>,
    pub clipboard_monitor: bool,
    pub close_to_tray: bool,
    /// Allowed schedule window (24h). `None` means always allowed.
    pub schedule_enabled: bool,
    pub schedule_start: (u8, u8),
    pub schedule_end: (u8, u8),
}

impl Settings {
    /// Resolve the effective global proxy URL to use, given the policy and the
    /// current process environment. `None` means "connect directly".
    pub fn effective_proxy_url(&self) -> Option<String> {
        match self.proxy_mode {
            ProxyMode::None => None,
            ProxyMode::System => std::env::var("HTTPS_PROXY")
                .or_else(|_| std::env::var("https_proxy"))
                .or_else(|_| std::env::var("HTTP_PROXY"))
                .or_else(|_| std::env::var("http_proxy"))
                .ok()
                .filter(|s| !s.is_empty()),
            ProxyMode::Manual => self.proxy.clone().filter(|s| !s.is_empty()),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        let default_directory = directories::UserDirs::new()
            .map(|u| u.download_dir().unwrap_or_else(|| u.home_dir()).to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Settings {
            max_concurrent_downloads: 3,
            connections_per_download: 8,
            default_directory,
            speed_limit_global: None,
            categories: Vec::new(),
            proxy_mode: ProxyMode::default(),
            proxy: None,
            clipboard_monitor: true,
            close_to_tray: true,
            schedule_enabled: false,
            schedule_start: (0, 0),
            schedule_end: (23, 59),
        }
    }
}

impl Settings {
    /// `true` when downloads are currently allowed to run.
    ///
    /// When scheduling is disabled this is always `true`. Otherwise the current
    /// time (UTC) is tested against the inclusive `schedule_start`..=`schedule_end`
    /// window, which may wrap past midnight (e.g. `22:00`–`06:00`).
    pub fn in_schedule_window(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        if !self.schedule_enabled {
            return true;
        }
        let now_min = now.hour() as u32 * 60 + now.minute() as u32;
        let start_min = self.schedule_start.0 as u32 * 60 + self.schedule_start.1 as u32;
        let end_min = self.schedule_end.0 as u32 * 60 + self.schedule_end.1 as u32;
        if start_min <= end_min {
            now_min >= start_min && now_min <= end_min
        } else {
            // Window wraps past midnight.
            now_min >= start_min || now_min <= end_min
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u8, m: u8) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .with_ymd_and_hms(2024, 1, 1, h as u32, m as u32, 0)
            .unwrap()
    }

    #[test]
    fn window_always_open_when_disabled() {
        let mut s = Settings::default();
        s.schedule_enabled = false;
        assert!(s.in_schedule_window(at(3, 14)));
    }

    #[test]
    fn window_simple_range() {
        let mut s = Settings::default();
        s.schedule_enabled = true;
        s.schedule_start = (9, 0);
        s.schedule_end = (17, 0);
        assert!(s.in_schedule_window(at(12, 0)));
        assert!(!s.in_schedule_window(at(8, 59)));
        assert!(!s.in_schedule_window(at(17, 1)));
    }

    #[test]
    fn window_wraps_midnight() {
        let mut s = Settings::default();
        s.schedule_enabled = true;
        s.schedule_start = (22, 0);
        s.schedule_end = (6, 0);
        assert!(s.in_schedule_window(at(23, 0)));
        assert!(s.in_schedule_window(at(0, 0)));
        assert!(s.in_schedule_window(at(5, 59)));
        assert!(!s.in_schedule_window(at(12, 0)));
        assert!(!s.in_schedule_window(at(21, 59)));
    }

    // ── Proxy resolution ───────────────────────────────────────

    /// Tests that touch the process-wide proxy env vars must run under this
    /// lock — Cargo runs tests in parallel and a sibling test clearing the
    /// env between our `set_var` and our `assert_eq` causes spurious
    /// failures.
    static PROXY_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_clean_env<F: FnOnce()>(f: F) {
        // Snapshot and clear the proxy env vars so tests don't leak into each other.
        let _guard = PROXY_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let keys = [
            "HTTP_PROXY", "http_proxy",
            "HTTPS_PROXY", "https_proxy",
            "ALL_PROXY", "all_proxy",
            "NO_PROXY", "no_proxy",
        ];
        let saved: Vec<(&str, Option<String>)> = keys
            .iter()
            .map(|k| (*k, std::env::var(k).ok()))
            .collect();
        for k in keys {
            std::env::remove_var(k);
        }
        f();
        for (k, v) in saved {
            if let Some(v) = v {
                std::env::set_var(k, v);
            } else {
                std::env::remove_var(k);
            }
        }
    }

    #[test]
    fn proxy_mode_none_never_returns_a_url() {
        with_clean_env(|| {
            std::env::set_var("HTTPS_PROXY", "http://from-env:8080");
            let mut s = Settings::default();
            s.proxy_mode = ProxyMode::None;
            s.proxy = Some("http://manual:3128".into());
            assert!(s.effective_proxy_url().is_none());
        });
    }

    #[test]
    fn proxy_mode_system_reads_https_proxy_first() {
        with_clean_env(|| {
            std::env::set_var("HTTP_PROXY", "http://from-env-http:8080");
            std::env::set_var("HTTPS_PROXY", "http://from-env-https:8443");
            let s = Settings {
                proxy_mode: ProxyMode::System,
                ..Settings::default()
            };
            assert_eq!(
                s.effective_proxy_url().as_deref(),
                Some("http://from-env-https:8443")
            );
        });
    }

    #[test]
    fn proxy_mode_system_falls_back_to_http_proxy() {
        with_clean_env(|| {
            std::env::set_var("HTTP_PROXY", "http://from-env-http:8080");
            let s = Settings {
                proxy_mode: ProxyMode::System,
                ..Settings::default()
            };
            assert_eq!(
                s.effective_proxy_url().as_deref(),
                Some("http://from-env-http:8080")
            );
        });
    }

    #[test]
    fn proxy_mode_system_no_env_means_direct() {
        with_clean_env(|| {
            let s = Settings {
                proxy_mode: ProxyMode::System,
                ..Settings::default()
            };
            assert!(s.effective_proxy_url().is_none());
        });
    }

    #[test]
    fn proxy_mode_manual_uses_settings_url() {
        let mut s = Settings::default();
        s.proxy_mode = ProxyMode::Manual;
        s.proxy = Some("socks5://localhost:1080".into());
        assert_eq!(
            s.effective_proxy_url().as_deref(),
            Some("socks5://localhost:1080")
        );
    }

    #[test]
    fn proxy_mode_manual_with_empty_url_falls_back_to_direct() {
        let mut s = Settings::default();
        s.proxy_mode = ProxyMode::Manual;
        s.proxy = Some("".into());
        assert!(s.effective_proxy_url().is_none());
    }

    #[test]
    fn settings_roundtrip_preserves_proxy_mode() {
        let mut s = Settings::default();
        s.proxy_mode = ProxyMode::Manual;
        s.proxy = Some("http://manual:3128".into());
        let json = serde_json::to_string(&s).unwrap();
        // camelCase on the wire
        assert!(json.contains("\"proxyMode\":\"manual\""));
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.proxy_mode, ProxyMode::Manual);
        assert_eq!(back.proxy.as_deref(), Some("http://manual:3128"));
    }

    #[test]
    fn settings_old_json_without_proxy_mode_defaults_to_system() {
        // Simulate a row written by an older build (no `proxyMode` field).
        let old = r#"{
            "maxConcurrentDownloads": 3,
            "connectionsPerDownload": 8,
            "defaultDirectory": "/tmp",
            "speedLimitGlobal": null,
            "categories": [],
            "proxy": null,
            "clipboardMonitor": true,
            "closeToTray": true,
            "scheduleEnabled": false,
            "scheduleStart": [0, 0],
            "scheduleEnd": [23, 59]
        }"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(s.proxy_mode, ProxyMode::System);
    }
}
