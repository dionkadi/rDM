//! Maps engine lifecycle events into serialisable payloads emitted to the
//! webview over a single Tauri event channel.
//!
//! In addition to engine lifecycle events, the channel also carries
//! user-facing "captured URL" notifications from the native-messaging
//! host. The browser extension forwards every URL it sees
//! (downloads the user clicks, "Save link as", grab buttons) and the
//! user expects a confirmation dialog — IDM-style — before the file
//! hits the queue. So the native host doesn't `manager.add()` on
//! capture; it emits a `Captured` event and the frontend shows the
//! `CaptureDialog` for the user to confirm category / path /
//! filename. Only when the user clicks "Download" does the frontend
//! call the `add_download` Tauri command, which is the path that
//! actually constructs the engine `Download` and starts the task.

use dm_engine::manager::DownloadEvent;
use dm_engine::model::Download;
use serde::Serialize;

/// One event channel name the frontend subscribes to.
pub const EVENT_CHANNEL: &str = "download-event";

/// Payload for a `Captured` event — a URL the browser sent us, ready
/// for the user to confirm via the capture dialog.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedUrl {
    /// Source of the capture, used in the dialog title and toast.
    /// One of: `"browser-click"`, `"browser-save-as"`, `"browser-grab"`,
    /// `"native-host"`, `"unknown"`. Forwarded verbatim by the native
    /// host; the frontend uses it to label the dialog (e.g. "Browser
    /// asked to download …") and to decide whether to show an
    /// "always-grab" preference in the future.
    pub source: String,
    /// The raw URL the user / browser handed us.
    pub url: String,
    /// Filename extracted from the URL path / Content-Disposition
    /// (the engine's `protocol::suggest_filename` does this; the
    /// native host calls it on the Rust side so the result is
    /// available before the frontend has a chance to fetch
    /// headers).
    pub suggested_filename: String,
    /// The default save directory at the time of capture (the
    /// engine's `save_dir_for(None)` — i.e. the user's default
    /// download location). The frontend can override this once
    /// the user picks a category, and the resolved path is what
    /// the `add_download` command will actually use.
    pub default_save_dir: String,
    /// `Referer` header value the browser captured for the
    /// *source* page (i.e. the page that contained the link the
    /// user clicked). This is the high-leverage auth hint for
    /// the Tier-1 "Referer / user-agent per download" item:
    /// the frontend pre-fills the per-download Referer row
    /// with this value, and the user can confirm or edit
    /// before clicking "Download". `None` when the browser
    /// didn't send one (e.g. a copy-paste capture with no
    /// originating page).
    pub referer: Option<String>,
    /// User-Agent the browser was using at the time of
    /// capture. Some servers gate downloads on UA fingerprint
    /// (e.g. mobile-only mirrors) so pre-filling this in the
    /// per-download headers is the path of least surprise.
    /// `None` when the native host didn't relay one.
    pub user_agent: Option<String>,
    /// Monotonic ID so the frontend can dedupe in case the same URL
    /// comes in twice in quick succession (e.g. the user double-
    /// clicks).
    pub nonce: String,
}

/// A frontend-friendly view of an engine event OR a capture
/// notification. `kind` discriminates; the payload structure varies
/// per variant.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FrontendEvent {
    Added(Download),
    Progress(Download),
    StatusChanged(Download),
    Completed(Download),
    Error(Download),
    Removed { id: String },
    /// Sent by the native-messaging listener when a URL is captured
    /// from the browser. The frontend shows a confirmation dialog;
    /// the actual `add_download` only runs after the user confirms.
    Captured(CapturedUrl),
}

impl From<DownloadEvent> for FrontendEvent {
    fn from(e: DownloadEvent) -> Self {
        match e {
            DownloadEvent::Added(d) => FrontendEvent::Added(d),
            DownloadEvent::Progress(d) => FrontendEvent::Progress(d),
            DownloadEvent::StatusChanged(d) => FrontendEvent::StatusChanged(d),
            DownloadEvent::Completed(d) => FrontendEvent::Completed(d),
            DownloadEvent::Error(d) => FrontendEvent::Error(d),
            DownloadEvent::Removed(id) => FrontendEvent::Removed { id },
        }
    }
}
