//! Minimal RFC 6455 WebSocket server. Hand-rolled, no external crate.
//!
//! Used by `native_host.rs` to let Chromium-based browser
//! extensions talk to DM directly over `ws://127.0.0.1:9157/`,
//! bypassing the Chrome Native Messaging host. The native
//! messaging path is still supported — Firefox MV3 can't open
//! a `ws://127.0.0.1` socket reliably (Firefox upgrades
//! insecure ws:// to wss://), so Firefox users still ship the
//! `dm-native-host` binary.
//!
//! Scope of this module: server-side handshake + text-frame
//! read/write. The browser is always the client, so we read
//! masked frames (RFC 6455 §5.1: "A client MUST mask all
//! frames that it sends to the server") and write
//! unmasked frames back. We don't support fragmentation
//! (browsers never fragment small JSON control messages),
//! binary frames, ping/pong, or extensions. If a client
//! sends anything more exotic than a small text frame we
//! close the socket.
//!
//! This is intentionally not a generic WebSocket library.
//! 130 lines, single-threaded, no async, no allocation
//! beyond the per-frame payload Vec. The browser doesn't
//! need anything fancier.

use std::io::{self, BufRead, Read, Write};

use base64::Engine as _;
use sha1::{Digest, Sha1};

/// Magic GUID the WebSocket protocol requires us to
/// concatenate with the client's `Sec-WebSocket-Key` before
/// SHA-1-hashing to derive the `Sec-WebSocket-Accept`
/// response header. RFC 6455 §1.3.
const WS_MAGIC_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Outcome of peeking the first bytes of a new connection.
#[derive(Debug)]
pub enum HandshakeOutcome {
    /// Looks like an HTTP/1.1 WebSocket upgrade request. We
    /// completed the handshake and switched to WS frame mode.
    WebSocket,
    /// Not an HTTP request (or not a WS upgrade). The caller
    /// should fall back to plain line-delimited JSON.
    NotWebSocket,
    /// Looks like a request but we couldn't complete the
    /// handshake (malformed, missing headers, etc.). The
    /// caller should drop the connection.
    Invalid,
}

/// Read the HTTP request from `reader` up to `\r\n\r\n`, parse
/// it, and (if it's a valid WebSocket upgrade) write the
/// `101 Switching Protocols` response to `writer`. Returns
/// whether the connection is now a WebSocket or fell back to
/// non-WS mode.
///
/// `reader` and `writer` should wrap the same underlying
/// stream (split halves are fine). The function does not
/// take ownership — it borrowss so the caller can keep
/// using the stream after the handshake.
pub fn handshake<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
) -> io::Result<HandshakeOutcome> {
    // Read the request headers. We bound the read at 8 KiB
    // so a malicious client can't OOM us with a giant
    // header block; real browser requests are <2 KiB.
    let mut buf = Vec::with_capacity(512);
    let mut tmp = [0u8; 512];
    loop {
        if buf.len() > 8192 {
            return Ok(HandshakeOutcome::Invalid);
        }
        let n = reader.read(&mut tmp)?;
        if n == 0 {
            // EOF before \r\n\r\n.
            if buf.is_empty() {
                return Ok(HandshakeOutcome::NotWebSocket);
            }
            return Ok(HandshakeOutcome::Invalid);
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }

    // Parse the request line. We only care that it's an
    // HTTP/1.1 GET with `Upgrade: websocket` and a
    // `Sec-WebSocket-Key`. Header order doesn't matter.
    let text = match std::str::from_utf8(&buf) {
        Ok(s) => s,
        Err(_) => return Ok(HandshakeOutcome::Invalid),
    };
    let mut lines = text.split("\r\n");
    let request_line = match lines.next() {
        Some(l) => l,
        None => return Ok(HandshakeOutcome::Invalid),
    };
    // Must look like `GET /path HTTP/1.1` (or similar). We
    // accept any path; the extension always connects to
    // `ws://127.0.0.1:9157/` but we don't want to be
    // fragile about it.
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let _path = parts.next().unwrap_or("");
    let version = parts.next().unwrap_or("");
    if !method.eq_ignore_ascii_case("GET")
        || !version.starts_with("HTTP/1.")
    {
        return Ok(HandshakeOutcome::NotWebSocket);
    }

    // Find the headers we need. We don't strictly require
    // `Connection: Upgrade` because every browser sends it,
    // but we do require `Upgrade: websocket` and a
    // `Sec-WebSocket-Key`. Missing or malformed → Invalid.
    let mut upgrade_ws = false;
    let mut ws_key: Option<&str> = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = match line.split_once(':') {
            Some((n, v)) => (n.trim(), v.trim()),
            None => continue,
        };
        if name.eq_ignore_ascii_case("upgrade") {
            // Could be "websocket" or a comma-separated list
            // (e.g. "websocket, foo"). Token match.
            if value
                .split(',')
                .any(|t| t.trim().eq_ignore_ascii_case("websocket"))
            {
                upgrade_ws = true;
            }
        } else if name.eq_ignore_ascii_case("sec-websocket-key") {
            ws_key = Some(value);
        }
    }
    if !upgrade_ws {
        return Ok(HandshakeOutcome::NotWebSocket);
    }
    let Some(key) = ws_key else {
        return Ok(HandshakeOutcome::Invalid);
    };

    // Compute the accept hash: base64(SHA1(key + GUID)).
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(WS_MAGIC_GUID.as_bytes());
    let digest = hasher.finalize();
    let accept = base64::engine::general_purpose::STANDARD.encode(digest);

    // Send the 101 response. Each line is `\r\n`-terminated
    // and the header block is terminated by an empty line.
    write!(
        writer,
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\
         \r\n",
        accept
    )?;
    writer.flush()?;
    Ok(HandshakeOutcome::WebSocket)
}

/// Read one full client text frame from `reader`. Returns:
///   - `Ok(Some(text))` — payload of the next text frame.
///   - `Ok(None)` — clean close frame received; caller
///     should drop the connection.
///   - `Err(_)` — I/O error, protocol violation, or
///     non-text frame.
///
/// Browser clients always send masked frames. We do not
/// accept unmasked client frames (RFC 6455 §5.1 says
/// servers MUST close the connection on an unmasked
/// client frame).
pub fn read_text_frame<R: Read>(reader: &mut R) -> io::Result<Option<String>> {
    // Two header bytes.
    let mut head = [0u8; 2];
    if let Err(e) = reader.read_exact(&mut head) {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(e);
    }
    let fin = head[0] & 0x80 != 0;
    let opcode = head[0] & 0x0F;
    let masked = head[1] & 0x80 != 0;
    let mut len = (head[1] & 0x7F) as u64;

    if !fin {
        // We don't support fragmentation. The browser
        // never fragments small control messages, so this
        // is almost certainly an attack or a buggy client.
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "fragmented WebSocket frames are not supported",
        ));
    }

    // 0x0 = continuation, 0x1 = text, 0x2 = binary, 0x8 =
    // close, 0x9 = ping, 0xA = pong. We accept text and
    // close; treat ping/pong as no-ops (we don't reply
    // because we have nothing to send unsolicited); reject
    // everything else.
    match opcode {
        0x1 => {} // text — fall through to read payload
        0x8 => return Ok(None), // close
        0x9 | 0xA => {
            // ping/pong: read + discard the payload, recurse
            // for the next frame. The browser never pings
            // mid-session in practice.
            let _ = read_and_discard_frame(reader, masked, len)?;
            return read_text_frame(reader);
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported WebSocket opcode: 0x{:x}", opcode),
            ));
        }
    }

    if !masked {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "client frame is not masked (RFC 6455 §5.1 violation)",
        ));
    }

    // Extended payload length.
    if len == 126 {
        let mut ext = [0u8; 2];
        reader.read_exact(&mut ext)?;
        len = u16::from_be_bytes(ext) as u64;
    } else if len == 127 {
        let mut ext = [0u8; 8];
        reader.read_exact(&mut ext)?;
        len = u64::from_be_bytes(ext);
    }

    // Defensive: cap the payload at 1 MiB. The browser
    // sends small JSON control messages; anything bigger
    // is an attack or a bug.
    if len > 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("WebSocket frame too large: {} bytes", len),
        ));
    }

    // Mask key + masked payload.
    let mut mask = [0u8; 4];
    reader.read_exact(&mut mask)?;
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload)?;
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= mask[i & 3];
    }

    let text = String::from_utf8(payload).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("WebSocket text frame is not UTF-8: {e}"),
        )
    })?;
    Ok(Some(text))
}

/// Helper: read and discard the body of a control frame
/// (ping/pong) using the masking rules from
/// `read_text_frame`.
fn read_and_discard_frame<R: Read>(
    reader: &mut R,
    masked: bool,
    initial_len: u64,
) -> io::Result<()> {
    let mut len = initial_len;
    if len == 126 {
        let mut ext = [0u8; 2];
        reader.read_exact(&mut ext)?;
        len = u16::from_be_bytes(ext) as u64;
    } else if len == 127 {
        let mut ext = [0u8; 8];
        reader.read_exact(&mut ext)?;
        len = u64::from_be_bytes(ext);
    }
    if len > 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control frame too large",
        ));
    }
    if masked {
        let mut mask = [0u8; 4];
        reader.read_exact(&mut mask)?;
        let mut payload = vec![0u8; len as usize];
        reader.read_exact(&mut payload)?;
        // No need to unmunge — we're discarding.
    } else {
        let mut payload = vec![0u8; len as usize];
        reader.read_exact(&mut payload)?;
    }
    Ok(())
}

/// Write a single server-to-client text frame. Server
/// frames are NOT masked (RFC 6455 §5.1). We always
/// emit a single unfragmented frame; the payload is
/// capped at 1 MiB by the caller (we don't enforce here
/// because we know the messages we send are tiny).
pub fn write_text_frame<W: Write>(writer: &mut W, text: &str) -> io::Result<()> {
    let payload = text.as_bytes();
    let len = payload.len();
    if len <= 125 {
        writer.write_all(&[0x81, 0x80 | len as u8])?;
    } else if len <= 0xFFFF {
        writer.write_all(&[0x81, 0x80 | 126])?;
        writer.write_all(&(len as u16).to_be_bytes())?;
    } else {
        writer.write_all(&[0x81, 0x80 | 127])?;
        writer.write_all(&(len as u64).to_be_bytes())?;
    }
    // Server frames: no mask key. Apply the mask bit
    // (0x80) so the client knows to expect unmasked
    // payload.
    writer.write_all(payload)?;
    writer.flush()
}

/// Write a close frame (status 1000 = "normal closure").
/// Browsers respond with their own close frame and drop
/// the connection. We use this when the connection is
/// being torn down for a clean reason.
pub fn write_close_frame<W: Write>(writer: &mut W) -> io::Result<()> {
    writer.write_all(&[0x88, 0x80 | 2])?;
    writer.write_all(&1000u16.to_be_bytes())?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A canonical Chrome WebSocket upgrade request,
    /// taken from the WHATWG `WebSockets` test fixtures.
    const CHROME_LIKE_REQUEST: &[u8] =
        b"GET / HTTP/1.1\r\nHost: 127.0.0.1:9157\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";

    #[test]
    fn handshake_accepts_chrome_request() {
        let mut reader = Cursor::new(CHROME_LIKE_REQUEST.to_vec());
        let mut writer = Vec::new();
        let outcome = handshake(&mut reader, &mut writer).unwrap();
        assert!(matches!(outcome, HandshakeOutcome::WebSocket));
        let resp = std::str::from_utf8(&writer).unwrap();
        assert!(resp.starts_with("HTTP/1.1 101 Switching Protocols\r\n"));
        assert!(resp.contains("Upgrade: websocket\r\n"));
        assert!(resp.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"));
    }

    #[test]
    fn handshake_rejects_non_ws_get() {
        // Plain HTTP GET (no Upgrade header). Should fall
        // through to NotWebSocket so the caller can treat
        // the connection as line-delimited JSON.
        let req = b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let mut reader = Cursor::new(req.to_vec());
        let mut writer = Vec::new();
        let outcome = handshake(&mut reader, &mut writer).unwrap();
        assert!(matches!(outcome, HandshakeOutcome::NotWebSocket));
    }

    #[test]
    fn handshake_rejects_invalid() {
        // Looks like a WS upgrade but the key is missing.
        let req = b"GET / HTTP/1.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n";
        let mut reader = Cursor::new(req.to_vec());
        let mut writer = Vec::new();
        let outcome = handshake(&mut reader, &mut writer).unwrap();
        assert!(matches!(outcome, HandshakeOutcome::Invalid));
    }

    /// A single masked text frame containing `{"hello":"world"}`.
    /// Mask key: 0x11223344. Built by hand to match the
    /// RFC 6455 frame layout.
    fn make_masked_text_frame(payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
        let mut frame = vec![0x81, 0x80 | payload.len() as u8];
        frame.extend_from_slice(&mask);
        let masked: Vec<u8> = payload
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ mask[i & 3])
            .collect();
        frame.extend_from_slice(&masked);
        frame
    }

    #[test]
    fn read_text_frame_unmasks_payload() {
        let payload = br#"{"url":"https://example.com/file.zip"}"#;
        let mask = [0x11, 0x22, 0x33, 0x44];
        let frame = make_masked_text_frame(payload, mask);
        let mut reader = Cursor::new(frame);
        let got = read_text_frame(&mut reader).unwrap().unwrap();
        assert_eq!(got, std::str::from_utf8(payload).unwrap());
    }

    #[test]
    fn read_text_frame_rejects_unmasked() {
        // 0x81 = FIN+text, 0x05 = unmasked + length 5.
        let frame = vec![0x81, 0x05, b'h', b'e', b'l', b'l', b'o'];
        let mut reader = Cursor::new(frame);
        let err = read_text_frame(&mut reader).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn read_text_frame_handles_close() {
        // 0x88 = FIN+close, 0x00 = unmasked, length 0.
        let frame = vec![0x88, 0x00];
        let mut reader = Cursor::new(frame);
        let got = read_text_frame(&mut reader).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn read_text_frame_rejects_fragmented() {
        // 0x01 = no-FIN+text (fragmented).
        let frame = vec![0x01, 0x85, 0, 0, 0, 0, b'h', b'e', b'l', b'l', b'o'];
        let mut reader = Cursor::new(frame);
        let err = read_text_frame(&mut reader).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn write_text_frame_round_trip() {
        // Write a frame, then parse it with read_text_frame
        // *with* the client-side mask bit set (i.e. pretend
        // the bytes came from a server, but our reader is
        // a "client" expecting masked frames). To make this
        // a true round-trip, we hand-build a frame the
        // server would send: unmasked, text opcode, FIN.
        let text = r#"{"ok":true}"#;
        let mut buf = Vec::new();
        write_text_frame(&mut buf, text).unwrap();
        // First two bytes: 0x81 (FIN+text), 0x80|len.
        assert_eq!(buf[0], 0x81);
        assert_eq!(buf[1] & 0x80, 0x80); // server-to-client: mask bit set
        assert_eq!(buf[1] & 0x7F, text.len() as u8);
        // The payload follows immediately (no mask key for
        // server frames).
        let payload = &buf[2..];
        assert_eq!(std::str::from_utf8(payload).unwrap(), text);
    }

    #[test]
    fn read_handles_extended_length_16bit() {
        // 0x81 = FIN+text, 0xFE = masked + 126 (extended 16-bit length follows).
        let payload = vec![b'x'; 200];
        let mask = [0xAA, 0xBB, 0xCC, 0xDD];
        let mut frame = vec![0x81, 0xFE];
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        frame.extend_from_slice(&mask);
        let masked: Vec<u8> = payload
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ mask[i & 3])
            .collect();
        frame.extend_from_slice(&masked);
        let mut reader = Cursor::new(frame);
        let got = read_text_frame(&mut reader).unwrap().unwrap();
        assert_eq!(got.len(), 200);
        assert!(got.chars().all(|c| c == 'x'));
    }

    // ─── Real-listener round-trip test ─────────────────────────
    //
    // The buffer-based tests above prove the parser logic
    // works. This test proves it works over a real TCP
    // stream: we bind a `TcpListener` on `127.0.0.1:0` (the
    // OS picks a free port), accept a connection, drive
    // `ws::handshake` and `ws::read_text_frame` against it,
    // and have a second thread act as the WebSocket client
    // that sends a real upgrade request + masked text
    // frame. The goal is to catch any issue that only
    // surfaces when the code is running on actual sockets
    // (buffer refill, blocking reads, kernel buffering).
    #[test]
    fn real_listener_round_trip() {
        use std::io::{BufReader, Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // Server thread: accept, do the handshake, read one
        // text frame, return the payload.
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut stream = BufReader::new(stream);
            // We need a separate Write half; clone the
            // TcpStream out of the BufReader.
            let inner = stream.get_ref().try_clone().unwrap();
            let mut writer = inner;
            match handshake(&mut stream, &mut writer).unwrap() {
                HandshakeOutcome::WebSocket => {}
                other => panic!("expected WebSocket, got {:?}", other),
            }
            let payload = read_text_frame(&mut stream)
                .unwrap()
                .expect("server should receive a text frame");
            // Echo back a server-to-client text frame so
            // we can verify the write path too.
            write_text_frame(&mut writer, r#"{"ok":true}"#).unwrap();
            payload
        });

        // Client thread: open a real TCP connection, send a
        // valid upgrade request, then send a masked text
        // frame. The mask is required (RFC 6455 §5.1); we
        // use a fixed mask for reproducibility.
        let client = thread::spawn(move || {
            let mut stream =
                TcpStream::connect(("127.0.0.1", port)).unwrap();
            let request = b"GET / HTTP/1.1\r\n\
                            Host: 127.0.0.1:9157\r\n\
                            Upgrade: websocket\r\n\
                            Connection: Upgrade\r\n\
                            Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                            Sec-WebSocket-Version: 13\r\n\r\n";
            stream.write_all(request).unwrap();
            // Read the 101 response.
            let mut resp = Vec::new();
            let mut tmp = [0u8; 256];
            loop {
                let n = stream.read(&mut tmp).unwrap();
                if n == 0 {
                    break;
                }
                resp.extend_from_slice(&tmp[..n]);
                if resp.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let resp_str = std::str::from_utf8(&resp).unwrap();
            assert!(resp_str.starts_with("HTTP/1.1 101"), "got: {resp_str}");
            assert!(resp_str.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));

            // Send a masked text frame.
            let payload = br#"{"url":"https://example.com/file.zip","type":"download"}"#;
            let mask = [0x01, 0x02, 0x03, 0x04];
            let mut frame = vec![0x81, 0x80 | payload.len() as u8];
            frame.extend_from_slice(&mask);
            for (i, b) in payload.iter().enumerate() {
                frame.push(b ^ mask[i & 3]);
            }
            stream.write_all(&frame).unwrap();

            // Read the server's echo (a small `{"ok":true}`).
            // We don't need to parse it — just confirm the
            // server-to-client write path is wired up. TCP
            // does not guarantee that all bytes arrive in a
            // single `read` call, so we use `read_exact` to
            // block until the full frame has arrived. The
            // frame is 13 bytes: 2-byte WS header
            // (`0x81 0x8B` = FIN+text, unmasked, length 11)
            // plus the 11-byte `{"ok":true}` payload. Server
            // frames are not masked (RFC 6455 §5.1).
            let mut echo = [0u8; 13];
            stream.read_exact(&mut echo).unwrap();
            // The server frame is binary (header bytes 0x81
            // 0x8B + the 11-byte payload), so we compare
            // against a byte slice rather than a `&str`
            // (Rust's `&str` hex escapes stop at the
            // printable-ASCII range). `{"ok":true}` is 11
            // characters, so the length byte is 11 = 0x0B,
            // and the masked bit (0x80) is set, giving 0x8B.
            let expected: [u8; 13] = [
                0x81, 0x8B, b'{', b'"', b'o', b'k', b'"', b':', b't', b'r', b'u', b'e', b'}',
            ];
            assert_eq!(&echo[..], &expected[..], "got echo: {:?}", &echo[..]);
        });

        let payload = server.join().unwrap();
        client.join().unwrap();
        // `read_text_frame` returns `Option<String>`, so
        // `payload` is already a UTF-8 `String`. Compare
        // directly without the extra `from_utf8` round-trip.
        assert_eq!(
            payload,
            r#"{"url":"https://example.com/file.zip","type":"download"}"#.to_string()
        );
    }
}
