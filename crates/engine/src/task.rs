//! Per-download execution: probing, planning, running chunk workers, progress
//! aggregation, pause/cancel handling and completion/checksum verification.

use crate::chunk::download_chunk;
use crate::control::SharedControl;
use crate::limiter::CombinedLimiter;
use crate::manager::{DownloadEvent, RunContext};
use crate::model::{ChunkState, Download, DownloadStatus};
use crate::protocol::{build_plan, plan_chunks};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Shared mutable state for one download.
pub struct TaskState {
    pub id: String,
    pub download: Arc<Mutex<Download>>,
    pub control: SharedControl,
}

/// Build a reqwest client, applying a proxy override when provided.
fn build_client(proxy: Option<&str>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(8);
    if let Some(p) = proxy {
        if !p.is_empty() {
            if let Ok(proxy) = reqwest::Proxy::all(p) {
                builder = builder.proxy(proxy);
            }
        }
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

/// Run a download to completion. Holds a global download slot for its lifetime.
pub async fn run_download(ctx: Arc<RunContext>, state: Arc<TaskState>) {
    let _slot = ctx.scheduler.acquire_download().await;
    let id = state.id.clone();

    // ---- 1. Connect & plan ----
    {
        let mut d = state.download.lock().unwrap();
        d.status = DownloadStatus::Connecting;
        ctx.emit(DownloadEvent::StatusChanged(d.clone()));
        let _ = ctx.storage.save_download(&d);
    }

    let effective_proxy = {
        let d = state.download.lock().unwrap();
        d.proxy.clone().or_else(|| ctx.global_proxy.clone())
    };
    let client = build_client(effective_proxy.as_deref());

    // Plan on a local clone so the (std) mutex guard is never held across an await.
    let needs_plan = {
        let d = state.download.lock().unwrap();
        d.total_size.is_none() || d.chunks.is_empty()
    };
    if needs_plan {
        let mut local = state.download.lock().unwrap().clone();
        build_plan(&client, &mut local, ctx.max_connections).await;
        {
            let mut d = state.download.lock().unwrap();
            d.can_resume = local.can_resume;
            d.content_type = local.content_type;
            if d.filename.is_empty() || d.filename == "download.bin" {
                d.filename = local.filename;
            }
            d.total_size = local.total_size;
            if d.chunks.is_empty() {
                if local.total_size.is_none() && local.chunks.is_empty() {
                    // Unknown size + no ranges → single open-ended connection.
                    d.chunks = vec![ChunkState {
                        index: 0,
                        start: 0,
                        end: u64::MAX,
                        downloaded: d.downloaded,
                    }];
                    d.can_resume = false;
                } else if local.chunks.is_empty() {
                    // Known size but no ranges → one whole-file connection.
                    let total = local.total_size.unwrap();
                    d.chunks = vec![ChunkState {
                        index: 0,
                        start: 0,
                        end: total.saturating_sub(1),
                        downloaded: d.downloaded,
                    }];
                } else {
                    d.chunks = local.chunks;
                }
            }
        }
    }
    {
        let mut d = state.download.lock().unwrap();
        d.status = DownloadStatus::Downloading;
        ctx.emit(DownloadEvent::StatusChanged(d.clone()));
        let _ = ctx.storage.save_download(&d);
    }

    // ---- 2. Spawn chunk workers ----
    let per_download_limit = state.download.lock().unwrap().speed_limit.unwrap_or(0);
    let limiter = Arc::new(CombinedLimiter::new(ctx.global_limiter.rate(), per_download_limit));

    let (url, save_path, chunks) = {
        let d = state.download.lock().unwrap();
        (d.url.clone(), d.save_path.clone(), d.chunks.clone())
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<(usize, u64)>();

    let mut handles = Vec::new();
    for chunk in chunks {
        let client = client.clone();
        let file_path = save_path.clone();
        let limiter = limiter.clone();
        let control = state.control.clone();
        let state = state.clone();
        let ctx = ctx.clone();
        let chunk_index = chunk.index;
        let tx = tx.clone();
        let url = url.clone();
        let handle = tokio::spawn(async move {
            let _conn = ctx.scheduler.acquire_connection(&state.id).await;
            let res = download_chunk(
                &client,
                &url,
                &chunk,
                &file_path,
                &limiter,
                &control,
                move |written| {
                    let _ = tx.send((chunk_index, written));
                },
            )
            .await;
            (chunk_index, res)
        });
        handles.push(handle);
    }
    drop(tx); // drop our sender; workers hold the rest

    // ---- 3. Aggregate progress (throttled emission) ----
    let agg_state = state.clone();
    let agg_ctx = ctx.clone();
    let agg = tokio::spawn(async move {
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        while let Some((idx, written)) = rx.recv().await {
            {
                let mut d = agg_state.download.lock().unwrap();
                d.downloaded += written;
                if let Some(ch) = d.chunks.get_mut(idx) {
                    ch.downloaded += written;
                }
            }
            let now = Instant::now();
            if now.duration_since(last_emit) >= Duration::from_millis(150) {
                last_emit = now;
                let d = agg_state.download.lock().unwrap();
                agg_ctx.emit(DownloadEvent::Progress(d.clone()));
            }
        }
    });

    // ---- 4. Wait for workers, collect outcomes ----
    let mut cancelled = false;
    let mut first_error: Option<String> = None;
    for h in handles {
        if let Ok((_, res)) = h.await {
            match res {
                Ok(_) => {}
                Err(e) if e.to_string().contains("canceled") => cancelled = true,
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(e.to_string());
                    }
                }
            }
        }
    }
    let _ = agg.await;

    // ---- 5. Finalize ----
    {
        let mut d = state.download.lock().unwrap();
        if cancelled {
            d.status = DownloadStatus::Canceled;
            let _ = ctx.storage.save_download(&d);
            ctx.emit(DownloadEvent::StatusChanged(d.clone()));
            ctx.scheduler.release_download_state(&id);
            return;
        }
        if let Some(err) = first_error {
            d.status = DownloadStatus::Error;
            d.error = Some(err.clone());
            let _ = ctx.storage.mark_error(&id, &err);
            ctx.emit(DownloadEvent::Error(d.clone()));
            ctx.scheduler.release_download_state(&id);
            return;
        }

        // Known size → assert completeness.
        if let Some(total) = d.total_size {
            if d.downloaded < total {
                d.status = DownloadStatus::Error;
                d.error = Some("download incomplete".into());
                let _ = ctx.storage.mark_error(&id, "download incomplete");
                ctx.emit(DownloadEvent::Error(d.clone()));
                ctx.scheduler.release_download_state(&id);
                return;
            }
        }

        // Checksum verification.
        if let Some(cs) = &d.checksum {
            if cs.algorithm.eq_ignore_ascii_case("sha256") {
                match sha256_of(&save_path) {
                    Ok(actual) if actual.eq_ignore_ascii_case(&cs.expected) => {}
                    Ok(actual) => {
                        d.status = DownloadStatus::Error;
                        d.error = Some(format!("checksum mismatch (got {actual})"));
                        let _ = ctx.storage.mark_error(&id, &d.error.clone().unwrap());
                        ctx.emit(DownloadEvent::Error(d.clone()));
                        ctx.scheduler.release_download_state(&id);
                        return;
                    }
                    Err(e) => {
                        d.status = DownloadStatus::Error;
                        d.error = Some(format!("checksum read failed: {e}"));
                        let _ = ctx.storage.mark_error(&id, &d.error.clone().unwrap());
                        ctx.emit(DownloadEvent::Error(d.clone()));
                        ctx.scheduler.release_download_state(&id);
                        return;
                    }
                }
            }
        }

        d.status = DownloadStatus::Completed;
        d.finished_at = Some(chrono::Utc::now());
        let _ = ctx.storage.save_download(&d);
        let _ = ctx.storage.add_history(
            Some(&id),
            &d.url,
            &d.filename,
            d.total_size,
            &d.finished_at.unwrap().to_rfc3339(),
            d.category.as_deref(),
            None,
        );
        ctx.emit(DownloadEvent::Completed(d.clone()));
        ctx.scheduler.release_download_state(&id);
    }
}

/// Compute the SHA-256 hex digest of a file.
pub fn sha256_of(path: &Path) -> Result<String, std::io::Error> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Helper used by resume: build a fresh chunk layout for a known-size download.
pub fn layout_for_resume(total: u64, max_connections: usize) -> Vec<ChunkState> {
    plan_chunks(total, max_connections.max(1))
}
