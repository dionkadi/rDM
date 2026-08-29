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
    pub fn size(&self) -> u64 {
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
}

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
    /// Global proxy (`None` = direct). Per-download override still wins.
    pub proxy: Option<String>,
    pub clipboard_monitor: bool,
    pub close_to_tray: bool,
    /// Allowed schedule window (24h). `None` means always allowed.
    pub schedule_enabled: bool,
    pub schedule_start: (u8, u8),
    pub schedule_end: (u8, u8),
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
}
