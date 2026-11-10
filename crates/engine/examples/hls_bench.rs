//! HLS stress + throughput benchmark.
//!
//! ```bash
//! cargo run --release --example hls_bench
//! ```
//!
//! Three scenarios, all against an in-process HTTP server so the numbers
//! are the engine's, not the network's:
//!
//! * **clear** — 300 × 512 KiB MPEG-TS segments (150 MiB), the shape of a
//!   normal VOD grab.
//! * **aes-128** — the same segment count at 256 KiB, AES-128 encrypted,
//!   to isolate the cost of decryption (buffered per segment).
//! * **many-small** — 2000 × 8 KiB segments (16 MiB): per-segment
//!   overhead (one request each), which is what a finely-segmented
//!   playlist looks like.
//!
//! Each run also hashes the output and compares it with the expected
//! concatenation, so a throughput number can never be reported for a
//! corrupt file.

use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use dm_engine::manager::DownloadManager;
use dm_engine::model::{Download, DownloadStatus};
use dm_engine::storage::Storage;
use sha2::{Digest, Sha256};

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

const KEY: [u8; 16] = [0x5Au8; 16];

fn sequence_iv(sequence: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&sequence.to_be_bytes());
    iv
}

fn encrypt(plain: &[u8], iv: [u8; 16]) -> Vec<u8> {
    let mut buf = vec![0u8; plain.len() + 16];
    buf[..plain.len()].copy_from_slice(plain);
    let encryptor = Aes128CbcEnc::new_from_slices(&KEY, &iv).expect("key/iv");
    let written = encryptor
        .encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len())
        .expect("encrypt")
        .len();
    buf.truncate(written);
    buf
}

/// Deterministic segment body: `(index, byte)` → value, so the server and
/// the verifier can generate the same bytes without sharing memory.
fn segment_bytes(index: usize, len: usize) -> Vec<u8> {
    (0..len)
        .map(|j| (index.wrapping_mul(31).wrapping_add(j) % 251) as u8)
        .collect()
}

struct Scenario {
    name: &'static str,
    segments: usize,
    segment_len: usize,
    encrypted: bool,
}

impl Scenario {
    fn total_bytes(&self) -> u64 {
        // Encrypted segments gain up to one AES block of PKCS#7 padding.
        let per = if self.encrypted {
            (self.segment_len / 16 + 1) * 16
        } else {
            self.segment_len
        };
        (per * self.segments) as u64
    }
}

/// Serve one scenario on an ephemeral port.
fn serve(scenario: &Scenario) -> u16 {
    let playlist = {
        let mut text = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n");
        text.push_str("#EXT-X-MEDIA-SEQUENCE:0\n");
        if scenario.encrypted {
            text.push_str("#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n");
        }
        for i in 0..scenario.segments {
            text.push_str(&format!("#EXTINF:4.000000,\nseg_{i}.ts\n"));
        }
        text.push_str("#EXT-X-ENDLIST\n");
        text
    };
    // Pre-encrypt once: doing it per request would measure the server's
    // AES speed, not the downloader's.
    let bodies: Arc<HashMap<String, Vec<u8>>> = Arc::new(
        (0..scenario.segments)
            .map(|i| {
                let plain = segment_bytes(i, scenario.segment_len);
                let body = if scenario.encrypted {
                    encrypt(&plain, sequence_iv(i as u64))
                } else {
                    plain
                };
                (format!("/seg_{i}.ts"), body)
            })
            .collect(),
    );
    let playlist = Arc::new(playlist);
    let key = Arc::new(KEY.to_vec());

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut socket) = stream else { break };
            let bodies = Arc::clone(&bodies);
            let playlist = Arc::clone(&playlist);
            let key = Arc::clone(&key);
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let mut byte = [0u8; 1];
                while socket.read(&mut byte).map(|n| n == 1).unwrap_or(false) {
                    buf.push(byte[0]);
                    if buf.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&buf);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let (content_type, body) = if path.ends_with(".m3u8") {
                    ("application/vnd.apple.mpegurl", playlist.as_bytes().to_vec())
                } else if path.ends_with("key.bin") {
                    ("application/octet-stream", key.as_ref().clone())
                } else {
                    (
                        "video/mp2t",
                        bodies.get(path).cloned().unwrap_or_default(),
                    )
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes());
                let _ = socket.write_all(&body);
            });
        }
    });
    port
}

fn expected_digest(scenario: &Scenario) -> String {
    let mut hasher = Sha256::new();
    for i in 0..scenario.segments {
        hasher.update(segment_bytes(i, scenario.segment_len));
    }
    hex::encode(hasher.finalize())
}

fn file_digest(path: &PathBuf) -> String {
    let mut file = std::fs::File::open(path).expect("open output");
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).expect("read output");
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    hex::encode(hasher.finalize())
}

async fn run_one(scenario: &Scenario) -> (Duration, bool) {
    let port = serve(scenario);
    let dir = std::env::temp_dir().join(format!(
        "dm-hls-bench-{}-{}",
        std::process::id(),
        scenario.name.replace('-', "_")
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let out = dir.join("out.ts");
    let _ = std::fs::remove_file(&out);

    let mgr = DownloadManager::new(Storage::open_memory().expect("storage"));
    let mut d = Download::new(format!("http://127.0.0.1:{port}/index.m3u8"));
    d.id = "bench".into();
    d.filename = "out.ts".into();
    d.save_path = out.clone();
    mgr.add(d);

    let started = Instant::now();
    loop {
        let current = mgr.get("bench").expect("row");
        match current.status {
            DownloadStatus::Completed => break,
            DownloadStatus::Error => panic!("{:?}", current.error),
            _ => tokio::time::sleep(Duration::from_millis(5)).await,
        }
    }
    let elapsed = started.elapsed();

    let ok = file_digest(&out) == expected_digest(scenario);
    let _ = std::fs::remove_dir_all(&dir);
    (elapsed, ok)
}

#[tokio::main]
async fn main() {
    println!(
        "{:<12} {:>9} {:>8} {:>10} {:>12} {:>10} {:>8}",
        "scenario", "segments", "seg size", "total", "elapsed", "MB/s", "hash"
    );
    for scenario in [
        Scenario {
            name: "clear",
            segments: 300,
            segment_len: 512 * 1024,
            encrypted: false,
        },
        Scenario {
            name: "aes-128",
            segments: 300,
            segment_len: 256 * 1024,
            encrypted: true,
        },
        Scenario {
            name: "many-small",
            segments: 2000,
            segment_len: 8 * 1024,
            encrypted: false,
        },
    ] {
        let (elapsed, ok) = run_one(&scenario).await;
        let total = scenario.total_bytes();
        let mbps = total as f64 / 1024.0 / 1024.0 / elapsed.as_secs_f64().max(1e-9);
        let per_segment_ms = elapsed.as_secs_f64() * 1000.0 / scenario.segments as f64;
        println!(
            "{:<12} {:>9} {:>6} KiB {:>7} MiB {:>9.2} s {:>9.1} {:>8}",
            scenario.name,
            scenario.segments,
            scenario.segment_len / 1024,
            total / 1024 / 1024,
            elapsed.as_secs_f64(),
            mbps,
            if ok { "ok" } else { "MISMATCH" }
        );
        println!(
            "             {:<9} {:>6.2} ms/segment, {:.0} req/s",
            "",
            per_segment_ms,
            1000.0 / per_segment_ms.max(1e-9)
        );
        if !ok {
            std::process::exit(1);
        }
    }
}
