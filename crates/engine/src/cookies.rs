//! Browser cookie import for site-login downloads.
//!
//! Some servers gate their download endpoints behind a login session
//! (a CDN-protected academic paper, a private file share, a paywalled
//! content host, …). A user who already has a valid session in their
//! browser does not want to retype the credentials into DM's
//! per-download auth dialog — they want DM to *re-use* the cookie
//! the browser has for that site.
//!
//! This module reads cookies directly from the browser's local
//! SQLite databases, filters them by host, and formats the result as
//! a `Cookie:` header value the engine can hand to reqwest. Two
//! backends are supported:
//!
//! * **Firefox** — `cookies.sqlite` is a plain unencrypted SQLite
//!   file on every platform (Linux, macOS, Windows). The `moz_cookies`
//!   table holds the per-cookie state. We open it read-only so a
//!   lock contention with the running browser does not freeze
//!   either side.
//!
//! * **Chromium-based browsers** (Chrome, Edge, Brave, Arc, …) —
//!   the `Cookies` SQLite file is **plaintext on Linux only**. On
//!   macOS and Windows the same columns are encrypted with a per-
//!   user key from the OS keychain (macOS Keychain) or DPAPI
//!   (Windows). Decrypting those requires reaching into a system
//!   service that is not part of the engine's Tauri-free contract
//!   and varies by Chromium version. For v1 we support the
//!   **Linux** path and surface a clear "encrypted; not supported
//!   on this OS" error elsewhere.
//!
//! The output is a list of `BrowserCookie { name, value, host,
//! path, secure, expires }` plus a `format_cookie_header(&cookies)`
//! helper that builds the actual `Cookie: a=1; b=2; c=3` string the
//! engine will send. The frontend lets the user pick a domain
//! (e.g. `example.com` vs `www.example.com`) and a `secure`-only
//! toggle before pressing "Apply", which goes through the
//! existing `set_download_auth(headers, ...)` Tauri command.

use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

/// One cookie as read from a browser database. The fields map
/// 1:1 to the standard cookie attributes (RFC 6265).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserCookie {
    pub name: String,
    pub value: String,
    /// Host the cookie is bound to, e.g. `example.com`,
    /// `.example.com` (domain cookies), or `www.example.com`.
    pub host: String,
    /// Path the cookie is bound to. Defaults to `/` if missing.
    pub path: String,
    /// `true` if the cookie was marked Secure and should only be
    /// sent over HTTPS. The frontend lets the user filter this on
    /// and off — sometimes the user wants to send a Secure cookie
    /// to a localhost debug server, sometimes they don't.
    pub secure: bool,
    /// Expiry as a Unix timestamp. `None` = session cookie.
    pub expires_unix: Option<i64>,
}

/// Errors that can occur while reading cookies. Every variant
/// carries a user-actionable message — the frontend shows the
/// error verbatim in the import dialog.
#[derive(Debug, thiserror::Error)]
pub enum CookieError {
    #[error("the browser's cookie database was not found at {path}. Is the browser installed?")]
    NotFound { path: PathBuf },
    #[error("could not open the browser's cookie database: {0}")]
    Open(#[from] rusqlite::Error),
    #[error(
        "Chromium cookie databases on macOS and Windows are encrypted with a system-managed key \
         (Keychain / DPAPI). Reading them from a separate process is not supported in v1. \
         On Linux, Chromium stores cookies in plaintext and the import will work. \
         Alternatively, paste the Cookie header value directly into the per-download headers."
    )]
    EncryptedChromium,
    #[error("no cookies matched domain filter {host:?}")]
    NoMatches { host: String },
}

/// A browser whose cookie database we can read. The list is
/// intentionally small and explicit — supporting "every browser"
/// means dealing with N slightly-different cookie file layouts, and
/// the user can pick the one they actually use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserKind {
    /// Mozilla Firefox.
    Firefox,
    /// Any Chromium-based browser that uses the standard `Cookies`
    /// SQLite path (Chrome, Edge, Brave, Arc, …). On macOS / Windows
    /// this will return `CookieError::EncryptedChromium`.
    Chromium,
}

impl BrowserKind {
    /// `display` name for the UI (e.g. "Firefox", "Chromium").
    pub fn display(&self) -> &'static str {
        match self {
            BrowserKind::Firefox => "Firefox",
            BrowserKind::Chromium => "Chromium",
        }
    }
}

/// Return the on-disk path of the cookie database for the given
/// browser on the current platform. `None` means "this browser is
/// not installed (or the platform does not support it)" — the
/// frontend shows a clear "not found" message and offers a
/// "Choose file…" fallback that calls `read_browser_cookies_at`
/// directly.
pub fn default_cookie_path(kind: BrowserKind) -> Option<PathBuf> {
    match kind {
        BrowserKind::Firefox => firefox_default_path(),
        BrowserKind::Chromium => chromium_default_path(),
    }
}

/// Read every cookie for the given browser, optionally filtered by
/// `host` (case-insensitive suffix match — `example.com` matches
/// both `example.com` and `.example.com`). `host = None` returns
/// every cookie the browser has. The result is sorted by (host,
/// name) for stable display.
///
/// `path_override` lets the user pick a custom file via the OS
/// file dialog — useful when the browser is installed in a
/// non-standard location (snap, flatpak, portable .AppImage, …).
pub fn read_browser_cookies(
    kind: BrowserKind,
    host: Option<&str>,
    path_override: Option<&Path>,
) -> Result<Vec<BrowserCookie>, CookieError> {
    let path = match path_override {
        Some(p) => p.to_path_buf(),
        None => default_cookie_path(kind).ok_or_else(|| CookieError::NotFound {
            path: PathBuf::from("<no default path on this platform>"),
        })?,
    };
    if !path.exists() {
        return Err(CookieError::NotFound { path });
    }
    let cookies = match kind {
        BrowserKind::Firefox => read_firefox(&path)?,
        BrowserKind::Chromium => read_chromium(&path)?,
    };
    Ok(filter_and_sort(cookies, host))
}

// ── Firefox ─────────────────────────────────────────────────────

/// Standard Firefox profile directory for the current OS. Returns
/// the **parent** of `cookies.sqlite` (we open the file
/// specifically — the profile directory holds many other SQLite
/// files we don't want to accidentally open).
///
/// * Linux:   `~/.mozilla/firefox/<profile>/`
/// * macOS:   `~/Library/Application Support/Firefox/Profiles/<profile>/`
/// * Windows: `%APPDATA%\Mozilla\Firefox\Profiles\<profile>\`
fn firefox_default_path() -> Option<PathBuf> {
    let mut base: PathBuf = if cfg!(target_os = "macos") {
        directories::UserDirs::new()?
            .home_dir()
            .join("Library")
            .join("Application Support")
            .join("Firefox")
            .join("Profiles")
    } else if cfg!(target_os = "windows") {
        directories::UserDirs::new()?
            .home_dir()
            .join("AppData")
            .join("Roaming")
            .join("Mozilla")
            .join("Firefox")
            .join("Profiles")
    } else {
        // Linux + BSD. Firefox's Snap install uses
        // `~/snap/firefox/common/.mozilla/firefox/<profile>/`; we
        // try that as a fallback after the standard path.
        directories::UserDirs::new()?
            .home_dir()
            .join(".mozilla")
            .join("firefox")
    };
    if !base.exists() {
        // Snap fallback.
        if cfg!(target_os = "linux") {
            if let Some(home) = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf()) {
                let snap = home.join("snap").join("firefox").join("common").join(".mozilla").join("firefox");
                if snap.exists() {
                    base = snap;
                }
            }
        }
        if !base.exists() {
            return None;
        }
    }
    // Pick the first profile directory. Real-world Firefox
    // installs usually have exactly one default profile
    // (`xxxxxxx.default-release`); multi-profile installs are
    // rare. The user can fall back to the "Choose file…" picker
    // for the rare case.
    let entries = std::fs::read_dir(&base).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            let cookies = p.join("cookies.sqlite");
            if cookies.exists() {
                return Some(cookies);
            }
        }
    }
    None
}

fn read_firefox(path: &Path) -> Result<Vec<BrowserCookie>, CookieError> {
    // `SQLITE_OPEN_READ_ONLY` is critical here: the running
    // Firefox has the cookie database locked for writing, and
    // opening it read-write would either fail with SQLITE_BUSY
    // or — worse — accidentally clobber Firefox's WAL file
    // when we close. Read-only is the safe mode for an
    // out-of-process reader.
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    // Firefox's moz_cookies schema (simplified):
    //   id INTEGER PRIMARY KEY
    //   name TEXT
    //   value TEXT
    //   host TEXT
    //   path TEXT
    //   expiry INTEGER       (Unix seconds)
    //   isSecure INTEGER     (0/1)
    //   isHttpOnly INTEGER   (0/1, we don't surface this)
    //   sameSite INTEGER     (we don't surface this)
    //   rawSameSite INTEGER  (Firefox 87+; same value as sameSite
    //                         for non-default profiles)
    //
    // The schema has been stable since Firefox 3.0. We don't
    // need a schema-version check here — `PRAGMA table_info`
    // would tell us, but every released Firefox has these
    // columns and a missing column would surface as a clear
    // "no such column" rusqlite error, which the `?` operator
    // propagates with the column name in the message.
    let mut stmt = conn.prepare(
        "SELECT name, value, host, path, expiry, isSecure \
         FROM moz_cookies",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(BrowserCookie {
            name: row.get(0)?,
            value: row.get(1)?,
            host: row.get(2)?,
            path: row.get(3)?,
            secure: row.get::<_, i64>(5)? != 0,
            expires_unix: {
                let e: i64 = row.get(4)?;
                if e <= 0 { None } else { Some(e) }
            },
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

// ── Chromium ────────────────────────────────────────────────────

/// Standard Chromium `Cookies` file path for the current OS.
/// Note: this is the **plaintext** path on Linux. On macOS and
/// Windows the same filename holds an encrypted blob — `read_chromium`
/// returns `CookieError::EncryptedChromium` in that case.
fn chromium_default_path() -> Option<PathBuf> {
    if cfg!(target_os = "linux") {
        // Chromium, Chrome, Edge, Brave, Arc all use the same
        // `~/.config/<browser>/Default/Cookies` layout. We pick
        // Chrome's path as the default (most-installed) and
        // expose the file picker for the other flavours.
        directories::UserDirs::new().map(|u| {
            u.home_dir()
                .join(".config")
                .join("google-chrome")
                .join("Default")
                .join("Cookies")
        })
    } else {
        // macOS / Windows: file exists but is encrypted. We
        // still return the path so the user can see "the file is
        // there, but we can't read it on this OS". A future
        // version can plumb a decryption path through the Tauri
        // shell.
        if cfg!(target_os = "macos") {
            directories::UserDirs::new().map(|u| {
                u.home_dir()
                    .join("Library")
                    .join("Application Support")
                    .join("Google")
                    .join("Chrome")
                    .join("Default")
                    .join("Cookies")
            })
        } else if cfg!(target_os = "windows") {
            directories::UserDirs::new().map(|u| {
                u.home_dir()
                    .join("AppData")
                    .join("Local")
                    .join("Google")
                    .join("Chrome")
                    .join("User Data")
                    .join("Default")
                    .join("Cookies")
            })
        } else {
            None
        }
    }
}

fn read_chromium(path: &Path) -> Result<Vec<BrowserCookie>, CookieError> {
    if !cfg!(target_os = "linux") {
        // The file is there, but its `encrypted_value` column
        // holds a blob encrypted with a key that lives in the
        // OS keychain (macOS) or DPAPI (Windows). Without a
        // platform-specific decryption path we cannot recover
        // the value. We surface a clear error rather than
        // silently returning the encrypted blob.
        return Err(CookieError::EncryptedChromium);
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    // Chromium's `cookies` table schema (Linux, unencrypted v):
    //   host_key TEXT
    //   name TEXT
    //   value TEXT
    //   encrypted_value BLOB    (we ignore; plaintext `value` is the
    //                            real one on Linux)
    //   path TEXT
    //   expires_utc INTEGER     (microseconds since the Unix epoch,
    //                            not seconds — we divide by 1e6
    //                            below to get a normal timestamp)
    //   is_secure INTEGER
    //   is_httponly INTEGER
    //   samesite INTEGER
    //   priority INTEGER
    //   source_scheme INTEGER
    // The schema has been stable since Chrome 25.
    let mut stmt = conn.prepare(
        "SELECT host_key, name, value, path, expires_utc, is_secure \
         FROM cookies",
    )?;
    let rows = stmt.query_map([], |row| {
        let expires_utc: i64 = row.get(4)?;
        // Chromium stores expiry in microseconds; convert to
        // whole seconds. 0 means "session cookie" in Chromium
        // (matches Firefox's `expiry <= 0` convention above).
        let expires_unix = if expires_utc <= 0 {
            None
        } else {
            Some(expires_utc / 1_000_000)
        };
        Ok(BrowserCookie {
            name: row.get(1)?,
            value: row.get(2)?,
            host: row.get(0)?,
            path: row.get(3)?,
            secure: row.get::<_, i64>(5)? != 0,
            expires_unix,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

// ── Filtering / formatting ──────────────────────────────────────

/// Filter `cookies` to those whose host matches `host` (suffix
/// match, case-insensitive). `host = None` returns all cookies
/// (after a stable sort). The result is sorted by (host, name)
/// so the UI shows a stable list across re-reads.
fn filter_and_sort(
    mut cookies: Vec<BrowserCookie>,
    host: Option<&str>,
) -> Vec<BrowserCookie> {
    if let Some(h) = host {
        let h = h.to_ascii_lowercase();
        cookies.retain(|c| {
            let ch = c.host.to_ascii_lowercase();
            // Domain cookies are stored with a leading dot
            // (".example.com") in both Firefox and Chromium —
            // strip it before the suffix match so the user's
            // "example.com" filter catches them.
            let ch = ch.strip_prefix('.').unwrap_or(&ch);
            ch == h || ch.ends_with(&format!(".{h}"))
        });
    }
    cookies.sort_by(|a, b| a.host.cmp(&b.host).then(a.name.cmp(&b.name)));
    cookies
}

/// Format a list of cookies as the value of a `Cookie:` request
/// header. Each cookie becomes `name=value`; pairs are joined
/// with `; ` (the standard separator). Cookies whose name or
/// value contains whitespace, `;`, or `,` are quoted — RFC 6265
/// §5.2 says the value can be either a token or a quoted-string,
/// and using the quoted form is always safe.
pub fn format_cookie_header(cookies: &[BrowserCookie]) -> String {
    let mut parts = Vec::with_capacity(cookies.len());
    for c in cookies {
        let needs_quote = |s: &str| {
            s.chars()
                .any(|ch| ch.is_whitespace() || ch == ';' || ch == ',')
        };
        if needs_quote(&c.name) || needs_quote(&c.value) {
            // We don't try to escape embedded quotes — the
            // cookies we're reading came from a browser which
            // (per RFC 6265) should never have produced them.
            // A defensive `replace('"', "")` keeps the header
            // parsable if a malformed entry sneaks in.
            let n = c.name.replace('"', "");
            let v = c.value.replace('"', "");
            parts.push(format!("{n}=\"{v}\""));
        } else {
            parts.push(format!("{}={}", c.name, c.value));
        }
    }
    parts.join("; ")
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Build a fresh, on-disk Firefox-compatible `moz_cookies`
    /// table populated with the given rows. Returns the path
    /// to the file. The file is leaked (so it survives until
    /// the test process exits) — tests don't care about cleanup
    /// beyond the per-test result. Using a real on-disk file
    /// (not an in-memory DB materialised with `VACUUM INTO`,
    /// which breaks on tmpfs with a per-process I/O quota)
    /// keeps the test independent of the surrounding filesystem.
    fn make_firefox_db_on_disk(
        rows: &[(&str, &str, &str, &str, i64, i64)],
    ) -> std::path::PathBuf {
        // Build the file in a real-on-disk path (`/var/tmp` is
        // on the root filesystem on most Linux distros; the
        // per-test cwd may be on tmpfs with a quota). The
        // filename includes the test's PID + a per-test counter
        // so parallel tests don't collide.
        let path = std::path::PathBuf::from(format!(
            "/var/tmp/dm_cookies_test_{}_{}.sqlite",
            std::process::id(),
            TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        ));
        // If a previous run left a file behind, drop it before
        // we recreate.
        let _ = std::fs::remove_file(&path);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE moz_cookies (
                 id INTEGER PRIMARY KEY,
                 name TEXT NOT NULL,
                 value TEXT,
                 host TEXT,
                 path TEXT,
                 expiry INTEGER,
                 isSecure INTEGER
             )",
        )
        .unwrap();
        for (i, (name, value, host, path_, expiry, is_secure)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO moz_cookies (id, name, value, host, path, expiry, isSecure)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![i as i64, name, value, host, path_, expiry, is_secure],
            )
            .unwrap();
        }
        // Drop the connection so the WAL/journal is checkpointed
        // and the file is in a clean read-only state for the
        // engine's `SQLITE_OPEN_READ_ONLY` open.
        drop(conn);
        path
    }

    /// Monotonic counter for the on-disk test files. Tests
    /// running in parallel might try to create the same
    /// filename; this counter guarantees uniqueness.
    static TEST_COUNTER: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0);

    #[test]
    fn filter_by_host_matches_subdomains() {
        let path = make_firefox_db_on_disk(&[
            ("sid", "AAA", "example.com", "/", 9_999_999_999, 0),
            ("pref", "blue", ".example.com", "/", 0, 0),
            ("session", "xyz", "www.example.com", "/", 0, 1),
            ("other", "1", "other.com", "/", 0, 0),
        ]);
        let filtered = filter_and_sort(
            read_firefox(&path).unwrap(),
            Some("example.com"),
        );
        // 3 matches: example.com, .example.com (domain cookie),
        // www.example.com. other.com filtered out.
        assert_eq!(filtered.len(), 3);
        let names: Vec<&str> = filtered.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"sid"));
        assert!(names.contains(&"pref"));
        assert!(names.contains(&"session"));
        // `other` is the only one missing.
        assert!(!names.contains(&"other"));
    }

    #[test]
    fn filter_by_host_is_case_insensitive() {
        let path = make_firefox_db_on_disk(&[("a", "1", "Example.COM", "/", 0, 0)]);
        let filtered = filter_and_sort(
            read_firefox(&path).unwrap(),
            Some("EXAMPLE.com"),
        );
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn format_cookie_header_basic() {
        let cookies = vec![
            BrowserCookie {
                name: "a".into(),
                value: "1".into(),
                host: "example.com".into(),
                path: "/".into(),
                secure: false,
                expires_unix: None,
            },
            BrowserCookie {
                name: "b".into(),
                value: "2".into(),
                host: "example.com".into(),
                path: "/".into(),
                secure: false,
                expires_unix: None,
            },
        ];
        assert_eq!(format_cookie_header(&cookies), "a=1; b=2");
    }

    #[test]
    fn format_cookie_header_quotes_whitespace() {
        // A cookie value with a space must be quoted per
        // RFC 6265 §5.2. The output is still valid as the
        // value of a `Cookie:` header.
        let cookies = vec![BrowserCookie {
            name: "msg".into(),
            value: "hello world".into(),
            host: "example.com".into(),
            path: "/".into(),
            secure: false,
            expires_unix: None,
        }];
        let header = format_cookie_header(&cookies);
        assert_eq!(header, "msg=\"hello world\"");
    }

    #[test]
    fn read_firefox_preserves_secure_flag() {
        let path = make_firefox_db_on_disk(&[("a", "1", "example.com", "/", 0, 1)]);
        let cookies = read_firefox(&path).unwrap();
        assert_eq!(cookies.len(), 1);
        assert!(cookies[0].secure);
    }

    #[test]
    fn read_firefox_zero_expiry_means_session() {
        // Firefox uses expiry = 0 for session cookies (i.e.
        // "delete when the browser closes"). We surface that
        // as `expires_unix = None` so the frontend can hide
        // the expiry field for session cookies.
        let path = make_firefox_db_on_disk(&[("a", "1", "example.com", "/", 0, 0)]);
        let cookies = read_firefox(&path).unwrap();
        assert!(cookies[0].expires_unix.is_none());

        // A real expiry timestamp is preserved.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 3600;
        let path = make_firefox_db_on_disk(&[("a", "1", "example.com", "/", now, 0)]);
        let cookies = read_firefox(&path).unwrap();
        assert_eq!(cookies[0].expires_unix, Some(now));
    }

    /// The open-ended-filter path: asking for a host that has no
    /// cookies should return an empty list (not an error). The
    /// user-facing error is reserved for the "you typed a host,
    /// we couldn't find anything" flow which is signalled by the
    /// `CookieError::NoMatches` variant — but in the lower-level
    /// `read_browser_cookies` we keep the empty list so the
    /// frontend can render "0 cookies found for X" without
    /// surfacing an error toast.
    #[test]
    fn no_matches_returns_empty_list() {
        let path = make_firefox_db_on_disk(&[("a", "1", "example.com", "/", 0, 0)]);
        let cookies = read_browser_cookies(
            BrowserKind::Firefox,
            Some("nope.invalid"),
            Some(&path),
        )
        .unwrap();
        assert!(cookies.is_empty());
    }

    #[test]
    fn missing_file_returns_not_found_error() {
        let res = read_browser_cookies(
            BrowserKind::Firefox,
            None,
            Some(std::path::Path::new("/nonexistent/cookies.sqlite")),
        );
        match res {
            Err(CookieError::NotFound { .. }) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }
}
