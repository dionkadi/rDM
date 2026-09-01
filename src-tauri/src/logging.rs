//! File-based logging setup.
//!
//! Every DM session writes to a fresh, timestamped log file under
//! `~/.local/share/DM/YYYYMMDD-HHMMSS.log` (with platform-appropriate
//! fallbacks when `XDG_DATA_HOME` is overridden, e.g. on macOS / Windows).
//!
//! Implemented on top of the `log` crate (which the engine and all its
//! transitive deps already use) via a tiny custom `log::Log` shim — no
//! new dependencies. Each log line is also mirrored to stderr so
//! `cargo tauri dev` and journalctl keep working.

use log::{Level, LevelFilter, Metadata, Record};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

/// Holds the open log file + the path it lives at. Returned from
/// [`init`] so callers can read the path (e.g. to show it in a
/// "Report a bug" dialog). Dropping it does nothing — the actual
/// `log::Log` instance is owned by the leaked shim.
pub struct LogGuard {
    file: Mutex<std::fs::File>,
    path: PathBuf,
}

impl LogGuard {
    /// Absolute path of the log file the guard is writing to.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// The actual `log::Log` implementation. Leaked into a `Box` so we
/// have a `&'static dyn log::Log` to pass to `log::set_logger`. The
/// `LogGuard` is held inside the shim (so dropping the original
/// `LogGuard` after `init()` returns doesn't break logging).
struct LoggingShim {
    inner: LogGuard,
}

impl log::Log for LoggingShim {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format_line(record);
        if let Ok(mut guard) = self.inner.file.lock() {
            if let Err(e) = guard.write_all(line.as_bytes()) {
                eprintln!("dm: log write failed: {e}");
            }
            // Always flush — the engine emits a small number of log
            // lines per download (one per status change, one per
            // error) and the cost of an extra fsync is negligible.
            let _ = guard.flush();
        }
        // Mirror to stderr so `cargo tauri dev` and journalctl still
        // work.
        eprint!("{line}");
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.inner.file.lock() {
            let _ = guard.flush();
        }
    }
}

/// Initialise the global `log` shim that writes to
/// `~/.local/share/DM/YYYYMMDD-HHMMSS.log`. The level is read from
/// the `DM_LOG` env var (a plain log level like `info`/`debug`); if
/// unset, defaults to `info`.
pub fn init() -> LogGuard {
    let dir = log_dir();
    let (path, file) = match std::fs::create_dir_all(&dir)
        .and_then(|_| timestamped_log_file(&dir))
    {
        Ok((p, f)) => {
            eprintln!("dm: logging to {}", p.display());
            (p, f)
        }
        Err(e) => {
            eprintln!(
                "dm: warning — could not create log file in {}: {e}; \
                 falling back to /dev/stderr",
                dir.display()
            );
            let stderr = std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/stderr")
                .or_else(|_| std::fs::File::create("/tmp/dm-stderr.log"))
                .expect("at least one of /dev/stderr or /tmp must be writable");
            (dir.join("stderr-fallback.log"), stderr)
        }
    };

    let guard = LogGuard {
        file: Mutex::new(file),
        path: path.clone(),
    };

    // The `log` crate requires a `&'static dyn Log`. The standard
    // pattern is to leak the shim into a `Box` (one-time allocation
    // for the lifetime of the process).
    let leaked: &'static dyn log::Log = Box::leak(Box::new(LoggingShim {
        inner: LogGuard {
            file: Mutex::new(
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .or_else(|_| {
                        std::fs::OpenOptions::new()
                            .write(true)
                            .open("/dev/stderr")
                    })
                    .expect("either the log file or /dev/stderr must be writable"),
            ),
            path: path.clone(),
        },
    }));
    log::set_logger(leaked).expect("set_logger failed");
    log::set_max_level(LevelFilter::Info);

    if let Ok(spec) = std::env::var("DM_LOG").or_else(|_| std::env::var("RUST_LOG")) {
        if let Ok(filter) = spec.parse::<LevelFilter>() {
            log::set_max_level(filter);
        }
    }

    guard
}

/// Format a `log` record as a single grep-friendly line.
fn format_line(record: &Record<'_>) -> String {
    let now = chrono::Local::now();
    let ts = now.format("%Y-%m-%d %H:%M:%S%.3f");
    let level = level_tag(record.level());
    let target = record.target();
    let msg = record.args().to_string();
    format!("{ts} {level} {target}: {msg}\n")
}

fn level_tag(level: Level) -> &'static str {
    match level {
        Level::Error => "ERROR",
        Level::Warn => "WARN ",
        Level::Info => "INFO ",
        Level::Debug => "DEBUG",
        Level::Trace => "TRACE",
    }
}

/// Resolve the per-user log directory.
///
/// Order of preference:
/// 1. `$DM_LOG_DIR` (manual override)
/// 2. `$XDG_DATA_HOME/dm/logs` (Linux)
/// 3. `~/.local/share/dm/logs` (Linux default)
/// 4. `~/Library/Application Support/dm/logs` (macOS)
/// 5. `%APPDATA%\dm\logs` (Windows)
fn log_dir() -> PathBuf {
    if let Ok(p) = std::env::var("DM_LOG_DIR") {
        return PathBuf::from(p);
    }
    if cfg!(target_os = "macos") {
        if let Some(home) = home_dir() {
            return home
                .join("Library")
                .join("Application Support")
                .join("dm")
                .join("logs");
        }
    }
    if cfg!(target_os = "windows") {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("dm").join("logs");
        }
        if let Some(home) = home_dir() {
            return home.join("AppData").join("Roaming").join("dm").join("logs");
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("dm").join("logs");
    }
    if let Some(home) = home_dir() {
        return home.join(".local").join("share").join("dm").join("logs");
    }
    std::env::temp_dir().join("dm").join("logs")
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

/// Create `YYYYMMDD-HHMMSS.log` inside `dir`. The timestamp is captured
/// at startup so a session always has exactly one file (no rotation).
fn timestamped_log_file(dir: &std::path::Path) -> std::io::Result<(PathBuf, std::fs::File)> {
    let now = chrono::Local::now();
    let path = dir.join(format!("{}.log", now.format("%Y%m%d-%H%M%S")));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    Ok((path, file))
}

// Reference to keep the unused import warning away — `Duration` is
// re-exported for callers that want to set their own flush cadence.
#[allow(dead_code)]
const _FLUSH_INTERVAL_TYPE: Option<Duration> = None;
