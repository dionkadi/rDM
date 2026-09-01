//! Listens for URLs forwarded by the browser native-messaging host
//! (`dm-native-host`) and enqueues them as downloads.
//!
//! The native host connects to a localhost TCP socket and sends newline-
//! delimited JSON (`{"url":"..."}`). Each URL is turned into a `Queued`
//! download exactly like the `add_download` command; engine events are then
//! emitted to the webview through the manager's existing event sink.
//!
//! The listener can be probed via `probe_native_host_port` to drive the
//! status panel in the Settings → Extensions tab.
//!
//! NOTE: this module cannot be compiled in the current sandbox (it depends on
//! Tauri/webview). It is written against the Tauri v2 API and mirrors
//! `commands::add_download`; build it on a host with `webkit2gtk`/webview.

use dm_engine::manager::DownloadManager;
use dm_engine::model::{Download, DownloadStatus};
use dm_engine::protocol;
use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Port the native host connects to. Keep in sync with
/// `crates/native-host/src/main.rs::DEFAULT_PORT`.
pub const DEFAULT_PORT: u16 = 9157;

/// Shared state the frontend can poll via `probe_native_host` to see whether
/// the listener is up and when it last saw traffic.
#[derive(Clone)]
pub struct NativeHostStatus {
    pub bound: Arc<AtomicBool>,
    pub last_event_unix: Arc<AtomicU64>,
}

impl Default for NativeHostStatus {
    fn default() -> Self {
        NativeHostStatus {
            bound: Arc::new(AtomicBool::new(false)),
            last_event_unix: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl NativeHostStatus {
    pub fn snapshot(&self) -> (bool, u64) {
        (self.bound.load(Ordering::Relaxed), self.last_event_unix.load(Ordering::Relaxed))
    }
    fn touch(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.last_event_unix.store(now, Ordering::Relaxed);
    }
}

/// Bind the listener on a background OS thread. Returns immediately; if the
/// port is unavailable the thread logs and exits without affecting the app.
pub fn start_native_host_listener(
    manager: Arc<DownloadManager>,
    port: u16,
    status: NativeHostStatus,
) {
    thread::spawn(move || {
        let listener = match TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("dm: native-host listener bind failed on {port}: {e}");
                return;
            }
        };
        // Make accept() non-blocking so we can update `bound` promptly and
        // exit cleanly if the user later closes the app.
        listener.set_nonblocking(true).ok();
        status.bound.store(true, Ordering::Relaxed);
        eprintln!("dm: native-host listener on 127.0.0.1:{port}");
        loop {
            match listener.accept() {
                Ok((s, _)) => {
                    let mgr = manager.clone();
                    let st = status.clone();
                    thread::spawn(move || handle_conn(s, mgr, st));
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Idle; sleep briefly then re-check.
                    thread::sleep(Duration::from_millis(250));
                }
                Err(_) => {
                    // Transient error: brief backoff, then keep accepting.
                    thread::sleep(Duration::from_millis(500));
                }
            }
        }
    });
}

fn handle_conn(stream: TcpStream, manager: Arc<DownloadManager>, status: NativeHostStatus) {
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
                status.touch();
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
