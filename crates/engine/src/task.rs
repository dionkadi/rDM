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
        {
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
}
