//! Native-messaging host for DM.
//!
//! The browser extension launches this binary and exchanges Chrome
//! native-messaging frames (a 4-byte little-endian length prefix followed by a
//! UTF-8 JSON message) over stdin/stdout. Each incoming message may carry a
//! captured page (`html`) and/or a direct `url`; this host extracts media URLs
//! (the "video grabber" heuristic) and forwards every candidate to the running
//! DM app over a localhost TCP socket, which the app relays to `add_download`.
//!
//! Build/run: `cargo run -p dm-native-host` (no Tauri/webview needed).

use std::io::{Read, Write};

/// Port the DM app listens on for forwarded URLs. Override with `DM_NATIVE_HOST_PORT`.
pub const DEFAULT_PORT: u16 = 9157;

/// Read one Chrome native-messaging frame. Returns `None` at a clean EOF.
pub fn read_frame<R: Read>(r: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

/// Write one Chrome native-messaging frame.
pub fn write_frame<W: Write>(w: &mut W, msg: &[u8]) -> std::io::Result<()> {
    w.write_all(&(msg.len() as u32).to_le_bytes())?;
    w.write_all(msg)?;
    w.flush()
}

/// Media file extensions the grabber treats as downloadable.
const MEDIA_EXTS: &[&str] = &[
    ".mp4", ".webm", ".mkv", ".mov", ".flv", ".avi", ".ts", ".m4v", ".m4a", ".ogg", ".mp3",
];

/// Heuristically pull downloadable media URLs out of an HTML/JSON blob.
///
/// Scans quoted strings for `http(s)` URLs; keeps those ending in a media
/// extension, any URL containing `.m3u8` (HLS), or any URL containing a DASH
/// `manifest` fragment. This is intentionally heuristic (v1): full
/// site-specific HLS/DASH resolving is out of scope.
pub fn extract_media_urls(html: &str) -> Vec<String> {
    let mut found = std::collections::HashSet::new();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '"' || c == '\'' {
            let quote = c;
            let mut j = i + 1;
            let mut s = String::new();
            while j < bytes.len() {
                if (bytes[j] as char) == quote {
                    break;
                }
                s.push(bytes[j] as char);
                j += 1;
            }
            i = j + 1;
            if let Some(u) = maybe_media(&s) {
                found.insert(u);
            }
        } else {
            i += 1;
        }
    }
    let mut out: Vec<String> = found.into_iter().collect();
    out.sort();
    out
}

fn maybe_media(s: &str) -> Option<String> {
    if !s.starts_with("http://") && !s.starts_with("https://") {
        return None;
    }
    let low = s.to_ascii_lowercase();
    if low.contains(".m3u8") || (low.contains("manifest") && low.contains("mpd")) {
        return Some(s.to_string());
    }
    for ext in MEDIA_EXTS {
        if low.contains(ext) {
            return Some(s.to_string());
        }
    }
    None
}

/// Forward a single URL to the running DM app over the localhost socket.
pub fn forward_url(url: &str, port: u16) -> std::io::Result<()> {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))?;
    let payload = serde_json::json!({ "url": url }).to_string() + "\n";
    stream.write_all(payload.as_bytes())?;
    stream.flush()
}

fn handle_message(msg: &serde_json::Value, port: u16) {
    let mut urls: Vec<String> = Vec::new();
    if let Some(html) = msg.get("html").and_then(|v| v.as_str()) {
        urls.extend(extract_media_urls(html));
    }
    if let Some(arr) = msg.get("urls").and_then(|v| v.as_array()) {
        for u in arr {
            if let Some(s) = u.as_str() {
                if let Some(m) = maybe_media(s) {
                    urls.push(m);
                }
            }
        }
    }
    for key in ["url", "download", "src"] {
        if let Some(u) = msg.get(key).and_then(|v| v.as_str()) {
            if let Some(m) = maybe_media(u) {
                urls.push(m);
            }
        }
    }
    for u in urls {
        if forward_url(&u, port).is_err() {
            // App not listening (or unreachable) — drop silently; the extension
            // will retry on the next capture.
            eprintln!("dm-native-host: could not forward {u}");
        }
    }
}

fn main() {
    let port = std::env::var("DM_NATIVE_HOST_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let mut reader = std::io::stdin().lock();
    let mut out = std::io::stdout().lock();
    loop {
        let frame = match read_frame(&mut reader) {
            Ok(Some(f)) => f,
            Ok(None) | Err(_) => break,
        };
        if let Ok(msg) = serde_json::from_slice::<serde_json::Value>(&frame) {
            handle_message(&msg, port);
        }
        // Acknowledge so the browser keeps the channel open.
        let _ = write_frame(&mut out, br#"{"ok":true}"#);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trip() {
        let mut buf: Vec<u8> = Vec::new();
        write_frame(&mut buf, b"hello").unwrap();
        write_frame(&mut buf, br#"{"a":1}"#).unwrap();
        let mut cur = std::io::Cursor::new(buf);
        assert_eq!(read_frame(&mut cur).unwrap().unwrap().as_slice(), &b"hello"[..]);
        assert_eq!(
            read_frame(&mut cur).unwrap().unwrap().as_slice(),
            &br#"{"a":1}"#[..]
        );
        assert!(read_frame(&mut cur).unwrap().is_none());
    }

    #[test]
    fn extracts_video_and_hls() {
        let html = r#"
            <video src="https://cdn.example.com/clip.mp4" controls></video>
            <source src='https://cdn.example.com/clip.webm' type='video/webm'>
            <a href="https://stream.example.com/playlist.m3u8">HLS</a>
            <a href="https://example.com/page.html">ignore</a>
            <script>var m = "https://cdn.example.com/ad.mp3";</script>
        "#;
        let urls = extract_media_urls(html);
        assert!(urls.iter().any(|u| u.contains("clip.mp4")));
        assert!(urls.iter().any(|u| u.contains("clip.webm")));
        assert!(urls.iter().any(|u| u.contains("playlist.m3u8")));
        assert!(urls.iter().any(|u| u.contains("ad.mp3")));
        assert!(!urls.iter().any(|u| u.contains("page.html")));
    }

    #[test]
    fn ignores_non_media_and_non_http() {
        assert!(extract_media_urls(r#"<img src="preview.png">"#).is_empty());
        assert!(extract_media_urls(r#"<a href="/relative/path.mp4">x</a>"#).is_empty());
    }
}
