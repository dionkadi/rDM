//! A single range connection: streams one chunk and writes it at the correct file
//! offset, honouring the speed limiter and pause/cancel control.

use crate::control::SharedControl;
use crate::limiter::CombinedLimiter;
use crate::model::ChunkState;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    #[error("download canceled")]
    Canceled,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    /// The chunk stopped making progress for too long. Surfaces as a
    /// distinct variant so the task layer can mark the download as
    /// "stalled" (vs. a generic network error) and so the log file
    /// makes the difference obvious.
    #[error("chunk stalled (no progress for {0}s)")]
    Stalled(u64),
}

/// Walk the `source()` chain of a `reqwest::Error` so the log shows
/// the *deepest* error (e.g. "Invalid gzip header" or "tls handshake
/// eof") rather than the high-level wrapper.
fn error_chain(err: &reqwest::Error) -> String {
    let mut chain = String::new();
    let mut src: Option<&dyn std::error::Error> = Some(err);
    while let Some(e) = src {
        if !chain.is_empty() {
            chain.push_str(" <- ");
        }
        chain.push_str(&e.to_string());
        src = e.source();
    }
    chain
}

/// Build a one-line description of a `reqwest::Error` for the log. We
/// intentionally do **not** try to extract the response body here —
/// `reqwest::Error` does not keep the response around (the response
/// object is consumed by `error_for_status()`). The metadata we have
/// (`status`, `url`, `is_decode()`, `is_request()`, `is_redirect()`,
/// `is_builder()` + the source chain) is enough to diagnose every
/// known failure mode including the "error decoding response body"
/// case (`is_decode() == true`).
fn describe_error(err: &reqwest::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = err.status() {
        parts.push(format!("status={s}"));
    } else {
        parts.push("status=<none>".into());
    }
    if let Some(u) = err.url() {
        parts.push(format!("url={u}"));
    }
    parts.push(format!("is_decode={}", err.is_decode()));
    parts.push(format!("is_request={}", err.is_request()));
    parts.push(format!("is_redirect={}", err.is_redirect()));
    parts.push(format!("is_builder={}", err.is_builder()));
    parts.push(format!("chain={}", error_chain(err)));
    parts.join(" ")
}

/// Convert a non-success `Response` into the matching
/// `ChunkError::Http(reqwest::Error)`. `Response::error_for_status`
/// returns the original error, but we want to log the body first, so
/// we do it manually here. Takes ownership because the response can
/// only be consumed once.
fn status_error(resp: reqwest::Response) -> ChunkError {
    ChunkError::Http(
        resp.error_for_status()
            .err()
            .expect("status_error called with a success response"),
    )
}

/// Download `chunk` of `url` into `file_path`, resuming from already-downloaded
/// bytes. `on_progress` is called with the number of bytes written per buffer so
/// the task can update shared counters.
///
/// `stall_timeout` is how long the chunk may go without making any
/// progress before it's cancelled. The task layer passes
/// `crate::task::CHUNK_STALL_SECS` (5 minutes). Tests pass a much
/// shorter value so the test suite stays fast.
pub async fn download_chunk(
    client: &reqwest::Client,
    url: &str,
    chunk: &ChunkState,
    file_path: &Path,
    limiter: &CombinedLimiter,
    control: &SharedControl,
    stall_timeout: Duration,
    mut on_progress: impl FnMut(u64),
) -> Result<u64, ChunkError> {
    use futures_util::StreamExt;
    let start = chunk.resume_offset();
    let end = chunk.end;
    // `end == u64::MAX` means "to the end of the resource" (unknown
    // total size). For an open-ended chunk the engine cannot trust
    // the server to honour a `Range:` header — many CDN proxies
    // (cdn.akaere.online and friends) advertise no `Content-Length`
    // and no `Accept-Ranges` on HEAD, then **reject** any `Range:`
    // request with 416 because they don't know the upstream size.
    // So:
    //   * Send a plain `GET` (no `Range:` header).
    //   * Write the body from offset 0. The whole resource is
    //     coming back, so any previous partial data we had on disk
    //     is overwritten.
    //   * Resume of an open-ended chunk is impossible by definition
    //     (the server has no way to tell us "the rest of the bytes
    //     starting at N"), so a non-zero `chunk.downloaded` here is
    //     a stale value from a previous run — we reset it to 0.
    let open_ended = end == u64::MAX;
    let (range, start) = if open_ended {
        // No `Range:` header, write from the beginning of the file.
        (String::new(), 0u64)
    } else {
        (format!("bytes={}-{}", start, end), start)
    };

    // Shared "last write" timestamp updated after every successful
    // write to the file. A background watchdog compares this against
    // `now`; if the gap exceeds `stall_timeout`, the watchdog calls
    // `control.cancel()` and the chunk loop unwinds with
    // `ChunkError::Stalled`. This replaces the old per-request 30s
    // blanket timeout, which was killing every chunk on
    // multi-gigabyte downloads (the user's "error decoding response
    // body" bug: the body stream had simply been transferring for
    // 30s when reqwest's per-request timer fired, and `is_decode=true`
    // is how reqwest classifies timeouts that happen during a body
    // read).
    let last_write: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));
    let stall_secs = stall_timeout.as_secs();
    let stall_watchdog = {
        let last_write = Arc::clone(&last_write);
        let control = Arc::clone(control);
        let url = url.to_string();
        let range = range.clone();
        tokio::spawn(async move {
            // For long stalls the watchdog samples every 15 s, so a
            // 5-minute cap fires within ~15 s of expiry. For short
            // test stalls we use min(15, stall/3) so a 1 s test cap
            // still wakes up within ~300 ms.
            let check_every = Duration::from_secs(15)
                .min(stall_timeout / 3)
                .max(Duration::from_millis(50));
            let limit = stall_timeout;
            loop {
                tokio::time::sleep(check_every).await;
                if control.is_canceled() {
                    return;
                }
                let last = *last_write.lock().await;
                let idle = last.elapsed();
                if idle >= limit {
                    // For an open-ended chunk the range string is
                    // empty; render it as "(none)" so the log line is
                    // unambiguous about which request stalled.
                    log::error!(
                        "chunk stalled url={url} range={} idle={:.0}s limit={}s \
                         — cancelling (no body bytes for the stall window)",
                        if range.is_empty() { "(none)" } else { &range },
                        idle.as_secs(),
                        stall_secs
                    );
                    control.cancel();
                    return;
                }
            }
        })
    };

    // We deliberately do **not** use `?` on the `.send().await` call so
    // that, on failure, we can log the full error chain (URL, status,
    // is_decode/is_request, source) to the log file.
    //
    // For open-ended chunks (`end == u64::MAX`) the range string is
    // empty — we deliberately omit the `Range:` header because the
    // server is a strict proxy that 416s on range requests. The
    // resulting `GET` returns the full body and the chunk loop below
    // writes it from offset 0.
    let mut req = client.get(url);
    if !range.is_empty() {
        req = req.header(reqwest::header::RANGE, &range);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            stall_watchdog.abort();
            log::error!(
                "chunk send() failed stage=send url={url} range=\"{}\" detail=\"{}\"",
                range,
                describe_error(&e)
            );
            return Err(ChunkError::Http(e));
        }
    };
    // Capture status + headers for 4xx/5xx. We log the status +
    // headers here (the response body is read on demand by
    // `error_for_status` below and not separately dumped to the log;
    // the user-facing "http error: NNN Reason" message that
    // `error_for_status` produces carries the same forensic value
    // and is what the user already sees in the UI).
    let status = resp.status();
    if !status.is_success() {
        stall_watchdog.abort();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v.to_str().unwrap_or("<binary>")))
            .collect::<Vec<_>>()
            .join(" | ");
        log::error!(
            "chunk non-success status={} url={url} range={range} headers=[{}]",
            status, headers
        );
        return Err(status_error(resp));
    }

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(open_ended)
        .open(file_path)
        .await?;

    file.seek(std::io::SeekFrom::Start(start)).await?;

    let mut stream = resp.bytes_stream();
    let mut total_written: u64 = 0;

    while let Some(bytes) = stream.next().await {
        let bytes = match bytes {
            Ok(b) => b,
            Err(e) => {
                // The watchdog may have tripped. If it did, the
                // control flag is set and the body stream has been
                // cancelled; report the more user-friendly
                // `Stalled` variant instead of the raw reqwest
                // timeout.
                if control.is_canceled() {
                    let idle = last_write.lock().await.elapsed().as_secs();
                    stall_watchdog.abort();
                    log::error!(
                        "chunk stalled url={url} range={range} idle={idle}s — aborting"
                    );
                    return Err(ChunkError::Stalled(idle));
                }
                // Body-stream error for a non-stall reason
                // (e.g. connection reset, or a misconfigured
                // server returning a corrupt body).
                stall_watchdog.abort();
                log::error!(
                    "chunk body-stream error stage=stream url={url} range={range} detail=\"{}\"",
                    describe_error(&e)
                );
                return Err(ChunkError::Http(e));
            }
        };

        if control.is_canceled() {
            stall_watchdog.abort();
            return Err(ChunkError::Canceled);
        }
        if control.is_paused() {
            if control.wait_while_paused().await {
                stall_watchdog.abort();
                return Err(ChunkError::Canceled);
            }
        }

        // Throttle before writing this buffer.
        limiter.acquire(bytes.len() as u64).await;

        file.write_all(&bytes).await?;
        total_written += bytes.len() as u64;
        on_progress(bytes.len() as u64);

        // Bump the stall watchdog. Doing this after the write means
        // the watchdog measures the *real* time between successful
        // disk writes, not just the time between body bytes (which
        // the limiter can stretch out).
        *last_write.lock().await = Instant::now();
    }

    // Stream finished cleanly. Abort the watchdog so it doesn't
    // outlive the chunk.
    stall_watchdog.abort();

    file.flush().await?;
    Ok(total_written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limiter::CombinedLimiter;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    /// Spin up a tiny TCP server that responds to one request with
    /// valid headers + a small body, but then **stalls** (refuses to
    /// send any more bytes) so the chunk's body stream has nothing to
    /// read. This simulates the user's real-world failure mode: the
    /// server is alive (handshake + headers succeed) but the transfer
    /// stops without an explicit error.
    fn spawn_stalling_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if let Ok(mut s) = stream {
                    s.set_read_timeout(Some(Duration::from_secs(2))).ok();
                    // Drain the request so the client can move past
                    // the headers phase.
                    let mut buf = [0u8; 1024];
                    let _ = s.read(&mut buf);
                    // Send headers + a Content-Length that promises
                    // 100 bytes but only delivers 5. The client will
                    // then wait for the remaining 95 bytes — which
                    // is the exact "server stops mid-body" scenario.
                    let _ = s.write_all(
                        b"HTTP/1.1 200 OK\r\n\
                          Content-Length: 100\r\n\
                          Content-Type: application/octet-stream\r\n\
                          \r\n\
                          hello",
                    );
                    let _ = s.flush();
                    // Then hold the connection open without sending
                    // any more bytes (until the client disconnects).
                    // `set_read_timeout` causes our blocked `read`
                    // to time out so this thread can exit cleanly.
                    let mut dummy = [0u8; 1];
                    let _ = s.read(&mut dummy);
                }
            }
        });
        port
    }

    /// Regression test for the "error decoding response body" bug: a
    /// chunk whose body stream stops delivering bytes must be
    /// cancelled by the stall detector (returning
    /// `ChunkError::Stalled`) rather than dying with reqwest's
    /// per-request timeout (which used to surface as
    /// `is_decode=true + "operation timed out"` after 30 s).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn chunk_stall_detector_cancels_unresponsive_server() {
        // Point the test environment at a writable tempdir so the
        // chunk's `tokio::fs::File::create` doesn't depend on the
        // (sometimes read-only) cwd.
        let tmp = tempfile::tempdir().expect("tempdir");
        let file_path = tmp.path().join("out.bin");

        let port = spawn_stalling_server();
        let url = format!("http://127.0.0.1:{port}/file");
        let client = reqwest::Client::new();
        let chunk = ChunkState {
            index: 0,
            start: 0,
            end: 1024 * 1024, // pretend 1 MiB chunk
            downloaded: 0,
        };
        let limiter = CombinedLimiter::new(0, 0);
        let control = crate::DownloadControl::new();

        // Use a tiny stall timeout (1 s) so the test runs in ~1.5 s.
        let start = std::time::Instant::now();
        let result = download_chunk(
            &client,
            &url,
            &chunk,
            &file_path,
            &limiter,
            &control,
            Duration::from_millis(1000),
            |_| {},
        )
        .await;
        let elapsed = start.elapsed();

        // The chunk should be cancelled by the stall detector, not
        // by reqwest's per-request timeout (which we removed in
        // this change).
        assert!(
            matches!(result, Err(ChunkError::Stalled(_))),
            "expected Stalled, got {result:?}"
        );
        // Should fire within ~1.5 s of the stall timeout (we give
        // the watchdog up to 15 s in the long-timeout case, but the
        // test threshold is 1 s and the watchdog polls at
        // `min(15s, stall/3) = 333 ms`).
        assert!(
            elapsed < Duration::from_secs(5),
            "stall detector took too long: {elapsed:?}"
        );
    }

    /// Pin the open-ended-chunk semantics: `ChunkState { end: u64::MAX }`
    /// is used for "unknown total size" downloads. `size()` must
    /// return `u64::MAX` (not overflow) and `remaining()` must return
    /// `u64::MAX - downloaded` so the skip-already-complete check in
    /// `task.rs` doesn't misfire on these chunks.
    #[test]
    fn open_ended_chunk_size_and_remaining() {
        let c = ChunkState {
            index: 0,
            start: 0,
            end: u64::MAX,
            downloaded: 0,
        };
        assert_eq!(c.size(), u64::MAX);
        assert_eq!(c.remaining(), u64::MAX);
        let c = ChunkState {
            index: 0,
            start: 0,
            end: u64::MAX,
            downloaded: 1234,
        };
        assert_eq!(c.size(), u64::MAX);
        assert_eq!(c.remaining(), u64::MAX - 1234);
    }
}
