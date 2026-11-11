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

use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
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

fn playlist(body: impl AsRef<str>) -> Asset {
    Asset {
        content_type: "application/vnd.apple.mpegurl",
        body: body.as_ref().as_bytes().to_vec(),
    }
}

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

/// AES-128-CBC + PKCS#7, the way an HLS packager encrypts segments.
fn encrypt(plain: &[u8], key: &[u8; 16], iv: [u8; 16]) -> Vec<u8> {
    let mut buf = vec![0u8; plain.len() + 16];
    buf[..plain.len()].copy_from_slice(plain);
    let encryptor = Aes128CbcEnc::new_from_slices(key, &iv).expect("key/iv");
    let written = encryptor
        .encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len())
        .expect("encrypt")
        .len();
    buf.truncate(written);
    buf
}

/// `EXT-X-KEY` without an `IV` uses the media sequence number.
fn sequence_iv(sequence: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&sequence.to_be_bytes());
    iv
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
async fn aes_128_segments_are_decrypted_before_they_are_written() {
    let key = [0x42u8; 16];
    let plain_a = b"first clear segment".to_vec();
    let plain_b = b"second clear segment, a bit longer".to_vec();
    let cipher_a = encrypt(&plain_a, &key, sequence_iv(0));
    let cipher_b = encrypt(&plain_b, &key, sequence_iv(1));

    let mut assets = HashMap::new();
    assets.insert(
        "/enc.m3u8".to_string(),
        playlist(
            "#EXTM3U\n\
             #EXT-X-MEDIA-SEQUENCE:0\n\
             #EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n\
             #EXTINF:6,\n\
             a.ts\n\
             #EXTINF:6,\n\
             b.ts\n\
             #EXT-X-ENDLIST\n",
        ),
    );
    assets.insert(
        "/key.bin".to_string(),
        Asset {
            content_type: "application/octet-stream",
            body: key.to_vec(),
        },
    );
    assets.insert("/a.ts".into(), segment(cipher_a.clone()));
    assets.insert("/b.ts".into(), segment(cipher_b.clone()));
    let port = serve(assets);

    let dir = tempfile::tempdir().unwrap();
    let mgr = DownloadManager::new(Storage::open_memory().unwrap());
    let id = add(
        &mgr,
        dir.path(),
        format!("http://127.0.0.1:{port}/enc.m3u8"),
        "enc.m3u8",
    );

    let done = wait_done(&mgr, &id, Duration::from_secs(30)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);

    let written = std::fs::read(dir.path().join("enc.ts")).expect("output");
    assert_eq!(
        written,
        concat(&[plain_a, plain_b]),
        "segments must be decrypted, not concatenated as ciphertext"
    );
    assert_ne!(
        written,
        concat(&[cipher_a, cipher_b]),
        "the ciphertext must never reach the output file"
    );
}

#[tokio::test]
async fn a_sample_aes_playlist_is_refused_before_any_segment_is_fetched() {
    let mut assets = HashMap::new();
    assets.insert(
        "/skd.m3u8".to_string(),
        playlist(
            "#EXTM3U\n\
             #EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://key\"\n\
             #EXTINF:6,\n\
             a.ts\n\
             #EXT-X-ENDLIST\n",
        ),
    );
    assets.insert("/a.ts".into(), segment(b"whatever".to_vec()));
    let port = serve(assets);

    let dir = tempfile::tempdir().unwrap();
    let mgr = DownloadManager::new(Storage::open_memory().unwrap());
    let id = add(
        &mgr,
        dir.path(),
        format!("http://127.0.0.1:{port}/skd.m3u8"),
        "skd.m3u8",
    );

    let done = wait_done(&mgr, &id, Duration::from_secs(10)).await;
    assert_eq!(done.status, DownloadStatus::Error);
    let error = done.error.unwrap_or_default();
    assert!(error.contains("SAMPLE-AES"), "{error}");
    assert!(!dir.path().join("skd.ts").exists());
}

/// Deterministic body so 120 segments can be generated without keeping
/// 2 MiB of fixtures in the source.
fn pattern(index: usize, len: usize) -> Vec<u8> {
    (0..len)
        .map(|j| (index.wrapping_mul(31).wrapping_add(j) % 251) as u8)
        .collect()
}

/// Stress: many segments must come out in playlist order, byte for byte.
/// This is the property a "throughput" number is meaningless without —
/// a fast download that reorders or drops a segment is a corrupt file.
#[tokio::test]
async fn many_segments_are_concatenated_in_order() {
    const N: usize = 120;
    const SEG: usize = 16 * 1024;
    let mut assets = HashMap::new();
    let mut playlist_text = String::from("#EXTM3U\n#EXT-X-TARGETDURATION:4\n");
    let mut expected = Vec::new();
    for i in 0..N {
        let body = pattern(i, SEG);
        playlist_text.push_str(&format!("#EXTINF:4,\nseg_{i}.ts\n"));
        assets.insert(format!("/seg_{i}.ts"), segment(body.clone()));
        expected.extend_from_slice(&body);
    }
    playlist_text.push_str("#EXT-X-ENDLIST\n");
    assets.insert("/index.m3u8".to_string(), playlist(playlist_text));

    let port = serve(assets);
    let dir = tempfile::tempdir().unwrap();
    let mgr = DownloadManager::new(Storage::open_memory().unwrap());
    let id = add(
        &mgr,
        dir.path(),
        format!("http://127.0.0.1:{port}/index.m3u8"),
        "many.m3u8",
    );

    let done = wait_done(&mgr, &id, Duration::from_secs(60)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    let written = std::fs::read(dir.path().join("many.ts")).expect("output");
    assert_eq!(written.len(), expected.len());
    assert_eq!(written, expected, "segments were reordered or dropped");
}

/// The same shape with AES-128 segments: decryption must not disturb the
/// order or the padding of the final segment.
#[tokio::test]
async fn many_encrypted_segments_are_decrypted_in_order() {
    const N: usize = 60;
    const SEG: usize = 8 * 1024;
    let key = [0x11u8; 16];
    let mut assets = HashMap::new();
    let mut playlist_text = String::from(
        "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n",
    );
    let mut expected = Vec::new();
    for i in 0..N {
        let plain = pattern(i, SEG);
        playlist_text.push_str(&format!("#EXTINF:4,\nseg_{i}.ts\n"));
        assets.insert(
            format!("/seg_{i}.ts"),
            segment(encrypt(&plain, &key, sequence_iv(i as u64))),
        );
        expected.extend_from_slice(&plain);
    }
    playlist_text.push_str("#EXT-X-ENDLIST\n");
    assets.insert("/index.m3u8".to_string(), playlist(playlist_text));
    assets.insert(
        "/key.bin".to_string(),
        Asset {
            content_type: "application/octet-stream",
            body: key.to_vec(),
        },
    );

    let port = serve(assets);
    let dir = tempfile::tempdir().unwrap();
    let mgr = DownloadManager::new(Storage::open_memory().unwrap());
    let id = add(
        &mgr,
        dir.path(),
        format!("http://127.0.0.1:{port}/index.m3u8"),
        "encmany.m3u8",
    );

    let done = wait_done(&mgr, &id, Duration::from_secs(60)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);
    let written = std::fs::read(dir.path().join("encmany.ts")).expect("output");
    assert_eq!(written, expected, "decryption must preserve plaintext order");
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


/// Serve `assets` with an artificial per-response delay and record the
/// highest number of requests that were ever in flight at the same time.
/// A serial segment walk can never exceed 1 here.
fn serve_with_delay(
    assets: HashMap<String, Asset>,
    delay: Duration,
) -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let assets = Arc::new(assets);
    let in_flight = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let reported = Arc::clone(&peak);

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut socket) = stream else { break };
            let assets = Arc::clone(&assets);
            let in_flight = Arc::clone(&in_flight);
            let peak = Arc::clone(&peak);
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

                // Count the request as in flight while the artificial
                // origin latency is being served.
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(delay);

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
                in_flight.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    (port, reported)
}

/// The reported bug: an HLS grab used one connection, so throughput was
/// one request's worth of latency at a time (~1 MB per second on a
/// segment-per-second playlist). Segments must be fetched concurrently,
/// while the bytes still land in playlist order.
#[tokio::test]
async fn segments_are_fetched_concurrently_and_stay_in_order() {
    use std::sync::atomic::Ordering;

    const COUNT: usize = 16;
    const SEG: usize = 4096;

    let mut assets = HashMap::new();
    let mut body = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n");
    let mut expected: Vec<u8> = Vec::new();
    for i in 0..COUNT {
        let bytes: Vec<u8> = (0..SEG).map(|n| ((n + i * 7) % 251) as u8).collect();
        assets.insert(format!("/segment_{i:03}.ts"), segment(bytes.clone()));
        expected.extend_from_slice(&bytes);
        body.push_str(&format!("#EXTINF:2.0,\nsegment_{i:03}.ts\n"));
    }
    body.push_str("#EXT-X-ENDLIST\n");
    assets.insert("/index.m3u8".to_string(), playlist(body));

    // 120 ms per response: 16 segments serial = ~1.9 s, 8 at a time = ~0.3 s.
    let (port, peak) = serve_with_delay(assets, Duration::from_millis(120));
    let dir = tempfile::tempdir().unwrap();
    let mgr = DownloadManager::new(Storage::open_memory().unwrap());
    let id = add(
        &mgr,
        dir.path(),
        format!("http://127.0.0.1:{port}/index.m3u8"),
        "parallel.m3u8",
    );

    let done = wait_done(&mgr, &id, Duration::from_secs(30)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);

    let out = std::fs::read(dir.path().join("parallel.ts")).expect("the .ts must exist");
    assert_eq!(out, expected, "parallel fetch must not reorder the segments");
    assert_eq!(done.downloaded, expected.len() as u64);

    // The row must be able to say "16 / 16" instead of "no chunks".
    let sp = done.segment_progress.expect("HLS rows carry a segment map");
    assert_eq!(sp.total, COUNT);
    assert_eq!(sp.done, COUNT, "every segment must be accounted for");
    assert_eq!(sp.active, 0, "nothing can be in flight once the file is done");

    let peak = peak.load(Ordering::SeqCst);
    assert!(
        peak >= 4,
        "segments were fetched {peak} at a time; an HLS grab should use several connections"
    );
}

/// The reported bug: the row said "Connecting" for the whole transfer,
/// because the HLS branch returns before `run_download`'s status
/// transition and nothing else set it until completion.
#[tokio::test]
async fn the_row_enters_downloading_before_the_segments_run() {
    use dm_engine::manager::DownloadEvent;
    use std::sync::{Arc, Mutex};

    let mut assets = HashMap::new();
    let mut body = String::from("#EXTM3U\n");
    for i in 0..6 {
        assets.insert(format!("/seg{i}.ts"), segment(vec![i as u8; 2048]));
        body.push_str(&format!("#EXTINF:2.0,\nseg{i}.ts\n"));
    }
    body.push_str("#EXT-X-ENDLIST\n");
    assets.insert("/index.m3u8".to_string(), playlist(body));

    let (port, _peak) = serve_with_delay(assets, Duration::from_millis(40));
    let dir = tempfile::tempdir().unwrap();
    let mgr = DownloadManager::new(Storage::open_memory().unwrap());

    let events: Arc<Mutex<Vec<DownloadStatus>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let events = Arc::clone(&events);
        mgr.set_event_sink(Arc::new(move |e: DownloadEvent| {
            if let DownloadEvent::StatusChanged(d) = e {
                events.lock().unwrap().push(d.status);
            }
        }));
    }

    let id = add(
        &mgr,
        dir.path(),
        format!("http://127.0.0.1:{port}/index.m3u8"),
        "status.m3u8",
    );
    let done = wait_done(&mgr, &id, Duration::from_secs(30)).await;
    assert_eq!(done.status, DownloadStatus::Completed, "{:?}", done.error);

    let seen = events.lock().unwrap().clone();
    assert_eq!(
        seen,
        vec![DownloadStatus::Connecting, DownloadStatus::Downloading],
        "the row must leave Connecting once the segments start, got {seen:?}"
    );
}
