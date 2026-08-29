//! Maps engine lifecycle events into serialisable payloads emitted to the
//! webview over a single Tauri event channel.

use dm_engine::manager::DownloadEvent;
use dm_engine::model::Download;
use serde::Serialize;

/// One event channel name the frontend subscribes to.
pub const EVENT_CHANNEL: &str = "download-event";

/// A frontend-friendly view of an engine event. `kind` discriminates the
/// variant; most variants carry the (possibly updated) `Download`.
#[derive(Clone, Serialize)]
#[serde(tag = "kind", content = "download", rename_all = "camelCase")]
pub enum FrontendEvent {
    Added(Download),
    Progress(Download),
    StatusChanged(Download),
    Completed(Download),
    Error(Download),
    Removed { id: String },
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
