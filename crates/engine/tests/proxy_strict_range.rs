//! Repro / regression test for the "downloading failure" reported with
//! CDN-proxy URLs like `cdn.akaere.online/...`. The proxy advertises no
//! `Content-Length` and no `Accept-Ranges` on HEAD, and then **rejects
//! any `Range:` request** with `416 Range Not Satisfiable` because it
//! doesn't know the upstream's total size. The engine's existing
//! single-chunk "unknown size" path (`bytes=0-`) therefore fails on the
//! first chunk GET.
//!
//! This test pins the new behaviour: when the chunk GET returns 416
//! (or any 4xx with the body still consumable), the engine must
//! **fall back to a no-`Range` GET** that streams the full body via
//! `Transfer-Encoding: chunked` (or reads to EOF), and the download
//! must complete with the file on disk matching the server's payload.
//!
//! The `truncated_body_marks_error` test pins the user-reported
//! "879 B downloaded, marked Completed" failure: when an open-ended
//! transfer closes the body stream with fewer bytes than
//! `OPEN_ENDED_MIN_BYTES` (a proxy-truncation symptom), the engine
//! must mark the download `Error` with a clear message, not silently
//! call it `Completed`.

use dm_engine::manager::DownloadManager;
use dm_engine::model::{Download, DownloadStatus, Settings};
use dm_engine::storage::Storage;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

const PAYLOAD_SIZE: usize = 8 * 1024 * 1024; // 8 MiB

/// Two-mode mock that mirrors the real proxy behaviour we've
/// observed: `/?strict` 416s on `Range:` and streams the full
/// 8 MiB body on a plain GET; `/?truncated` 416s on `Range:` and
/// then **closes the plain GET body with a 879 B stub** — the
/// "Chrome raced us and the proxy aborted the upstream" failure
/// mode.
fn spawn_two_mode() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if let Ok(s) = stream {
                let mut s = s;
                std::thread::spawn(move || handle_two_mode(&mut s));
            }
        }
    });
    port
}

fn handle_two_mode(s: &mut std::net::TcpStream) {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if Instant::now() > deadline {
            return;
        }
        match s.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                // Destructure the single-byte buffer so the linter
                // doesn't complain about indexing a `[u8; 1]`.
                let [b] = byte;
                buf.push(b);
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
    let method = request_line.split_whitespace().next().unwrap_or("").to_string();
    let path = request_line.split_whitespace().nth(1).unwrap_or("").to_string();
    let has_range = lines.any(|l| l.to_ascii_lowercase().starts_with("range:"));

    if method.eq_ignore_ascii_case("HEAD") {
        let _ = write!(
            s,
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n"
        );
        return;
    }

    if has_range {
        // 416 — proxy doesn't know the upstream size, so it can't
        // honour the Range request.
        let body = b"Range Not Satisfiable";
        let _ = write!(
            s,
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = s.write_all(body);
        return;
    }

    // Plain GET → mode-dependent.
    if path.contains("truncated") {
        // The proxy is supposed to send a 142 MB file but Chrome's
        // own download was racing DM's; the proxy aborted the
        // upstream connection and forwarded a tiny stub body to
        // DM. We send 879 B (the exact size from the bug report) and
        // close the connection cleanly.
        let stub_len: usize = 879;
        let stub = vec![b'X'; stub_len];
        let _ = write!(
            s,
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
        );
        let _ = write!(s, "{:x}\r\n", stub_len);
        let _ = s.write_all(&stub);
        let _ = write!(s, "\r\n0\r\n\r\n");
        return;
    }

    // Plain GET → 200 + chunked full body.
    let _ = write!(
        s,
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    );
    let mut n = 0;
    while n < PAYLOAD_SIZE {
        let chunk = std::cmp::min(64 * 1024, PAYLOAD_SIZE - n);
        let _ = write!(s, "{:x}\r\n", chunk);
        let body = vec![b'X'; chunk];
        let _ = s.write_all(&body);
        let _ = write!(s, "\r\n");
        let _ = s.flush();
        n += chunk;
    }
    let _ = write!(s, "0\r\n\r\n");
}

async fn wait_for_terminal(mgr: &DownloadManager, id: &str) -> Download {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let d = mgr.get(id).expect("download exists while waiting");
        if matches!(
            d.status,
            DownloadStatus::Completed | DownloadStatus::Error | DownloadStatus::Canceled
        ) {
            return d;
        }
        if Instant::now() > deadline {
            panic!(
                "download {id} did not reach terminal state within 20s (status: {:?})",
                d.status
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proxy_416_triggers_fallback_to_plain_get() {
    let port = spawn_two_mode();
    let tmp = tempfile::tempdir().expect("tempdir");
    let out_path = tmp.path().join("out.bin");

    let storage = Storage::open_memory().expect("memory storage");
    let mgr = DownloadManager::with_settings(storage, Settings::default());

    let mut d = Download::new(format!("http://127.0.0.1:{port}/file.bin"));
    d.id = "proxy".into();
    d.save_path = out_path.clone();
    let id = d.id.clone();
    mgr.add(d);

    let snap = wait_for_terminal(&mgr, &id).await;
    assert_eq!(
        snap.status,
        DownloadStatus::Completed,
        "proxy-416 path should fall back to a plain GET and complete, \
         got status={:?} err={:?}",
        snap.status,
        snap.error
    );
    let written = std::fs::read(&out_path).expect("output file exists");
    assert_eq!(
        written.len(),
        PAYLOAD_SIZE,
        "file length should match the streamed body"
    );
    // Sanity: the bytes should all be 'X'.
    assert!(written.iter().all(|&b| b == b'X'));
}

/// Regression test for the user-reported "879 B downloaded, marked
/// Completed" failure: when the proxy truncates the body to a tiny
/// stub (the symptom of Chrome racing the DM copy and the upstream
/// connection being killed), the engine must mark the open-ended
/// transfer `Error`, not `Completed`. The user gets a clear error
/// message instead of a fake-success.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn truncated_body_marks_error_not_completed() {
    let port = spawn_two_mode();
    let tmp = tempfile::tempdir().expect("tempdir");
    let out_path = tmp.path().join("out.bin");

    let storage = Storage::open_memory().expect("memory storage");
    let mgr = DownloadManager::with_settings(storage, Settings::default());

    let mut d = Download::new(format!(
        "http://127.0.0.1:{port}/file.bin?truncated"
    ));
    d.id = "truncated".into();
    d.save_path = out_path.clone();
    let id = d.id.clone();
    mgr.add(d);

    let snap = wait_for_terminal(&mgr, &id).await;
    assert_eq!(
        snap.status,
        DownloadStatus::Error,
        "an open-ended transfer that received only {} B should be Error, \
         not Completed (proxy truncation symptom — the upstream \
         connection was almost certainly aborted by the proxy). \
         Got status={:?} err={:?}",
        879,
        snap.status,
        snap.error
    );
    let err_msg = snap
        .error
        .as_deref()
        .unwrap_or("<no error message>");
    assert!(
        err_msg.contains("truncated") || err_msg.contains("879"),
        "error message should explain the truncation: got {err_msg:?}"
    );
    // We should NOT have left a misleading 879 B file on disk
    // marked Completed.
    assert_ne!(
        snap.status,
        DownloadStatus::Completed,
        "the bug from the report: 879 B must NOT be marked Completed"
    );
}
