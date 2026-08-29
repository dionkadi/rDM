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

// 256 KiB of a deterministic, easily-verifiable pattern.
fn payload() -> Vec<u8> {
    let mut v = Vec::with_capacity(256 * 1024);
    for i in 0..(256 * 1024) {
        v.push((i as u8).wrapping_mul(31).wrapping_add(7));
    }
    v
}

/// 0 = full Range support, 1 = claims no ranges (fallback path).
fn spawn_server(mode: u8) -> u16 {
    let data = payload();
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
        }
        None => {
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                data.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(data);
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
    let port = spawn_server(0);
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
    let port = spawn_server(1);
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
    let port = spawn_server(0);
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
