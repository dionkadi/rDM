//! dm-engine: core download engine for the DM download manager.
//!
//! UI-agnostic and fully testable without Tauri. The Tauri backend (`src-tauri`)
//! depends on this crate and bridges it to the web frontend via commands/events.

/// Engine version. Mirrors the `[package] version` in
/// `crates/engine/Cargo.toml` and is updated by `scripts/release.sh`
/// in lockstep with the rest of the workspace.
///
/// Exposed to the frontend via the Tauri `app_info` command so the
/// Settings → About panel can show the running engine's version
/// instead of a hard-coded string.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod chunk;
pub mod config;
pub mod cookies;
pub mod control;
pub mod limiter;
pub mod manager;
pub mod model;
pub mod protocol;
pub mod scheduler;
pub mod storage;
pub mod task;

pub use chunk::ChunkError;
pub use config::{categorize, default_categories, load_or_default};
pub use cookies::{read_browser_cookies, BrowserCookie, BrowserKind, CookieError, format_cookie_header};
pub use control::DownloadControl;
pub use limiter::{CombinedLimiter, TokenBucket};
pub use manager::{DownloadEvent, DownloadManager, EventSink, RunContext};
pub use model::{Category, ChecksumSpec, ChunkState, Download, DownloadId, DownloadStatus, Settings};
pub use scheduler::DownloadScheduler;
pub use storage::{Storage, StorageError};
pub use task::{layout_for_resume, run_download, sha256_of, TaskState};
