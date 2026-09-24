//! Runs the **real** `src-tauri/src/ws.rs` listener as a terminal
//! program, plus a mirror of the accept/dispatch logic in
//! `native_host.rs` (peek the first byte: `G` → WebSocket, anything else
//! → line-delimited JSON).
//!
//! Why: the browser reports `close code 1006` for every possible way a
//! WebSocket can fail, so "the app is running but the extension won't
//! connect" is undecidable from the browser side. This lets you test the
//! server half on its own, without building or launching the Tauri app.
//!
//!   cargo run            # 127.0.0.1:9158
//!   cargo run -- 9159    # another port
//!
//! Then: `node ../../browser-extension/scripts/check-host.mjs 9158`
//!
//! The dispatch below is a **copy** of `native_host.rs` — that module
//! depends on `tauri::AppHandle` and can't be imported here. Keep them in
//! step; see README.md.

// `#[path]` on a crate root resolves relative to the crate root's
// directory (`probe-listener/src/`), not to this file — hence the two
// hops up to `src-tauri/`.
#[path = "../../src/ws.rs"]
mod ws;

use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::thread;

/// Port 9158 by default: never collides with a running DM app (9157), so
/// a stray `cargo run` can't be mistaken for the real listener.
const DEFAULT_PORT: u16 = 9158;

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("probe-listener: bind failed on 127.0.0.1:{port}: {e}");
            std::process::exit(1);
        }
    };
    println!("dm: native-host listener on 127.0.0.1:{port}");
    println!("probe it with: node browser-extension/scripts/check-host.mjs {port}");

    for conn in listener.incoming().flatten() {
        thread::spawn(move || handle(conn));
    }
}

fn handle(stream: TcpStream) {
    let mut reader = BufReader::new(stream);

    // Same peek as native_host.rs: only the first byte, without
    // consuming it (`fill_buf` advances the cursor, not the socket).
    let is_ws = match reader.fill_buf() {
        Ok(buf) => buf.first().copied() == Some(b'G') || buf.first().copied() == Some(b'g'),
        Err(_) => false,
    };

    if !is_ws {
        // Legacy path: the dm-native-host binary speaks line-delimited
        // JSON. Log it so a mis-dispatched connection is visible.
        println!("legacy: line-JSON path");
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            println!("legacy: {}", line.trim_end());
            line.clear();
        }
        return;
    }

    let mut writer = match reader.get_ref().try_clone() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("ws: failed to clone stream: {e}");
            return;
        }
    };

    match ws::handshake(&mut reader, &mut writer) {
        Ok(ws::HandshakeOutcome::WebSocket) => {}
        other => {
            println!("ws: handshake not completed: {other:?}");
            return;
        }
    }
    println!("ws: client connected");

    loop {
        match ws::read_text_frame(&mut reader) {
            Ok(Some(payload)) => {
                println!("ws: frame {payload}");
                // The real server acks so the extension knows the payload
                // was accepted; mirror that.
                let _ = ws::write_text_frame(&mut writer, r#"{"ok":true}"#);
            }
            Ok(None) => {
                let _ = ws::write_close_frame(&mut writer);
                println!("ws: clean close");
                break;
            }
            Err(e) => {
                println!("ws: disconnected: {e}");
                break;
            }
        }
    }
}
