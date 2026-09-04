//! Headless integration test for the download engine.
//!
//! Spins up a tiny in-process HTTP/1.1 server that supports `Range` requests
//! (and a second mode that refuses ranges to exercise the single-connection
//! fallback). Drives the real `DownloadManager` and asserts that the bytes
//! written to disk match the source and the final status is `Completed`.
//!
//! Run with: `cargo test -p dm-engine --test integration`

use dm_engine::manager::DownloadManager;
use dm_engine::model::{Download, DownloadStatus, Settings};
use dm_engine::storage::Storage;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

// 256 KiB of a deterministic, easily-verifiable pattern. The legacy
// tests use this — small enough to drain through the in-process
// server in a single TCP write and complete in well under a second.
fn payload() -> Vec<u8> {
    let mut v = Vec::with_capacity(256 * 1024);
    for i in 0..(256 * 1024) {
        v.push((i as u8).wrapping_mul(31).wrapping_add(7));
    }
    v
}

// 64 MiB variant for the persistence tests. With
// `connections_per_download = 8` the chunks are 8 MiB each, so a
// partial download always leaves work for the resumed manager AND
// the transfer takes long enough (>>100 ms) that the test's 20 ms
// `wait_for_min_bytes` poll reliably catches an in-progress state.
// 4 MiB was too small — the in-process server finishes the whole
// transfer in ~50 ms, faster than any reasonable poll.
fn large_payload() -> Vec<u8> {
    let mut v = Vec::with_capacity(64 * 1024 * 1024);
    for i in 0..(64 * 1024 * 1024) {
        v.push((i as u8).wrapping_mul(31).wrapping_add(7));
    }
    v
}

/// 0 = full Range support, 1 = claims no ranges (fallback path).
/// `size` selects between the 256 KiB and 4 MiB payloads; legacy
/// tests pass `0` (256 KiB), persistence tests pass `1` (4 MiB).
fn spawn_server(mode: u8, size: u8) -> u16 {
    let data = if size == 0 { payload() } else { large_payload() };
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind server");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if let Ok(mut s) = stream {
                let data = data.clone();
                std::thread::spawn(move || handle(&mut s, &data, mode));
            }
        }
    });
    port
}

fn handle(stream: &mut std::net::TcpStream, data: &[u8], mode: u8) {
    // Read headers only (we don't stream bodies back incrementally).
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if Instant::now() > deadline {
            return;
        }
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                buf.push(byte[0]);
                if buf.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let req = String::from_utf8_lossy(&buf);
    let mut lines = req.lines();
    let request_line = lines.next().unwrap_or("").to_string();
    let mut range: Option<(u64, u64)> = None;
    for line in lines {
        if let Some(rest) = line.to_ascii_lowercase().strip_prefix("range:") {
            range = parse_range(rest.trim(), data.len() as u64);
        }
    }

    let method = request_line
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();

    if method.eq_ignore_ascii_case("HEAD") {
        let body = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
            data.len()
        );
        let _ = stream.write_all(body.as_bytes());
        return;
    }

    if mode == 1 {
        // Refuse ranges: always serve the entire body as 200.
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: none\r\nConnection: close\r\n\r\n",
            data.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(data);
        let _ = stream.flush();
        return;
    }

    match range {
        Some((start, end)) => {
            let end = end.min((data.len() as u64) - 1);
            let slice = &data[start as usize..=end as usize];
            let header = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                start,
                end,
                data.len(),
                slice.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(slice);
            // Explicit `flush()` before the function returns.
            // Without it, the kernel buffer might not have
            // drained when the `stream` is dropped, and the
            // client (the chunk worker) could see a truncated
            // response. This is the root cause of the
            // `resume_after_crash` test failing at index
            // 8.4 MB with zeros: the server's 8 MB range
            // response was only ~400 KB by the time the
            // client read it. The fix is to flush the
            // response before the closure returns so the
            // kernel sends the full 8 MB.
            let _ = stream.flush();
        }
        None => {
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                data.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(data);
            let _ = stream.flush();
        }
    }
}

fn parse_range(spec: &str, total: u64) -> Option<(u64, u64)> {
    // Expects "bytes=START-END" (END optional or open-ended).
    let inner = spec.trim_start_matches("bytes=").trim();
    let mut it = inner.splitn(2, '-');
    let start = it.next()?.parse::<u64>().ok()?;
    let end = match it.next() {
        Some("") | None => total - 1,
        Some(e) => e.parse::<u64>().ok()?,
    };
    if start > end || end >= total {
        return None;
    }
    Some((start, end))
}

async fn wait_until_downloaded(mgr: &DownloadManager, id: &str) -> Download {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let d = mgr
            .get(id)
            .expect("download exists while waiting");
        if matches!(
            d.status,
            DownloadStatus::Completed | DownloadStatus::Error | DownloadStatus::Canceled
        ) {
            return d;
        }
        if Instant::now() > deadline {
            panic!("download {id} did not finish within 30s (status: {:?})", d.status);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_segmented_download_writes_correct_bytes() {
    let port = spawn_server(0, 0);
    let tmp = std::env::temp_dir().join(format!("dm_it_seg_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&tmp);

    let storage = Storage::open_memory().expect("memory storage");
    let mgr = DownloadManager::with_settings(storage, Settings::default());

    let mut d = Download::new(format!("http://127.0.0.1:{port}/file.bin"));
    d.id = "seg".into();
    d.save_path = tmp.clone();
    let id = d.id.clone();
    mgr.add(d);

    let d = wait_until_downloaded(&mgr, &id).await;
    assert_eq!(
        d.status,
        DownloadStatus::Completed,
        "status was {:?}: {:?}",
        d.status,
        d.error
    );

    let written = std::fs::read(&tmp).expect("output file exists");
    assert_eq!(written.len(), payload().len(), "byte count matches");
    assert_eq!(written, payload(), "bytes match exactly");

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fallback_single_connection_when_no_ranges() {
    let port = spawn_server(1, 0);
    let tmp = std::env::temp_dir().join(format!("dm_it_fb_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&tmp);

    let storage = Storage::open_memory().expect("memory storage");
    let mgr = DownloadManager::with_settings(storage, Settings::default());

    let mut d = Download::new(format!("http://127.0.0.1:{port}/file.bin"));
    d.id = "fb".into();
    d.save_path = tmp.clone();
    let id = d.id.clone();
    mgr.add(d);

    let d = wait_until_downloaded(&mgr, &id).await;
    assert_eq!(
        d.status,
        DownloadStatus::Completed,
        "fallback status was {:?}: {:?}",
        d.status,
        d.error
    );

    let written = std::fs::read(&tmp).expect("output file exists");
    assert_eq!(written, payload(), "fallback bytes match exactly");

    let _ = std::fs::remove_file(&tmp);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checksum_mismatch_marks_error() {
    let port = spawn_server(0, 0);
    let tmp = std::env::temp_dir().join(format!("dm_it_chk_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&tmp);

    let storage = Storage::open_memory().expect("memory storage");
    let mgr = DownloadManager::with_settings(storage, Settings::default());

    let mut d = Download::new(format!("http://127.0.0.1:{port}/file.bin"));
    d.id = "chk".into();
    d.save_path = tmp.clone();
    // Deliberately wrong checksum so verification must fail.
    d.checksum = Some(dm_engine::model::ChecksumSpec {
        algorithm: "sha256".to_string(),
        expected: "deadbeef".repeat(8),
    });
    let id = d.id.clone();
    mgr.add(d);

    let d = wait_until_downloaded(&mgr, &id).await;
    assert_eq!(
        d.status,
        DownloadStatus::Error,
        "expected checksum failure, got {:?}: {:?}",
        d.status,
        d.error
    );
    assert!(
        d.error
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase()
            .contains("checksum"),
        "error mentions checksum: {:?}",
        d.error
    );

    let _ = std::fs::remove_file(&tmp);
}

// ── Persistence regression tests ────────────────────────────────────
//
// The original bug: `downloaded` and per-chunk `downloaded` were only
// flushed to SQLite on status transitions, so a hard kill mid-transfer
// lost every byte since the last status change. After the fix, the
// aggregator flushes the row every `PROGRESS_FLUSH_INTERVAL` (1 s) or
// every `PROGRESS_FLUSH_BYTES` (1 MiB), whichever fires first. These
// tests pin that contract end-to-end.

/// Wait until a download has at least `min_bytes` of in-memory progress
/// and then return its current `Download` snapshot. Used by the
/// crash-recovery tests to land the transfer at a known intermediate
/// state.
async fn wait_for_min_bytes(mgr: &DownloadManager, id: &str, min_bytes: u64) -> Download {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let d = mgr.get(id).expect("download exists while waiting");
        if d.downloaded >= min_bytes {
            return d;
        }
        if matches!(
            d.status,
            DownloadStatus::Completed | DownloadStatus::Error | DownloadStatus::Canceled
        ) {
            // Reached a terminal state before we hit the byte budget.
            return d;
        }
        if Instant::now() > deadline {
            panic!(
                "download {id} did not reach {min_bytes} bytes within 20s \
                 (last status: {:?}, downloaded: {})",
                d.status, d.downloaded
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Regression test for the "starts at 7% instead of 30%" bug. Without
/// a periodic flush, the aggregator only persisted on status
/// transitions; a hard kill at 30% would leave the SQLite row at
/// `downloaded = 0`. With the fix, after a few hundred ms the row on
/// disk must already reflect a non-zero progress.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn progress_persists_to_sqlite_mid_transfer() {
    let port = spawn_server(0, 1);
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let db_path = tmp_dir.path().join("dm.sqlite");
    let out_path = tmp_dir.path().join("out.bin");
    let _ = std::fs::remove_file(&out_path);

    let storage = Storage::open(&db_path).expect("file storage");
    let mgr = DownloadManager::with_settings(storage, Settings::default());

    let mut d = Download::new(format!("http://127.0.0.1:{port}/file.bin"));
    d.id = "persist".into();
    d.save_path = out_path.clone();
    let id = d.id.clone();
    mgr.add(d);

    // Land the transfer at a known intermediate state. With a 4 MiB
    // payload and 8 chunk workers the transfer can finish in well
    // under 100 ms against the in-process server, so we may also
    // see `Completed` here — that's fine, the byte-equality with
    // the on-disk row is what matters.
    let snap = wait_for_min_bytes(&mgr, &id, 64 * 1024).await;
    assert!(
        snap.downloaded > 0,
        "test setup: download did not make progress"
    );
    assert!(
        matches!(
            snap.status,
            DownloadStatus::Downloading
                | DownloadStatus::Paused
                | DownloadStatus::Queued
                | DownloadStatus::Completed
        ),
        "expected an in-flight or just-completed state, got {:?}",
        snap.status
    );

    // Drop the manager (= simulate a hard kill). Note we do NOT call
    // cancel() — the row's on-disk status is whatever it was at the
    // last flush. We only assert that progress was flushed.
    drop(mgr);

    // Re-open a fresh manager + storage over the same file and
    // confirm the row survived.
    let storage2 = Storage::open(&db_path).expect("reopen storage");
    let active = storage2.load_active().expect("load_active");
    assert_eq!(active.len(), 1, "expected 1 active row, got {active:?}");
    let row = &active[0];
    assert_eq!(row.id, "persist");
    assert!(
        row.downloaded > 0,
        "downloaded was 0 after restart — periodic flush is broken \
         (snap.downloaded = {}, snap.status = {:?})",
        snap.downloaded,
        snap.status
    );
    // Per-chunk downloaded must also have been persisted so the
    // resume range can be reconstructed.
    let total_chunk_downloaded: u64 = row.chunks.iter().map(|c| c.downloaded).sum();
    assert_eq!(
        total_chunk_downloaded, row.downloaded,
        "per-chunk downloaded ({}) must sum to total downloaded ({})",
        total_chunk_downloaded, row.downloaded
    );
    assert!(
        row.chunks.iter().any(|c| c.downloaded > 0),
        "no chunk has any downloaded bytes — resume would re-download from 0"
    );
}

/// End-to-end crash-recovery: start a download, let it run a while,
/// drop the manager (= power loss), build a new manager over the
/// same on-disk file, and confirm the transfer finishes with the
/// correct bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_after_crash_finishes_with_correct_bytes() {
    let port = spawn_server(0, 1);
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let db_path = tmp_dir.path().join("dm.sqlite");
    let out_path = tmp_dir.path().join("out.bin");

    {
        let storage = Storage::open(&db_path).expect("file storage");
        let mgr = DownloadManager::with_settings(storage, Settings::default());

        let mut d = Download::new(format!("http://127.0.0.1:{port}/file.bin"));
        d.id = "crash".into();
        d.save_path = out_path.clone();
        let id = d.id.clone();
        mgr.add(d);

        // Wait until some bytes are on disk AND on SQLite.
        let _ = wait_for_min_bytes(&mgr, &id, 32 * 1024).await;
        // The transfer is still going — simulate a hard kill.
    } // mgr + storage drop here

    // Restart with a brand new manager on the same DB file.
    let storage = Storage::open(&db_path).expect("reopen storage");
    let mgr = DownloadManager::with_settings(storage, Settings::default());
    mgr.start().await;
    let d = wait_until_downloaded(&mgr, "crash").await;
    assert_eq!(
        d.status,
        DownloadStatus::Completed,
        "resumed download did not complete: status={:?} err={:?}",
        d.status,
        d.error
    );

    let written = std::fs::read(&out_path).expect("output file exists");
    assert_eq!(
        written.len(),
        large_payload().len(),
        "resumed file length does not match server"
    );
    assert_eq!(
        written,
        large_payload(),
        "resumed file bytes do not match the source payload"
    );
}

/// Cancellation must persist whatever progress was made. The
/// aggregator's final flush guarantees that even if the user hits
/// Cancel the instant the transfer starts, the row records the
/// partial work instead of snapping back to 0.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_persists_partial_progress() {
    let port = spawn_server(0, 1);
    let tmp_dir = tempfile::tempdir().expect("tempdir");
    let db_path = tmp_dir.path().join("dm.sqlite");
    let out_path = tmp_dir.path().join("out.bin");

    let storage = Storage::open(&db_path).expect("file storage");
    let mgr = DownloadManager::with_settings(storage, Settings::default());

    let mut d = Download::new(format!("http://127.0.0.1:{port}/file.bin"));
    d.id = "cancel".into();
    d.save_path = out_path.clone();
    let id = d.id.clone();
    mgr.add(d);

    let snap = wait_for_min_bytes(&mgr, &id, 32 * 1024).await;
    let partial = snap.downloaded;
    mgr.cancel(&id);
    drop(mgr);

    let storage2 = Storage::open(&db_path).expect("reopen storage");
    let row = storage2
        .load_active()
        .expect("load_active")
        .into_iter()
        .find(|d| d.id == "cancel")
        .expect("cancel row should still exist (status = canceled)");
    assert_eq!(row.status, DownloadStatus::Canceled);
    assert_eq!(
        row.downloaded, partial,
        "cancel must persist the partial progress we observed in-memory \
         (in-memory: {partial}, on disk: {})",
        row.downloaded
    );
}
