//! Per-download execution: probing, planning, running chunk workers, progress
//! aggregation, pause/cancel handling and completion/checksum verification.

use crate::chunk::download_chunk;
use crate::control::SharedControl;
use crate::limiter::CombinedLimiter;
use crate::manager::{DownloadEvent, RunContext};
use crate::model::{
    AuthSpec, ChunkState, Download, DownloadStatus, MediaKind, SegmentProgress,
};
use crate::protocol::{build_plan, plan_chunks};
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::SeekFrom;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::mpsc;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// Shared mutable state for one download.
pub struct TaskState {
    pub id: String,
    pub download: Arc<Mutex<Download>>,
    pub control: SharedControl,
}

/// Default `User-Agent` for every request the engine makes.
///
/// Many CDNs, university mirrors (e.g. `mirrors.tuna.tsinghua.edu.cn`) and
/// software vendors reject requests that lack a `User-Agent` or that use a
/// non-browser UA like `reqwest/0.12`. We identify as a "real" browser-like
/// client with a recognisable product token. The string includes a short
/// build identifier and a contact line so an operator who needs to
/// rate-limit a misbehaving client can reach us.
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (compatible; DM-DownloadManager/0.1; +https://github.com/dm-project/dm)";

/// Stall threshold for a single chunk — if no progress has been made
/// for this many seconds, the chunk is considered dead and is
/// cancelled. 5 minutes is generous (the slowest legitimate link
/// we're targeting is a ~1 MB/s mirror, which still produces ~17 KB
/// every 17 ms) and small enough that a hung connection is caught
/// before the user gives up.
pub const CHUNK_STALL_SECS: u64 = 5 * 60;

/// Connect + headers timeout for the probe. 15 s is plenty: the probe
/// does a single HEAD (or a 0-byte range GET as a fallback) and only
/// reads headers, so it either succeeds in <2 s on a healthy server
/// or the server is dead. A 15 s cap lets the user see a clear
/// "probe timed out" instead of a 30 s blank screen.
pub const PROBE_TIMEOUT_SECS: u64 = 15;

/// Minimum body size (in bytes) we accept before auto-completing an
/// **open-ended** download (one where the probe couldn't discover
/// `Content-Length` and we created a chunk with `end == u64::MAX`).
///
/// When the proxy / CDN doesn't relay `Content-Length` (e.g.
/// `mirrors.ustc.edu.cn` for GitHub releases, `cdn.akaere.online`,
/// etc.), the chunk worker treats the body's end-of-stream as
/// completion. Without this guard, a truncated body — caused by the
/// proxy aborting the upstream connection because Chrome's own
/// download was racing ours, or because the upstream sent a 200-OK
/// with a placeholder body — would be marked `Completed` with a
/// misleading on-disk size (e.g. 879 B for a 142 MB file).
///
/// 1 KB is below the size of any reasonable user-facing download and
/// well above the size of an HTML error page or 302-redirect stub
/// that a misbehaving proxy might serve as the "complete" body. A
/// download that ends with less than this is marked `Error` with a
/// clear "open-ended transfer truncated at N B" message so the user
/// can see the problem and try again (with Chrome's own download
/// cancelled by the extension, see `browser-extension/background.js`).
pub const OPEN_ENDED_MIN_BYTES: u64 = 1024;

/// How often the aggregator persists the in-memory `downloaded` and
/// per-chunk `downloaded` counters to SQLite while a transfer is in
/// flight. Without this, a hard kill (force-quit, power loss, kernel
/// panic) loses every byte since the last status transition — the
/// "starts at 7% instead of 30%" bug. 1 s of lost progress is the
/// worst-case window.
pub const PROGRESS_FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// Byte threshold for the same flush. A fat pipe transferring 100 MB
/// in 1 s would otherwise see one flush per second regardless of how
/// much data moved; this caps per-flush write volume at ~1 MiB of
/// "delta" so the on-disk row is never more than ~1 MiB behind the
/// in-memory truth. The two thresholds are OR'd, so a slow link still
/// flushes every `PROGRESS_FLUSH_INTERVAL` even if it hasn't moved
/// 1 MiB yet.
pub const PROGRESS_FLUSH_BYTES: u64 = 1024 * 1024;

/// Build a reqwest client used for **chunk downloads**. It deliberately
/// does **not** set `.timeout(…)` — a 30 s blanket per-request
/// timeout was the previous behaviour, and it killed every chunk
/// after 30 s, surfacing as `is_decode=true` + `chain="error decoding
/// response body <- request or response body error <- operation
/// timed out"` from reqwest. A 2.7 GB ISO on a 5 MB/s mirror needs
/// ~9 minutes, and the connection's *transfer* time dwarfs the
/// per-request timeout we want for headers/connect.
///
/// Instead, `chunk.rs` implements a per-chunk stall detector
/// ([`CHUNK_STALL_SECS`]) that cancels the chunk if no bytes are
/// written for too long. That gives us the "fail on hung server"
/// guarantee without penalising slow-but-progressing transfers.
fn build_client(proxy: Option<&str>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        // `connect_timeout` caps only the TCP/TLS handshake (no body
        // transfer). 30 s is generous for slow / lossy networks and
        // still catches unreachable hosts quickly.
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(8)
        // Most mirrors / CDNs / vendor download portals reject requests that
        // arrive with the default `User-Agent: reqwest/x.y.z` (or with no UA
        // at all). The DEFAULT_USER_AGENT constant above is the polite
        // default; per-call overrides are still possible via the
        // `User-Agent` request header set in the chunk workers.
        .user_agent(DEFAULT_USER_AGENT)
        // See the function-level doc above for why we disable transparent
        // gzip / brotli / deflate decompression. The chunk client uses
        // raw bytes from the wire.
        .gzip(false)
        .brotli(false)
        .deflate(false)
        // `Accept: */*` is what browsers send by default; mirrors occasionally
        // gate `Accept` sniffing (e.g. serving an HTML error page to clients
        // that don't accept text/html). This keeps DM in the same boat as a
        // browser.
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("*/*"),
            );
            // Tell mirrors that send `Vary: Accept-Encoding` that we
            // understand the standard encodings. reqwest already advertises
            // these via `Accept-Encoding` automatically; the explicit header
            // here is a belt-and-braces for servers that grep on `Accept`.
            h.insert(
                reqwest::header::HeaderName::from_static("accept-language"),
                reqwest::header::HeaderValue::from_static("en-US,en;q=0.9,*;q=0.8"),
            );
            h
        });
    if let Some(p) = proxy {
        if !p.is_empty() {
            // Best-effort — invalid URLs (e.g. malformed SOCKS scheme) fall
            // back to a direct client and surface a runtime error when used.
            if let Ok(proxy) = reqwest::Proxy::all(p) {
                builder = builder.proxy(proxy);
            }
        }
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

/// Build a client used **only** for the probe (HEAD + a single 0-byte
/// range GET). It deliberately turns off transparent gzip / brotli /
/// deflate decompression:
///
/// * The probe never reads the body, so decompression is pure overhead.
/// * Some CDNs and mirrors (notably the `mirrors.tuna.tsinghua.edu.cn`
///   ISO tree) serve a `Content-Encoding: gzip` header but then send
///   either an empty / malformed body for HEAD or a tiny error page
///   that isn't actually gzipped. With auto-decompression enabled, this
///   surfaces as a generic `"error decoding response body"` from
///   `reqwest` — and the probe fails even though the **headers** we
///   care about (size, content-type, content-disposition) are valid.
/// * Disabling decompression on the probe keeps the real download
///   chunks on `build_client` (which still decompresses), so actual
///   gzipped ISOs are still handled correctly.
///
/// The probe uses a short per-request timeout ([`PROBE_TIMEOUT_SECS`])
/// so a dead server produces a clear "probe timed out" instead of
/// making the user wait 30 s for a connect-then-hang.
fn build_probe_client(proxy: Option<&str>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .pool_max_idle_per_host(2)
        .user_agent(DEFAULT_USER_AGENT)
        .gzip(false)
        .brotli(false)
        .deflate(false)
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("*/*"),
            );
            h.insert(
                reqwest::header::HeaderName::from_static("accept-language"),
                reqwest::header::HeaderValue::from_static("en-US,en;q=0.9,*;q=0.8"),
            );
            h
        });
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

    // Per-download override always wins; otherwise fall back to the global
    // proxy URL resolved from the active policy (None / System / Manual).
    let effective_proxy = {
        let d = state.download.lock().unwrap();
        d.proxy.clone().or_else(|| ctx.global_proxy.clone())
    };
    let client = build_client(effective_proxy.as_deref());
    // The probe client is identical to the chunk client *except* it has
    // transparent gzip / brotli / deflate decompression disabled. The probe
    // never reads the body, so decompression is unnecessary and breaks
    // on misconfigured mirrors that advertise `Content-Encoding: gzip`
    // but send a non-gzipped (or empty) body. See `build_probe_client`.
    let probe_client = build_probe_client(effective_proxy.as_deref());

    // An HLS download is a playlist, not a file, so the probe/chunk
    // pipeline below does not apply: fetch the manifest, walk its
    // segments, concatenate. Same slot, same limiter, same events.
    if state.download.lock().unwrap().media == Some(MediaKind::Hls) {
        run_hls_download(
            ctx.clone(),
            state.clone(),
            effective_proxy.clone(),
            id.clone(),
        )
        .await;
        return;
    }

    // Plan on a local clone so the (std) mutex guard is never held across an await.
    let needs_plan = {
        let d = state.download.lock().unwrap();
        d.total_size.is_none() || d.chunks.is_empty()
    };
    if needs_plan {
        let mut local = state.download.lock().unwrap().clone();
        build_plan(&probe_client, &mut local, ctx.max_connections).await;
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

    let (url, save_path, chunks, headers, auth) = {
        let d = state.download.lock().unwrap();
        (
            d.url.clone(),
            d.save_path.clone(),
            d.chunks.clone(),
            d.headers.clone(),
            d.auth.clone(),
        )
    };
    // The chunk workers write to `<save_path>.part` so a crash
    // mid-transfer can't leave a half-written file at the
    // final filename. On success the task layer renames
    // `.part` → `save_path` atomically; on failure the `.part`
    // file is left in place (so the user can inspect it or
    // resume) and the final filename is never created.
    let part_path = {
        let d = state.download.lock().unwrap();
        d.part_path()
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<(usize, u64)>();

    let mut handles = Vec::new();
    for chunk in chunks {
        // Skip already-complete chunks. On resume, a chunk whose
        // `downloaded` field already covers the entire range has no
        // work to do. Issuing a range GET for `bytes=N-N-1` (a
        // zero-length, end-before-start range) would be a malformed
        // request and most servers return 416, surfacing as a
        // spurious chunk error. Skipping here is both cheaper and
        // safer. The aggregator thread will still see no progress
        // from this chunk, which is fine — the other chunks drive
        // the download forward.
        if chunk.remaining() == 0 {
            log::debug!(
                "skipping already-complete chunk id={} index={} start={} end={} downloaded={}",
                id, chunk.index, chunk.start, chunk.end, chunk.downloaded
            );
            continue;
        }
        let client = client.clone();
        let limiter = limiter.clone();
        let control = state.control.clone();
        let state = state.clone();
        let ctx = ctx.clone();
        let chunk_index = chunk.index;
        // Clone `part_path` per chunk worker so each
        // `async move` closure owns its own copy. The chunk
        // worker writes to `<save_path>.part`; on success the
        // task layer renames it to the final filename.
        let file_path = part_path.clone();
        let tx = tx.clone();
        let url = url.clone();
        // Clone per-download headers and auth so the `async move`
        // closure can take ownership of them. The chunk workers
        // are spawned per-chunk and run concurrently; each
        // needs its own `BTreeMap` (cheap: usually 0–4 entries)
        // and `AuthSpec` (a `String` or two) so the closure
        // doesn't borrow from a value that's already been
        // moved into a sibling worker.
        let headers = headers.clone();
        let auth = auth.clone();
        let handle = tokio::spawn(async move {
            let _conn = ctx.scheduler.acquire_connection(&state.id).await;
            let res = download_chunk(
                &client,
                &url,
                &chunk,
                &file_path,
                &limiter,
                &control,
                Duration::from_secs(CHUNK_STALL_SECS),
                move |written| {
                    let _ = tx.send((chunk_index, written));
                },
                &headers,
                auth.as_ref(),
            )
            .await;
            (chunk_index, res)
        });
        handles.push(handle);
    }
    drop(tx); // drop our sender; workers hold the rest

    // ---- 3. Aggregate progress (throttled emission + periodic persistence) ----
    //
    // Two throttles, both important:
    //
    // * The frontend `Progress` event is emitted at most every 150 ms so the
    //   UI doesn't drown in IPC under heavy load.
    // * The SQLite row is persisted (`save_download`) at most every
    //   `PROGRESS_FLUSH_*`. Without this, a hard kill mid-transfer would lose
    //   every byte since the last status transition — i.e. the "starts at 7%
    //   instead of 30%" bug. The flush threshold is **bytes-based OR
    //   time-based** so a slow trickle (e.g. 10 KB/s on a throttled link) still
    //   gets persisted within a few seconds, and a fat pipe doesn't hammer
    //   the DB on every buffer.
    let agg_state = state.clone();
    let agg_ctx = ctx.clone();
    let agg_id = id.clone();
    let agg = tokio::spawn(async move {
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        let mut last_flush = Instant::now() - Duration::from_secs(1);
        let mut unflushed_bytes: u64 = 0;
        while let Some((idx, written)) = rx.recv().await {
            {
                let mut d = agg_state.download.lock().unwrap();
                d.downloaded += written;
                if let Some(ch) = d.chunks.get_mut(idx) {
                    ch.downloaded += written;
                }
            }
            // The user removed this download while it was running. Keep
            // draining the channel (so the chunk workers never block on
            // a full one) but stop touching storage and stop emitting.
            // `save_download` is an UPSERT, so a flush after `remove()`
            // would re-insert the very row that was just deleted, and
            // the row would also come back on the next launch.
            if agg_state.control.is_removed() {
                continue;
            }
            unflushed_bytes = unflushed_bytes.saturating_add(written);
            let now = Instant::now();
            // UI throttle (150 ms) — same as before, do not change.
            if now.duration_since(last_emit) >= Duration::from_millis(150) {
                last_emit = now;
                let d = agg_state.download.lock().unwrap();
                agg_ctx.emit(DownloadEvent::Progress(d.clone()));
            }
            // Persistence throttle (time + bytes). Either condition triggers
            // a flush. Both are loose, both intentionally cheap.
            let since_flush = now.duration_since(last_flush);
            if since_flush >= PROGRESS_FLUSH_INTERVAL
                || unflushed_bytes >= PROGRESS_FLUSH_BYTES
            {
                last_flush = now;
                unflushed_bytes = 0;
                let d = agg_state.download.lock().unwrap();
                if let Err(e) = agg_ctx.storage.save_download(&d) {
                    log::warn!(
                        "progress flush failed id={agg_id} downloaded={} err={e}",
                        d.downloaded
                    );
                }
            }
        }
        // Final flush on graceful stream-end so the very last bytes are
        // durable even if the run_download() code path doesn't immediately
        // transition the status (it does, but defence in depth).
        //
        // Skipped when the download was removed mid-flight: see the
        // `is_removed` check in the loop above.
        if !agg_state.control.is_removed() {
            let d = agg_state.download.lock().unwrap();
            if let Err(e) = agg_ctx.storage.save_download(&d) {
                log::warn!("final progress flush failed id={agg_id} err={e}");
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
                Err(crate::chunk::ChunkError::Canceled) => cancelled = true,
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
        // If the user removed this download while we were running, the
        // row is already gone and there is no status left to publish.
        // Everything below either writes to storage (`save_download` is
        // an UPSERT, so it would re-create the deleted row and the ghost
        // would survive a restart) or emits an event (which the frontend
        // would re-insert as a "canceled ghost" row). Neither is wanted:
        // `DownloadManager::remove` has already emitted the only event
        // this download gets, `Removed`, and the cancel flag below is
        // what makes the workers unwind — not a reason to report anything.
        if state.control.is_removed() {
            ctx.scheduler.release_download_state(&id);
            return;
        }
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
        } else {
            // Open-ended: we have no `Content-Length` to assert against,
            // but we can still catch the "the proxy gave us a 879-byte
            // truncated body and closed the stream" failure mode.
            // Anything below `OPEN_ENDED_MIN_BYTES` is almost
            // certainly a redirect HTML, an error page, or a
            // race-truncated response — not a real file.
            if d.downloaded < OPEN_ENDED_MIN_BYTES {
                let msg = format!(
                    "open-ended transfer truncated at {} B (expected a real file \
                     but the upstream closed the body stream almost immediately; \
                     this is usually a CDN mirror that aborted the upstream \
                     connection — retrying after a few seconds often works)",
                    d.downloaded
                );
                d.status = DownloadStatus::Error;
                d.error = Some(msg.clone());
                let _ = ctx.storage.mark_error(&id, &msg);
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

        // Atomic rename: `<save_path>.part` → `save_path`. The
        // chunk workers wrote to `.part` so a crash mid-transfer
        // can't leave a half-written file at the final
        // filename. On success we promote the `.part` to the
        // real filename in a single `rename` syscall (atomic
        // on the same filesystem). If the rename fails — e.g.
        // a permission error, or the `.part` file is missing
        // because an earlier error already cleaned it up — we
        // log the error and continue (the download is still
        // marked Completed; the user can manually rename the
        // `.part` file). We do NOT roll back the status: the
        // bytes are on disk, just under a different name.
        let part_path = d.part_path();
        match std::fs::rename(&part_path, &save_path) {
            Ok(()) => {}
            Err(e) => {
                log::error!(
                    "failed to rename .part to final filename id={id} part={} final={} err={e}",
                    part_path.display(),
                    save_path.display()
                );
            }
        }

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

// ─── HLS ────────────────────────────────────────────────────────
//
// A `.m3u8` grab used to download the playlist itself — a few hundred
// bytes of text saved under the user's filename. The playlist is a
// table of contents, so the download only becomes media once the
// segments are fetched in order and concatenated. Everything below
// does that with the same client, limiter, pause/cancel flags and event
// stream as the chunk path; the parsing lives in `hls.rs`.

/// Backoff between segment attempts. Three retries is what "a 404 on a
/// CDN edge that has the segment one node over" needs, without turning
/// a genuinely dead playlist into a multi-minute hang.
const HLS_RETRY_DELAYS_MS: [u64; 3] = [500, 1500, 4000];

/// Progress emission throttle, matching the chunk aggregator's 150 ms.
const HLS_EMIT_INTERVAL: Duration = Duration::from_millis(150);

/// Cap on a single HLS remux, mirroring `media::REMUX_TIMEOUT`.
const HLS_REMUX_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Which segments are already in the `.part` file.
///
/// `Download.chunks` cannot express this: HLS progress is "N segments of
/// this playlist", not a byte range, so resuming from a byte offset
/// would splice the wrong segment into the file. The sidecar is written
/// after each completed segment and deleted on success.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct HlsProgress {
    url: String,
    /// First segment URI — a playlist that changed invalidates the
    /// offsets, so resume restarts rather than splicing two edits.
    first: String,
    count: usize,
    next: usize,
    bytes: u64,
}

fn hls_sidecar(part: &Path) -> std::path::PathBuf {
    let mut name = part.as_os_str().to_os_string();
    name.push(".hls.json");
    std::path::PathBuf::from(name)
}

/// A sidecar may only be trusted for the exact playlist it describes:
/// same URL, same first segment, same length. A re-encoded or edited
/// playlist must restart instead of splicing two edits together.
fn hls_resume_matches(
    progress: &HlsProgress,
    url: &str,
    media: &crate::hls::MediaPlaylist,
) -> bool {
    progress.url == url
        && progress.first == media.segments[0].uri
        && progress.count == media.segments.len()
        && progress.next <= media.segments.len()
}

fn hls_request(
    client: &reqwest::Client,
    url: &str,
    headers: &BTreeMap<String, String>,
    auth: Option<&AuthSpec>,
) -> reqwest::RequestBuilder {
    let req = client.get(url);
    let req = crate::chunk::apply_extra_headers(req, headers);
    crate::chunk::apply_auth(req, auth, url)
}

async fn hls_fetch_text(
    client: &reqwest::Client,
    url: &str,
    headers: &BTreeMap<String, String>,
    auth: Option<&AuthSpec>,
) -> Result<String, String> {
    let resp = hls_request(client, url, headers, auth)
        .send()
        .await
        .map_err(|e| format!("could not fetch the playlist: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("the playlist returned HTTP {status}"));
    }
    resp.text()
        .await
        .map_err(|e| format!("could not read the playlist: {e}"))
}

/// Terminal state for the HLS path. Mirrors the finalize block of
/// `run_download` (removed ⇒ say nothing, cancel ⇒ Canceled, else Error).
fn hls_fail(ctx: &RunContext, state: &TaskState, id: &str, message: String) {
    if state.control.is_removed() {
        ctx.scheduler.release_download_state(id);
        return;
    }
    let mut d = state.download.lock().unwrap();
    d.status = DownloadStatus::Error;
    d.error = Some(message.clone());
    // The fetches are gone with this task; leaving `active` at N would
    // show a permanently "in flight" strip on a dead row.
    if let Some(sp) = d.segment_progress.as_mut() {
        sp.active = 0;
    }
    let _ = ctx.storage.mark_error(id, &message);
    ctx.emit(DownloadEvent::Error(d.clone()));
    drop(d);
    ctx.scheduler.release_download_state(id);
}

fn hls_canceled(ctx: &RunContext, state: &TaskState, id: &str) {
    if state.control.is_removed() {
        ctx.scheduler.release_download_state(id);
        return;
    }
    let mut d = state.download.lock().unwrap();
    d.status = DownloadStatus::Canceled;
    if let Some(sp) = d.segment_progress.as_mut() {
        sp.active = 0;
    }
    let _ = ctx.storage.save_download(&d);
    ctx.emit(DownloadEvent::StatusChanged(d.clone()));
    drop(d);
    ctx.scheduler.release_download_state(id);
}

enum SegmentError {
    Canceled,
    /// Worth another attempt: connection error, 5xx, 408, 429.
    Retry(String),
    /// Retrying cannot help: 4xx, local write failure.
    Fatal(String),
}

/// Connect + response-headers budget for one segment attempt.
///
/// The old code had no per-request bound at all: the only guard was a
/// five-minute timeout wrapped around the *whole* segment, so a CDN edge
/// that accepted the connection and then went silent looked like "no
/// progress" for five minutes before anything happened. 30 s is long
/// enough for a slow origin and short enough that a dead edge is retried
/// within the same minute.
const HLS_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Longest silence allowed between two body chunks of one segment. A
/// socket that dies without a FIN has to be caught here rather than by a
/// blanket timeout — this is the per-read stall detector the chunk path
/// has always had and the HLS path never did.
const HLS_READ_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Upper bound on concurrent segment fetches. `connections_per_download`
/// (default 8) is the real knob; this is a sanity cap so a user who sets
/// 64 connections does not open 64 sockets to one CDN edge.
const HLS_MAX_PARALLEL: usize = 8;

/// Bytes a fetcher has counted as in flight but not yet handed to the
/// writer. Dropping the guard without `keep()` subtracts them again, so
/// a failed attempt can never leave the live counter permanently
/// inflated (which would show up as a one-off speed spike).
struct LiveBytes<'a> {
    counter: &'a AtomicU64,
    unclaimed: u64,
}

impl<'a> LiveBytes<'a> {
    fn new(counter: &'a AtomicU64) -> Self {
        Self {
            counter,
            unclaimed: 0,
        }
    }

    fn add(&mut self, n: u64) {
        self.unclaimed += n;
        self.counter.fetch_add(n, Ordering::Relaxed);
    }

    /// Hand the bytes to the caller: they are no longer rolled back on
    /// drop, because the writer now owns them (and subtracts them once
    /// they are on disk).
    fn keep(&mut self) {
        self.unclaimed = 0;
    }
}

impl Drop for LiveBytes<'_> {
    fn drop(&mut self) {
        if self.unclaimed > 0 {
            self.counter.fetch_sub(self.unclaimed, Ordering::Relaxed);
        }
    }
}

/// One attempt at fetching one segment (or the init section) into
/// memory. Bounded at both ends: 30 s for the response headers, 30 s of
/// silence for each body read.
///
/// The bytes go into memory rather than straight to the file because
/// segments are fetched concurrently but written in playlist order — a
/// worker cannot know its file offset until every earlier segment has
/// landed. `HLS_MAX_SEGMENT_BYTES` caps the exposure.
#[allow(clippy::too_many_arguments)]
async fn hls_fetch_once(
    client: &reqwest::Client,
    url: &str,
    range: Option<(u64, u64)>,
    headers: &BTreeMap<String, String>,
    auth: Option<&AuthSpec>,
    limiter: &CombinedLimiter,
    control: &SharedControl,
    live: &AtomicU64,
) -> Result<Vec<u8>, SegmentError> {
    let mut req = hls_request(client, url, headers, auth);
    if let Some((length, offset)) = range {
        req = req.header(
            reqwest::header::RANGE,
            format!("bytes={}-{}", offset, offset.saturating_add(length).saturating_sub(1)),
        );
    }
    let resp = match tokio::time::timeout(HLS_RESPONSE_TIMEOUT, req.send()).await {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => return Err(SegmentError::Retry(format!("{e}"))),
        Err(_) => {
            return Err(SegmentError::Retry(format!(
                "no response headers within {}s",
                HLS_RESPONSE_TIMEOUT.as_secs()
            )))
        }
    };
    hls_check_status(resp.status(), url, range.is_some())?;

    // `Content-Length` is the only per-segment integrity check a
    // playlist gives us: an entry cut short by a proxy would otherwise
    // be written as a hole in the middle of the file.
    let declared = resp.content_length();

    let mut gauge = LiveBytes::new(live);
    let mut out: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    loop {
        if control.is_canceled() {
            return Err(SegmentError::Canceled);
        }
        let bytes = match tokio::time::timeout(HLS_READ_IDLE_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(bytes))) => bytes,
            Ok(Some(Err(e))) => return Err(SegmentError::Retry(format!("{e}"))),
            Ok(None) => break,
            Err(_) => {
                return Err(SegmentError::Retry(format!(
                    "no data for {}s",
                    HLS_READ_IDLE_TIMEOUT.as_secs()
                )))
            }
        };
        if out.len() as u64 + bytes.len() as u64 > HLS_MAX_SEGMENT_BYTES {
            return Err(SegmentError::Fatal(format!(
                "a segment exceeded {} MiB; refusing to buffer it",
                HLS_MAX_SEGMENT_BYTES / (1024 * 1024)
            )));
        }
        limiter.acquire(bytes.len() as u64).await;
        gauge.add(bytes.len() as u64);
        out.extend_from_slice(&bytes);
    }
    if let Some(expected) = declared {
        if expected != out.len() as u64 {
            return Err(SegmentError::Retry(format!(
                "the body ended at {} B of {expected} B",
                out.len()
            )));
        }
    }
    gauge.keep();
    Ok(out)
}

/// `hls_fetch_once` plus the retry policy, and the AES-128 decryption
/// that has to happen between the network and the file.
#[allow(clippy::too_many_arguments)]
async fn hls_fetch_segment(
    client: &reqwest::Client,
    url: &str,
    range: Option<(u64, u64)>,
    crypto: Option<([u8; 16], [u8; 16])>,
    headers: &BTreeMap<String, String>,
    auth: Option<&AuthSpec>,
    limiter: &CombinedLimiter,
    control: &SharedControl,
    live: &AtomicU64,
) -> Result<Vec<u8>, SegmentError> {
    let mut attempt: usize = 0;
    loop {
        if control.is_canceled() {
            return Err(SegmentError::Canceled);
        }
        let failure = match hls_fetch_once(
            client, url, range, headers, auth, limiter, control, live,
        )
        .await
        {
            Ok(mut bytes) => {
                if let Some((key, iv)) = crypto {
                    aes128_cbc_decrypt(&mut bytes, &key, &iv).map_err(SegmentError::Fatal)?;
                }
                return Ok(bytes);
            }
            Err(SegmentError::Retry(message)) => message,
            Err(other) => return Err(other),
        };
        if attempt >= HLS_RETRY_DELAYS_MS.len() {
            return Err(SegmentError::Retry(format!(
                "gave up after {} attempts: {failure}",
                attempt + 1
            )));
        }
        let delay = HLS_RETRY_DELAYS_MS[attempt];
        attempt += 1;
        log::warn!("hls segment attempt {attempt} failed ({failure}); retrying in {delay} ms");
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
}

/// A single segment larger than this is refused rather than buffered.
/// AES-128 decryption needs the whole ciphertext in memory, and an HLS
/// segment is a few seconds of media (single-digit MB), so a larger one
/// means the playlist is not what it claims.
const HLS_MAX_SEGMENT_BYTES: u64 = 128 * 1024 * 1024;

/// Shared status handling for segment / key requests.
fn hls_check_status(
    status: reqwest::StatusCode,
    url: &str,
    ranged: bool,
) -> Result<(), SegmentError> {
    if !status.is_success() {
        let message = format!("HTTP {status} for {url}");
        let retryable = status.is_server_error()
            || status.as_u16() == 408
            || status.as_u16() == 429;
        return Err(if retryable {
            SegmentError::Retry(message)
        } else {
            SegmentError::Fatal(message)
        });
    }
    // A 200 answers a ranged request with the *whole* file: appending it
    // would splice the wrong bytes into the output.
    if ranged && status.as_u16() != 206 {
        return Err(SegmentError::Fatal(format!(
            "the server ignored the byte range (HTTP {status}); this playlist's \
             segments are slices of one file"
        )));
    }
    Ok(())
}

/// Fetch the AES-128 key. Sixteen bytes is the only valid length.
async fn hls_fetch_key(
    client: &reqwest::Client,
    url: &str,
    headers: &BTreeMap<String, String>,
    auth: Option<&AuthSpec>,
) -> Result<[u8; 16], String> {
    let resp = hls_request(client, url, headers, auth)
        .send()
        .await
        .map_err(|e| format!("could not fetch the key: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("the key URL returned HTTP {status}"));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("could not read the key: {e}"))?;
    if bytes.len() != 16 {
        return Err(format!(
            "the AES-128 key is {} bytes, expected 16",
            bytes.len()
        ));
    }
    let mut key = [0u8; 16];
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// The IV for one AES-128 segment: the `IV=` attribute when present,
/// otherwise the media sequence number in a 16-byte big-endian block
/// (RFC 8216 §5.2).
fn hls_iv(key: &crate::hls::Key, sequence: u64) -> Result<[u8; 16], String> {
    let mut iv = [0u8; 16];
    match &key.iv {
        Some(raw) => {
            let hex = raw.trim().trim_start_matches("0x").trim_start_matches("0X");
            if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("unsupported AES-128 IV {raw:?}"));
            }
            for (i, byte) in iv.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
                    .map_err(|_| format!("unsupported AES-128 IV {raw:?}"))?;
            }
        }
        None => iv[8..].copy_from_slice(&sequence.to_be_bytes()),
    }
    Ok(iv)
}

/// AES-128-CBC decrypt in place, PKCS#7 unpadded.
fn aes128_cbc_decrypt(data: &mut Vec<u8>, key: &[u8; 16], iv: &[u8; 16]) -> Result<(), String> {
    if data.is_empty() || data.len() % 16 != 0 {
        return Err(format!(
            "encrypted segment is {} bytes, not a positive multiple of the AES block size",
            data.len()
        ));
    }
    let plain_len = {
        let decryptor = Aes128CbcDec::new_from_slices(key, iv)
            .map_err(|_| "invalid AES-128 key or IV length".to_string())?;
        decryptor
            .decrypt_padded_mut::<Pkcs7>(data.as_mut_slice())
            .map_err(|_| "AES-128 padding is invalid — wrong key or IV".to_string())?
            .len()
    };
    data.truncate(plain_len);
    Ok(())
}

/// `ffmpeg -c copy` an MPEG-TS file into MP4. Returns the MP4 path.
///
/// The ADTS→ASC bitstream filter is what makes AAC audio from TS valid
/// in MP4, but it fails on non-AAC audio, so a second attempt runs
/// without it. Both attempts remove their own partial output.
async fn hls_remux_to_mp4(ts: &Path) -> Result<std::path::PathBuf, String> {
    if crate::media::which("ffmpeg").is_none() {
        return Err("ffmpeg is not installed".into());
    }
    let mp4 = ts.with_extension("mp4");
    for with_bsf in [true, false] {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-hide_banner", "-nostdin", "-y", "-i"])
            .arg(ts)
            .args(["-c", "copy"]);
        if with_bsf {
            cmd.args(["-bsf:a", "aac_adtstoasc"]);
        }
        cmd.args(["-movflags", "+faststart"])
            .arg(&mp4)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let ok = matches!(
            tokio::time::timeout(HLS_REMUX_TIMEOUT, cmd.status()).await,
            Ok(Ok(status)) if status.success()
        );
        if ok {
            if tokio::fs::metadata(&mp4)
                .await
                .map(|m| m.len() > 0)
                .unwrap_or(false)
            {
                return Ok(mp4);
            }
        }
        let _ = tokio::fs::remove_file(&mp4).await;
    }
    Err("ffmpeg could not remux the stream to MP4".into())
}

/// Download an HLS playlist by concatenating its segments.
///
/// Runs on the same slot the caller already acquired. Output is a single
/// `.ts` (or `.mp4` for fMP4 playlists); when ffmpeg is available the TS
/// is remuxed to MP4 and the row is retargeted to it, with the TS kept
/// as the source (same contract as the DASH pair merge).
async fn run_hls_download(
    ctx: Arc<RunContext>,
    state: Arc<TaskState>,
    proxy: Option<String>,
    id: String,
) {
    let control = Arc::clone(&state.control);
    let (url, headers, auth, per_download_limit) = {
        let d = state.download.lock().unwrap();
        (
            d.url.clone(),
            d.headers.clone(),
            d.auth.clone(),
            d.speed_limit.unwrap_or(0),
        )
    };
    let client = build_client(proxy.as_deref());
    let limiter = Arc::new(CombinedLimiter::new(
        ctx.global_limiter.rate(),
        per_download_limit,
    ));

    // ---- 1. Read the playlist (following one master hop) ----
    let text = match hls_fetch_text(&client, &url, &headers, auth.as_ref()).await {
        Ok(text) => text,
        Err(e) => return hls_fail(&ctx, &state, &id, e),
    };
    let base = match url::Url::parse(&url) {
        Ok(base) => base,
        Err(e) => return hls_fail(&ctx, &state, &id, format!("invalid playlist URL: {e}")),
    };
    let mut playlist = match crate::hls::parse(&text, &base) {
        Ok(playlist) => playlist,
        Err(e) => return hls_fail(&ctx, &state, &id, e.to_string()),
    };
    if let crate::hls::Playlist::Master {
        variants,
        separate_audio,
    } = &playlist
    {
        if *separate_audio {
            return hls_fail(
                &ctx,
                &state,
                &id,
                "this HLS master keeps audio in a separate rendition; DM cannot merge \
                 the two yet, so downloading the video variant alone would give a silent file"
                    .into(),
            );
        }
        let Some(best) = crate::hls::best_variant(variants) else {
            return hls_fail(&ctx, &state, &id, "the playlist lists no variants".into());
        };
        let variant_url = best.uri.clone();
        let variant_text = match hls_fetch_text(&client, &variant_url, &headers, auth.as_ref()).await {
            Ok(text) => text,
            Err(e) => return hls_fail(&ctx, &state, &id, e),
        };
        let variant_base = match url::Url::parse(&variant_url) {
            Ok(base) => base,
            Err(e) => return hls_fail(&ctx, &state, &id, format!("invalid variant URL: {e}")),
        };
        playlist = match crate::hls::parse(&variant_text, &variant_base) {
            Ok(playlist) => playlist,
            Err(e) => return hls_fail(&ctx, &state, &id, e.to_string()),
        };
    }
    let crate::hls::Playlist::Media(media) = playlist else {
        return hls_fail(
            &ctx,
            &state,
            &id,
            "the playlist points at another playlist; nested masters are not supported".into(),
        );
    };
    if !media.ended {
        return hls_fail(
            &ctx,
            &state,
            &id,
            "this is a live HLS playlist (no #EXT-X-ENDLIST): there is no finished file \
             to download, only an endless stream"
                .into(),
        );
    }
    if let Some(method) = media.unsupported_key_method() {
        return hls_fail(
            &ctx,
            &state,
            &id,
            format!(
                "this HLS stream uses {method} encryption; DM only decrypts AES-128 \
                 (and clear) segments — writing the ciphertext as media would produce a \
                 corrupt file"
            ),
        );
    }

    if media.init_encrypted {
        return hls_fail(
            &ctx,
            &state,
            &id,
            "the fMP4 init section of this playlist is encrypted (#EXT-X-KEY before \
             #EXT-X-MAP); DM cannot decrypt it, and a ciphertext init makes the whole \
             file unplayable"
                .into(),
        );
    }

    // ---- 2. AES-128 keys (one fetch per key URI) ----
    let mut keys: HashMap<String, [u8; 16]> = HashMap::new();
    for segment in &media.segments {
        let Some(key) = segment.key.as_ref() else {
            continue;
        };
        let Some(uri) = key.uri.as_deref() else {
            return hls_fail(
                &ctx,
                &state,
                &id,
                "an AES-128 segment has no key URI in the playlist".into(),
            );
        };
        if keys.contains_key(uri) {
            continue;
        }
        match hls_fetch_key(&client, uri, &headers, auth.as_ref()).await {
            Ok(bytes) => {
                keys.insert(uri.to_string(), bytes);
            }
            Err(e) => {
                return hls_fail(
                    &ctx,
                    &state,
                    &id,
                    format!("could not fetch the AES-128 key {uri}: {e}"),
                )
            }
        }
    }

    // ---- 3. Output naming + resume point ----
    // fMP4 playlists (`#EXT-X-MAP`) concatenate into a valid MP4; TS
    // playlists stay `.ts` until the optional remux.
    let container = if media.init.is_some() { "mp4" } else { "ts" };
    {
        let mut d = state.download.lock().unwrap();
        d.retarget_extension(container);
    }
    let (save_path, part_path) = {
        let d = state.download.lock().unwrap();
        (d.save_path.clone(), d.part_path())
    };
    let sidecar = hls_sidecar(&part_path);

    let resume = match tokio::fs::read(&sidecar).await {
        Ok(raw) => serde_json::from_slice::<HlsProgress>(&raw)
            .ok()
            .filter(|p| hls_resume_matches(p, &url, &media)),
        Err(_) => None,
    };

    let mut file = match tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(&part_path)
        .await
    {
        Ok(file) => file,
        Err(e) => {
            return hls_fail(
                &ctx,
                &state,
                &id,
                format!("could not create {}: {e}", part_path.display()),
            )
        }
    };
    let (start_index, downloaded) = match &resume {
        Some(p) => (p.next, p.bytes),
        None => (0, 0),
    };
    // A sidecar claiming more than the file holds (truncated `.part`,
    // external cleanup) would otherwise be "resumed" by extending the
    // file with zeros. Start over instead.
    let on_disk = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let (start_index, mut downloaded) = if downloaded > on_disk {
        log::warn!(
            "hls resume discarded id={id}: sidecar claims {downloaded} B, file holds {on_disk} B"
        );
        (0, 0u64)
    } else {
        (start_index, downloaded)
    };
    if let Err(e) = file.set_len(downloaded).await {
        return hls_fail(&ctx, &state, &id, format!("could not prepare the output: {e}"));
    }
    if let Err(e) = file.seek(SeekFrom::End(0)).await {
        return hls_fail(&ctx, &state, &id, format!("could not seek the output: {e}"));
    }
    if start_index > 0 {
        log::info!(
            "hls resume id={id} segment={start_index}/{} bytes={downloaded}",
            media.segments.len()
        );
    }

    let remaining = media.segments.len().saturating_sub(start_index);
    let concurrency = ctx
        .max_connections
        .clamp(1, HLS_MAX_PARALLEL)
        .min(remaining.max(1));

    // The playlist is understood and the output file is open: from the
    // user's point of view the download is running now, not connecting.
    // The previous code never left `Connecting` for the whole transfer —
    // `run_download` sets it before dispatching here and returns before
    // its own `Downloading` transition, and only the finalize path ever
    // touched the status again.
    //
    // The segment map is published in the same transition so the row
    // shows "12 / 264 · 8 in flight" from the first frame instead of
    // claiming there is no structure at all.
    {
        let mut d = state.download.lock().unwrap();
        d.status = DownloadStatus::Downloading;
        d.segment_progress = Some(SegmentProgress {
            total: media.segments.len(),
            done: start_index,
            active: concurrency.min(remaining),
        });
        ctx.emit(DownloadEvent::StatusChanged(d.clone()));
        let _ = ctx.storage.save_download(&d);
    }

    // ---- 4. Init section, then segments ----
    //
    // Segments are fetched `concurrency` at a time and written in
    // playlist order: `buffered` polls every in-flight fetch and yields
    // their results *in order*, so the writer only ever appends and the
    // `.part` stays a valid prefix of the output — which is exactly what
    // the resume sidecar claims it is.
    //
    // `live` counts bytes that in-flight fetches have received but that
    // are not on disk yet. `downloaded + live` is the honest progress
    // number; reporting only `downloaded` (the previous behaviour) made
    // the row advance in whole-segment jumps and the throughput read
    // 0 B/s in between.
    let live = Arc::new(AtomicU64::new(0));

    // The init segment is part of the file, and only valid at offset 0.
    if start_index == 0 {
        if let Some(init) = media.init.clone() {
            match hls_fetch_segment(
                &client, &init, None, None, &headers, auth.as_ref(), &limiter, &control, &live,
            )
            .await
            {
                Ok(bytes) => {
                    if let Err(e) = file.write_all(&bytes).await {
                        return hls_fail(
                            &ctx,
                            &state,
                            &id,
                            format!("could not write the file: {e}"),
                        );
                    }
                    downloaded += bytes.len() as u64;
                    live.fetch_sub(bytes.len() as u64, Ordering::Relaxed);
                }
                Err(SegmentError::Canceled) => return hls_canceled(&ctx, &state, &id),
                Err(SegmentError::Fatal(e)) | Err(SegmentError::Retry(e)) => {
                    return hls_fail(&ctx, &state, &id, format!("the init segment failed: {e}"))
                }
            }
        }
    }

    // Resolve every segment's key material up front: a missing key or a
    // malformed IV is a property of the playlist, not of an attempt, so
    // it must fail before any segment work is queued.
    let mut cryptos: Vec<Option<([u8; 16], [u8; 16])>> = Vec::with_capacity(media.segments.len());
    for segment in &media.segments {
        let crypto = match segment.key.as_ref() {
            None => None,
            Some(key) => {
                let Some(uri) = key.uri.as_deref() else {
                    return hls_fail(
                        &ctx,
                        &state,
                        &id,
                        "an AES-128 segment has no key URI in the playlist".into(),
                    );
                };
                let Some(bytes) = keys.get(uri).copied() else {
                    return hls_fail(
                        &ctx,
                        &state,
                        &id,
                        format!("the AES-128 key {uri} was not fetched"),
                    );
                };
                match hls_iv(key, segment.sequence) {
                    Ok(iv) => Some((bytes, iv)),
                    Err(e) => return hls_fail(&ctx, &state, &id, e),
                }
            }
        };
        cryptos.push(crypto);
    }

    // One future per pending segment. Built in a plain loop rather than
    // with `Iterator::map`: a closure here has to be higher-ranked over
    // the `&Segment` lifetime, which Rust infers as "not general
    // enough" once the future also borrows the client.
    let mut jobs = Vec::with_capacity(remaining);
    for (index, segment) in media.segments.iter().enumerate().skip(start_index) {
        // Bind the shared borrows *outside* the `async move` so each
        // future holds references instead of trying to move the client
        // out of the enclosing scope.
        let crypto = cryptos[index];
        let live = Arc::clone(&live);
        let client = &client;
        let headers = &headers;
        let auth = auth.as_ref();
        let limiter = limiter.as_ref();
        let control = &control;
        jobs.push(async move {
            let outcome = if control.wait_while_paused().await {
                Err(SegmentError::Canceled)
            } else {
                hls_fetch_segment(
                    client,
                    &segment.uri,
                    segment.range,
                    crypto,
                    headers,
                    auth,
                    limiter,
                    control,
                    &live,
                )
                .await
            };
            (index, outcome)
        });
    }
    let mut results = futures_util::stream::iter(jobs).buffered(concurrency);

    let mut media_done: f64 = media
        .segments
        .iter()
        .take(start_index)
        .map(|s| s.duration)
        .sum();
    let mut ticker = tokio::time::interval(HLS_EMIT_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // `interval` yields one tick immediately; consume it so the loop
    // below does not publish a progress event before any byte moves.
    ticker.tick().await;

    loop {
        tokio::select! {
            biased;
            next = results.next() => {
                let Some((index, outcome)) = next else { break };
                let bytes = match outcome {
                    Ok(bytes) => bytes,
                    Err(SegmentError::Canceled) => return hls_canceled(&ctx, &state, &id),
                    Err(SegmentError::Fatal(e)) => {
                        return hls_fail(
                            &ctx,
                            &state,
                            &id,
                            format!("segment {} failed: {e}", index + 1),
                        )
                    }
                    Err(SegmentError::Retry(e)) => {
                        return hls_fail(
                            &ctx,
                            &state,
                            &id,
                            format!(
                                "segment {} of {} failed: {e}",
                                index + 1,
                                media.segments.len()
                            ),
                        )
                    }
                };
                if control.is_canceled() {
                    return hls_canceled(&ctx, &state, &id);
                }
                if let Err(e) = file.write_all(&bytes).await {
                    return hls_fail(
                        &ctx,
                        &state,
                        &id,
                        format!("could not write the file: {e}"),
                    );
                }
                live.fetch_sub(bytes.len() as u64, Ordering::Relaxed);
                downloaded += bytes.len() as u64;
                media_done += media.segments[index].duration;

                let estimate = if media.duration > 0.0 && media_done > 0.0 {
                    // Bytes-per-second of media so far × total duration.
                    // The row shows a real percentage instead of
                    // "unknown total"; it is corrected on completion.
                    let est = (downloaded as f64 / media_done) * media.duration;
                    Some(est.max(downloaded as f64) as u64)
                } else {
                    None
                };
                let snapshot = {
                    let mut d = state.download.lock().unwrap();
                    d.downloaded = downloaded + live.load(Ordering::Relaxed);
                    if let Some(total) = estimate {
                        d.total_size = Some(total);
                    }
                    // `done` counts what is *in the file*; the fetches
                    // still in flight are the next `active` ones, which
                    // is what `buffered` guarantees once the queue is
                    // saturated.
                    d.segment_progress = Some(SegmentProgress {
                        total: media.segments.len(),
                        done: index + 1,
                        active: concurrency
                            .min(media.segments.len().saturating_sub(index + 1)),
                    });
                    d.clone()
                };
                ctx.emit(DownloadEvent::Progress(snapshot.clone()));
                let _ = ctx.storage.save_download(&snapshot);

                let progress = HlsProgress {
                    url: url.clone(),
                    first: media.segments[0].uri.clone(),
                    count: media.segments.len(),
                    next: index + 1,
                    bytes: downloaded,
                };
                if let Ok(json) = serde_json::to_vec(&progress) {
                    let _ = tokio::fs::write(&sidecar, json).await;
                }
            }
            _ = ticker.tick() => {
                let snapshot = {
                    let mut d = state.download.lock().unwrap();
                    d.downloaded = downloaded + live.load(Ordering::Relaxed);
                    d.clone()
                };
                ctx.emit(DownloadEvent::Progress(snapshot));
            }
        }
    }

    if let Err(e) = file.flush().await {
        return hls_fail(&ctx, &state, &id, format!("could not flush the output: {e}"));
    }
    drop(file);

    // ---- 5. Finalize ----
    if state.control.is_removed() {
        ctx.scheduler.release_download_state(&id);
        return;
    }
    if let Err(e) = tokio::fs::rename(&part_path, &save_path).await {
        return hls_fail(
            &ctx,
            &state,
            &id,
            format!(
                "could not move {} to {}: {e}",
                part_path.display(),
                save_path.display()
            ),
        );
    }
    let _ = tokio::fs::remove_file(&sidecar).await;

    // MPEG-TS plays everywhere worth playing it, but MP4 is what users
    // expect from a download manager. Remuxing is a stream copy; when
    // it fails the `.ts` is still a complete, playable file.
    if container == "ts" {
        match hls_remux_to_mp4(&save_path).await {
            Ok(mp4) => {
                let size = tokio::fs::metadata(&mp4)
                    .await
                    .map(|m| m.len())
                    .unwrap_or(downloaded);
                let mut d = state.download.lock().unwrap();
                d.save_path = mp4.clone();
                if let Some(name) = mp4.file_name().and_then(|n| n.to_str()) {
                    d.filename = name.to_string();
                }
                d.total_size = Some(size);
                d.downloaded = size;
                d.chunks = vec![ChunkState {
                    index: 0,
                    start: 0,
                    end: size.saturating_sub(1),
                    downloaded: size,
                }];
                let _ = ctx.storage.save_download(&d);
                log::info!("hls remuxed id={id} to {}", mp4.display());
            }
            Err(e) => {
                log::warn!(
                    "hls kept {} (remux unavailable: {e})",
                    save_path.display()
                );
            }
        }
    }

    {
        let mut d = state.download.lock().unwrap();
        if container == "ts" && d.save_path == save_path {
            // No remux: the row keeps the `.ts` and its real size.
            d.downloaded = downloaded;
            d.total_size = Some(downloaded);
            d.chunks = vec![ChunkState {
                index: 0,
                start: 0,
                end: downloaded.saturating_sub(1),
                downloaded,
            }];
        }
        if let Some(sp) = d.segment_progress.as_mut() {
            sp.done = sp.total;
            sp.active = 0;
        }
        d.can_resume = false;
        d.status = DownloadStatus::Completed;
        d.finished_at = Some(chrono::Utc::now());
        d.error = None;
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
    }
    ctx.scheduler.release_download_state(&id);
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
        hasher.update(&buf[0..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Helper used by resume: build a fresh chunk layout for a known-size download.
pub fn layout_for_resume(total: u64, max_connections: usize) -> Vec<ChunkState> {
    plan_chunks(total, max_connections.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    /// Spin up a tiny in-process HTTP server that captures the `User-Agent`
    /// header from the very first request, then answers `200 OK` with an
    /// empty body so reqwest's HEAD/GET probe completes.
    fn capture_user_agent() -> (u16, std::sync::Arc<std::sync::Mutex<Option<String>>>) {
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let cap = captured.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut s = match stream {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let cap = cap.clone();
                s.set_read_timeout(Some(Duration::from_secs(2))).ok();
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                if let Some(line) = head.lines().find(|l| l.to_ascii_lowercase().starts_with("user-agent:")) {
                    if let Some(value) = line.split_once(':') {
                        *cap.lock().unwrap() = Some(value.1.trim().to_string());
                    }
                }
                let body = b"OK";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes());
                let _ = s.write_all(body);
                break;
            }
        });
        (port, captured)
    }

    #[test]
    fn client_uses_polite_user_agent() {
        let (port, captured) = capture_user_agent();
        let client = build_client(None);
        let url = format!("http://127.0.0.1:{port}/probe");
        // Drive a real request so reqwest actually emits the headers.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let _ = client.get(&url).send().await;
        });
        // Give the server a beat to flush.
        std::thread::sleep(Duration::from_millis(50));
        let seen = captured.lock().unwrap().clone().unwrap_or_default();
        assert_eq!(seen, DEFAULT_USER_AGENT, "saw UA = {seen:?}");
        // The default reqwest UA would have been "reqwest/x.y.z"; the
        // polite string must not start with that.
        assert!(
            !seen.starts_with("reqwest/"),
            "engine should not use the default reqwest UA: {seen}"
        );
    }

    #[test]
    fn client_sends_accept_and_accept_language() {
        use std::sync::Arc;
        let captured: Arc<std::sync::Mutex<Vec<(String, String)>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let cap = captured.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut s = match stream {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let cap = cap.clone();
                s.set_read_timeout(Some(Duration::from_secs(2))).ok();
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                for line in head.lines() {
                    if let Some((k, v)) = line.split_once(':') {
                        if !k.starts_with("HTTP/") {
                            cap.lock().unwrap().push((k.trim().to_string(), v.trim().to_string()));
                        }
                    }
                }
                let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = s.write_all(resp);
                break;
            }
        });

        let client = build_client(None);
        let url = format!("http://127.0.0.1:{port}/probe");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let _ = client.get(&url).send().await;
        });
        std::thread::sleep(Duration::from_millis(50));
        let headers = captured.lock().unwrap().clone();
        let get = |k: &str| headers.iter().find(|(name, _)| name.eq_ignore_ascii_case(k)).map(|(_, v)| v.to_lowercase());
        assert_eq!(get("accept").as_deref(), Some("*/*"));
        assert!(get("accept-language").map(|v| v.starts_with("en")).unwrap_or(false));
    }

    /// Regression test: a misconfigured server that lies about its
    /// `Content-Encoding: gzip` (i.e. the header is set but the body is
    /// plain bytes, common on directory listings and error pages from
    /// some CDNs / mirrors) must NOT surface as
    /// `http error: error decoding response body`. Both the probe and the
    /// chunk client have transparent decompression disabled for exactly
    /// this reason; this test pins the behaviour.
    #[test]
    fn client_tolerates_misleading_content_encoding() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop2 = stop.clone();
        std::thread::spawn(move || {
            // Accept up to two connections (one for probe, one for chunk)
            // before shutting down.
            for stream in listener.incoming() {
                if stop2.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                if let Ok(mut s) = stream {
                    s.set_read_timeout(Some(Duration::from_secs(2))).ok();
                    let mut buf = [0u8; 1024];
                    let _ = s.read(&mut buf);
                    // Send a body that is NOT gzipped but claim it is.
                    // If transparent decompression were enabled, reqwest
                    // would fail with "error decoding response body".
                    // The body is large enough that a real gzip decoder
                    // will trip over the bad bytes (the gzip magic
                    // header 0x1f 0x8b is required at offset 0, and
                    // our random-ish bytes don't match it).
                    let body: Vec<u8> = (0..4096).map(|i| b'a' + (i % 23) as u8).collect();
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Encoding: gzip\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = s.write_all(resp.as_bytes());
                    let _ = s.write_all(body.as_slice());
                }
            }
        });

        let probe = build_probe_client(None);
        let chunk = build_client(None);
        let url = format!("http://127.0.0.1:{port}/file");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            // Probe — must not error and must return valid headers.
            let resp = probe.head(&url).send().await.expect("probe head failed");
            assert!(resp.status().is_success(), "probe head status: {}", resp.status());

            // Chunk — must not error. We *also* read the body to the
            // end: reqwest's transparent decompression only fails when
            // the body stream is actually consumed (it is lazy), which
            // is what happens in production when the engine writes the
            // bytes to disk.
            let resp = chunk.get(&url).send().await.expect("chunk get failed");
            assert!(resp.status().is_success(), "chunk get status: {}", resp.status());
            let body = resp.bytes().await.expect("chunk body failed");
            assert_eq!(body.len(), 4096, "body length");
        });
        // Tell the server thread to stop, then drop the listener.
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        // Open + close a connection to unblock the accept loop.
        let _ = std::net::TcpStream::connect(("127.0.0.1", port));
        std::thread::sleep(Duration::from_millis(50));
    }
    #[test]
    fn hls_resume_only_matches_the_playlist_it_describes() {
        let base = url::Url::parse("https://cdn.test/hls/stream.m3u8").unwrap();
        let media = match crate::hls::parse(
            "#EXTM3U\n#EXTINF:6,\na.ts\n#EXTINF:6,\nb.ts\n#EXT-X-ENDLIST\n",
            &base,
        )
        .expect("playlist")
        {
            crate::hls::Playlist::Media(media) => media,
            _ => unreachable!(),
        };
        let good = HlsProgress {
            url: "https://cdn.test/hls/stream.m3u8".into(),
            first: "https://cdn.test/hls/a.ts".into(),
            count: 2,
            next: 1,
            bytes: 123,
        };
        assert!(hls_resume_matches(&good, &good.url, &media));
        assert!(
            !hls_resume_matches(&good, "https://cdn.test/other.m3u8", &media),
            "a different playlist must restart"
        );
        assert!(
            !hls_resume_matches(
                &HlsProgress {
                    count: 3,
                    ..good.clone()
                },
                "https://cdn.test/hls/stream.m3u8",
                &media
            ),
            "an edited playlist (different segment count) must restart"
        );
        assert!(
            !hls_resume_matches(
                &HlsProgress {
                    next: 3,
                    ..good.clone()
                },
                "https://cdn.test/hls/stream.m3u8",
                &media
            ),
            "a next index past the end is not a resume point"
        );
    }

    #[test]
    fn aes_iv_defaults_to_the_media_sequence() {
        let key = crate::hls::Key {
            method: "AES-128".into(),
            uri: None,
            iv: None,
        };
        let iv = hls_iv(&key, 0x0102).expect("iv");
        assert_eq!(&iv[..8], &[0u8; 8], "the sequence lives in the low 64 bits");
        assert_eq!(&iv[8..], &[0, 0, 0, 0, 0, 0, 1, 2]);

        let explicit = crate::hls::Key {
            method: "AES-128".into(),
            uri: None,
            iv: Some("0x000102030405060708090A0B0C0D0E0F".into()),
        };
        assert_eq!(
            hls_iv(&explicit, 9).expect("explicit iv"),
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
        );

        let malformed = crate::hls::Key {
            method: "AES-128".into(),
            uri: None,
            iv: Some("0x12".into()),
        };
        assert!(hls_iv(&malformed, 0).is_err(), "short IVs must be reported");
    }

    #[test]
    fn aes_decryption_roundtrips_and_rejects_non_block_data() {
        use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
        type Enc = cbc::Encryptor<aes::Aes128>;

        let key = [7u8; 16];
        let iv = [0u8; 16];
        let plain = b"a segment payload that is not block aligned".to_vec();

        let mut buf = vec![0u8; plain.len() + 16];
        buf[..plain.len()].copy_from_slice(&plain);
        let encryptor = Enc::new_from_slices(&key, &iv).expect("key/iv");
        let written = encryptor
            .encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len())
            .expect("encrypt")
            .len();
        buf.truncate(written);

        let mut decrypted = buf.clone();
        aes128_cbc_decrypt(&mut decrypted, &key, &iv).expect("decrypt");
        assert_eq!(decrypted, plain);

        let mut ragged = vec![0u8; 17];
        assert!(
            aes128_cbc_decrypt(&mut ragged, &key, &iv).is_err(),
            "ciphertext that is not a multiple of 16 cannot be AES-CBC"
        );
        let mut empty: Vec<u8> = Vec::new();
        assert!(aes128_cbc_decrypt(&mut empty, &key, &iv).is_err());
    }

}
