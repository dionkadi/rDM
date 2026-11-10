//! HLS end-to-end: a `.m3u8` grab must produce media, not the playlist.
//!
//! The reported bug: grabbing `…/1080p/stream.m3u8` saved the playlist
//! text (`#EXTM3U … segment_000.ts`) under the user's filename. These
//! tests drive the real `DownloadManager` against a local HTTP server
//! that serves a master playlist, a media playlist and segment bodies,
//! and assert on the bytes that end up on disk.

use dm_engine::manager::DownloadManager;
use dm_engine::model::{Download, DownloadStatus};
use dm_engine::storage::Storage;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// One served path: content type + body.
struct Asset {
    content_type: &'static str,
    body: Vec<u8>,
}

/// Serve `assets` on an ephemeral port. Each request is answered on its
/// own thread, so the engine's connection pool is not serialised.
fn serve(assets: HashMap<String, Asset>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let assets = std::sync::Arc::new(assets);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut socket) = stream else { break };
            let assets = assets.clone();
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let mut byte = [0u8; 1];
                let deadline = Instant::now() + Duration::from_secs(5);
                while Instant::now() < deadline {
                    match socket.read(&mut byte) {
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
                let request = String::from_utf8_lossy(&buf);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let (status, content_type, body) = match assets.get(path) {
                    Some(asset) => ("200 OK", asset.content_type, asset.body.clone()),
                    None => ("404 Not Found", "text/plain", b"not found".to_vec()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes());
                let _ = socket.write_all(&body);
            });
        }
    });
    port
}

fn playlist(body: &str) -> Asset {
    Asset {
        content_type: "application/vnd.apple.mpegurl",
        body: body.as_bytes().to_vec(),
    }
}

fn segment(body: Vec<u8>) -> Asset {
    Asset {
        content_type: "video/mp2t",
        body,
    }
}

fn add(mgr: &DownloadManager, dir: &Path, url: String, name: &str) -> String {
    let mut d = Download::new(url);
    d.filename = name.to_string();
    d.save_path = dir.join(name);
    let id = d.id.clone();
    mgr.add(d);
    id
}

/// Poll until the download leaves the active states (or the timeout).
async fn wait_done(mgr: &DownloadManager, id: &str, timeout: Duration) -> Download {
    let deadline = Instant::now() + timeout;
    loop {
        let d = mgr.get(id).expect("row still exists");
        if matches!(
            d.status,
            DownloadStatus::Completed | DownloadStatus::Error | DownloadStatus::Canceled
        ) {
            return d;
        }
        assert!(Instant::now() < deadline, "download did not finish: {d:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn concat(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.iter().flat_map(|p| p.iter().copied()).collect()
}

#[tokio::test]
async fn a_grabbed_playlist_downloads_its_segments_not_the_text() {
    let seg_a: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
    let seg_b: Vec<u8> = (0..2048u32).map(|i| (i % 97) as u8).collect();
    let seg_c = b"third-segment".to_vec();

    let mut assets = HashMap::new();
    // Master with two variants: the 1080p one must win.
    assets.insert(
        "/master.m3u8".to_string(),
        playlist(
            "#EXTM3U\n\
             #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=1280x720\n\
             v720/index.m3u8\n\
             #EXT-X-STREAM-INF:BANDWIDTH=2400000,RESOLUTION=1920x1080\n\
             v1080/index.m3u8\n",
        ),
    );
    assets.insert(
        "/v1080/index.m3u8".to_string(),
        playlist(
            "#EXTM3U\n\
             #EXT-X-VERSION:3\n\
             #EXT-X-TARGETDURATION:6\n\
             #EXT-X-MEDIA-SEQUENCE:0\n\
             #EXTINF:6.000000,\n\
             segment_000.ts\n\
             #EXTINF:6.000000,\n\
             segment_001.ts\n\
             #EXTINF:6.000000,\n\
             segment_002.ts\n\
             #EXT-X-ENDLIST\n",
        ),
    );
    assets.insert("/v1080/segment_000.ts".into(), segment(seg_a.clone()));
    assets.insert("/v1080/segment_001.ts".into(), segment(seg_b.clone()));
    assets.insert("/v1080/segment_002.ts".into(), segment(seg_c.clone()));
    // A 720p variant that must NOT be fetched.
    assets.insert(
        "/v720/index.m3u8".to_string(),
        playlist("#EXTM3U\n#EXTINF:1,\nmissing.ts\n#EXT-X-ENDLIST\n"),
    );

    let port = serve(assets);
    let dir = tempfile::tempdir().unwrap();
    let mgr = DownloadManager::new(Storage::open_memory().unwrap());
    let id = add(
        &mgr,
        dir.path(),
        format!("http://127.0.0.1:{port}/master.m3u8"),
        "stream.m3u8",
    );

    let done = wait_done(&mgr, &id, Duration::from_secs(30)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);

    // The playlist text must never be the payload.
    let ts = dir.path().join("stream.ts");
    let bytes = std::fs::read(&ts).expect("the concatenated .ts must exist");
    assert_eq!(bytes, concat(&[seg_a, seg_b, seg_c]));
    assert!(!bytes.starts_with(b"#EXTM3U"), "the playlist was saved as media");

    // The `.ts` is the source; with ffmpeg the row follows the remuxed
    // MP4 (the bytes here are not a real TS stream, so the remux is
    // expected to fail — what matters is that failure is not fatal).
    assert_eq!(done.downloaded, bytes.len() as u64);
}

#[tokio::test]
async fn a_live_playlist_is_refused_with_a_reason() {
    let mut assets = HashMap::new();
    assets.insert(
        "/live.m3u8".to_string(),
        playlist("#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\na.ts\n"),
    );
    assets.insert("/a.ts".into(), segment(b"x".to_vec()));
    let port = serve(assets);

    let dir = tempfile::tempdir().unwrap();
    let mgr = DownloadManager::new(Storage::open_memory().unwrap());
    let id = add(
        &mgr,
        dir.path(),
        format!("http://127.0.0.1:{port}/live.m3u8"),
        "live.m3u8",
    );

    let done = wait_done(&mgr, &id, Duration::from_secs(10)).await;
    assert_eq!(done.status, DownloadStatus::Error);
    let error = done.error.unwrap_or_default();
    assert!(error.contains("live HLS playlist"), "{error}");
    // Nothing pretends to be a finished file.
    assert!(!dir.path().join("live.ts").exists());
}

#[tokio::test]
async fn an_encrypted_playlist_fails_loudly_instead_of_writing_ciphertext() {
    let mut assets = HashMap::new();
    assets.insert(
        "/enc.m3u8".to_string(),
        playlist(
            "#EXTM3U\n\
             #EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n\
             #EXTINF:6,\n\
             a.ts\n\
             #EXT-X-ENDLIST\n",
        ),
    );
    assets.insert("/a.ts".into(), segment(b"ciphertext".to_vec()));
    let port = serve(assets);

    let dir = tempfile::tempdir().unwrap();
    let mgr = DownloadManager::new(Storage::open_memory().unwrap());
    let id = add(
        &mgr,
        dir.path(),
        format!("http://127.0.0.1:{port}/enc.m3u8"),
        "enc.m3u8",
    );

    let done = wait_done(&mgr, &id, Duration::from_secs(10)).await;
    assert_eq!(done.status, DownloadStatus::Error);
    let error = done.error.unwrap_or_default();
    assert!(error.contains("encrypted"), "{error}");
    assert!(!dir.path().join("enc.ts").exists());
}

/// Real-MPEG-TS variant: exercises the remux step, so it only runs
/// where ffmpeg exists (CI installs it; a dev box may not have it).
#[tokio::test]
async fn a_real_ts_stream_is_remuxed_to_mp4() {
    if dm_engine::media::which("ffmpeg").is_none() {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let full = dir.path().join("full.ts");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=128x72:rate=5",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-c:v",
            "mpeg2video",
            "-c:a",
            "mp2",
            "-f",
            "mpegts",
        ])
        .arg(&full)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("spawn ffmpeg");
    assert!(status.success(), "could not synthesise a TS stream");

    // Split on 188-byte TS packet boundaries and serve the halves.
    let data = std::fs::read(&full).unwrap();
    let split = (data.len() / 2 / 188) * 188;
    assert!(split > 0 && split < data.len(), "stream too short to split");
    let first = data[..split].to_vec();
    let second = data[split..].to_vec();

    let mut assets = HashMap::new();
    assets.insert(
        "/stream.m3u8".to_string(),
        playlist("#EXTM3U\n#EXTINF:1,\npart0.ts\n#EXTINF:1,\npart1.ts\n#EXT-X-ENDLIST\n"),
    );
    assets.insert("/part0.ts".into(), segment(first));
    assets.insert("/part1.ts".into(), segment(second));
    let port = serve(assets);

    let out_dir = tempfile::tempdir().unwrap();
    let mgr = DownloadManager::new(Storage::open_memory().unwrap());
    let id = add(
        &mgr,
        out_dir.path(),
        format!("http://127.0.0.1:{port}/stream.m3u8"),
        "stream.m3u8",
    );

    let done = wait_done(&mgr, &id, Duration::from_secs(60)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    assert_eq!(done.filename, "stream.mp4", "the row follows the remuxed file");

    let mp4 = PathBuf::from(&done.save_path);
    assert!(mp4.is_file(), "remuxed file missing: {}", mp4.display());
    assert!(std::fs::metadata(&mp4).unwrap().len() > 0);
    // The TS is kept as the source (same contract as the pair merge).
    assert!(out_dir.path().join("stream.ts").is_file());
}

