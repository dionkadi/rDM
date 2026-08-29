//! Listens for URLs forwarded by the browser native-messaging host
//! (`dm-native-host`) and enqueues them as downloads.
//!
//! The native host connects to a localhost TCP socket and sends newline-
//! delimited JSON (`{"url":"..."}`). Each URL is turned into a `Queued`
//! download exactly like the `add_download` command; engine events are then
//! emitted to the webview through the manager's existing event sink.
//!
//! NOTE: this module cannot be compiled in the current sandbox (it depends on
//! Tauri/webview). It is written against the Tauri v2 API and mirrors
//! `commands::add_download`; build it on a host with `webkit2gtk`/webview.

use dm_engine::manager::DownloadManager;
use dm_engine::model::{Download, DownloadStatus};
use dm_engine::protocol;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

/// Port the native host connects to. Keep in sync with
/// `crates/native-host/src/main.rs::DEFAULT_PORT`.
pub const DEFAULT_PORT: u16 = 9157;

/// Bind the listener on a background OS thread. Returns immediately; if the
/// port is unavailable the thread logs and exits without affecting the app.
pub fn start_native_host_listener(manager: Arc<DownloadManager>, port: u16) {
    thread::spawn(move || {
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("dm: native-host listener bind failed on {port}: {e}");
                return;
            }
        };
        eprintln!("dm: native-host listener on 127.0.0.1:{port}");
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    let mgr = manager.clone();
                    thread::spawn(move || handle_conn(s, mgr));
                }
                Err(_) => continue,
            }
        }
    });
}

fn handle_conn(stream: std::net::TcpStream, manager: Arc<DownloadManager>) {
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(url) = v.get("url").and_then(|u| u.as_str()) {
                enqueue(manager.clone(), url);
            }
        }
    }
}

fn enqueue(manager: Arc<DownloadManager>, url: &str) {
    let dir = manager.save_dir_for(None);
    let fname = protocol::suggest_filename(url, None, "download.bin");
    let save_path = dir.join(&fname);
    let mut d = Download::new(url.to_string());
    d.filename = fname;
    d.save_path = save_path;
    d.status = DownloadStatus::Queued;
    manager.add(d);
}
