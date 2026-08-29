//! dm-engine: core download engine for the DM download manager.
//!
//! UI-agnostic and fully testable without Tauri. The Tauri backend (`src-tauri`)
//! depends on this crate and bridges it to the web frontend via commands/events.

pub mod chunk;
pub mod config;
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
pub use control::DownloadControl;
pub use limiter::{CombinedLimiter, TokenBucket};
pub use manager::{DownloadEvent, DownloadManager, EventSink, RunContext};
pub use model::{Category, ChecksumSpec, ChunkState, Download, DownloadId, DownloadStatus, Settings};
pub use scheduler::DownloadScheduler;
pub use storage::{Storage, StorageError};
pub use task::{layout_for_resume, run_download, sha256_of, TaskState};
