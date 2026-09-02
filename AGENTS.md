# AGENTS.md — DM Download Manager

A guide for AI coding agents (and humans) working in this repo. Read this before touching code.

## What DM is

**DM** is a cross-platform, IDM-style download manager built with **Tauri 2 (Rust + WebView)**. The Rust half is `src-tauri` plus a UI-agnostic core in `crates/engine`; the UI is a **Svelte 4 + TypeScript + Vite** SPA. A Chromium MV3 browser extension (`browser-extension/`) feeds URLs into the app via a small native-messaging shim (`crates/native-host`).

## Repo layout

```
DM/
├── Cargo.toml                 # Cargo workspace (resolver v2). Members:
│                              #   crates/engine, crates/native-host, src-tauri
├── package.json               # `dm-frontend` (Svelte+Vite). Scripts: dev, build, check, tauri
├── vite.config.ts             # Fixed port 5173, strictPort, host:false (Tauri quirk)
├── tsconfig.json / tsconfig.node.json
├── svelte.config.js           # vitePreprocess only
├── index.html                 # Loads /src/main.ts → mounts <App/> into #app
├── src/                       # Frontend (Svelte)
│   ├── main.ts, App.svelte, app.css
│   └── lib/
│       ├── api.ts             # Thin invoke(...) wrappers (one per Tauri command)
│       ├── types.ts           # TS mirrors of Rust model (camelCase JSON)
│       ├── design-tokens.ts   # Single source of design tokens (CSS-var compatible)
│       ├── components/        # DownloadRow, FileIcon, ProgressBar, StatusBadge,
│       │                      # DropdownMenu, InlineSpeedLimit, GlobalSearch,
│       │                      # CommandPalette, SmartFilters, SettingsTabs,
│       │                      # StatusBar, Toast (barrel: index.ts)
│       ├── stores/            # downloads.ts, ui.ts, settings.ts — Svelte writables/derived
│       ├── hooks/             # useKeyboardShortcuts.ts (global ⌘K + per-view shortcuts)
│       └── utils/             # fuzzySearch.ts, formatters.ts, env.ts (browser/Mac detection)
├── src-tauri/                 # Tauri shell (Rust)
│   ├── Cargo.toml             # depends on dm-engine (path = ../crates/engine)
│   ├── tauri.conf.json        # identifier "com.dm.downloadmanager", frontendDist=../dist
│   ├── build.rs               # tauri_build::build()
│   ├── capabilities/default.json
│   ├── icons/                 # All platform icon sizes (32..512, .ico/.icns)
│   └── src/
│       ├── main.rs            # logging::init() → Builder + plugins + setup (storage,
│       │                      #   manager, listener, tray) — injects the Tokio spawner
│       ├── commands.rs        # #[tauri::command] surface → delegates to DownloadManager
│       ├── events.rs          # Engine DownloadEvent → FrontendEvent (one channel: "download-event")
│       ├── logging.rs         # `log` crate shim → per-session file under XDG_DATA_HOME,
│       │                      #   mirrored to stderr; returns a LogGuard with .path()
│       ├── tray.rs            # Show / Hide / Quit tray menu (best-effort)
│       └── native_host.rs     # TCP listener on 127.0.0.1:9157 → enqueue URLs;
│                              #   exposes NativeHostStatus (bound + last_event_unix) for
│                              #   the Settings → Extensions tab.
├── crates/
│   ├── engine/                # UI-agnostic download engine (testable, no Tauri)
│   │   ├── Cargo.toml         # tokio, reqwest (rustls+stream+cookies+http2), rusqlite
│   │   │                      #   (bundled), serde, sha2, chrono, uuid, thiserror…
│   │   ├── src/
│   │   │   ├── lib.rs         # module list + re-exports
│   │   │   ├── model.rs       # Download, ChunkState, DownloadStatus, ChecksumSpec,
│   │   │   │                  #   Settings, Category, ProxyMode, effective_proxy_url()
│   │   │   ├── manager.rs     # DownloadManager (Arc-cheap clone). Owns tasks, scheduler,
│   │   │   │                  #   limiter, spawner. With_settings_and_spawner() entry point.
│   │   │   ├── scheduler.rs   # DownloadScheduler: Semaphore-based slots + per-download conn cap
│   │   │   ├── limiter.rs     # TokenBucket + CombinedLimiter (global + per-download; more-restrictive wins)
│   │   │   ├── control.rs     # DownloadControl: AtomicBool pause/cancel + tokio Notify
│   │   │   ├── protocol.rs    # HEAD/range probe, filename extraction, chunk planning, Content-Range parsing
│   │   │   ├── chunk.rs       # download_chunk(): one range GET, throttled, honours pause/cancel
│   │   │   ├── task.rs        # build_client/build_probe_client (raw bytes, custom UA, no
│   │   │   │                  #   transparent decompression). run_download() pipeline.
│   │   │   ├── storage.rs     # SQLite (rusqlite bundled). Tables: downloads, history,
│   │   │   │                  #   categories, settings. delete_download() for hard-removes.
│   │   │   └── config.rs      # default_categories() + load_or_default()
│   │   └── tests/integration.rs  # Spawns a local HTTP server, drives DownloadManager end-to-end
│   └── native-host/           # Standalone bin: dm-native-host (Chrome native-messaging shim)
│       └── src/main.rs        # read/write 4-byte-LE-prefixed frames, extract_media_urls, forward_url
├── browser-extension/         # MV3 (manifest_version: 3) extension
│   ├── manifest.json          # Chrome / Edge / Brave / Arc + Firefox 109+ (unified)
│   │                          #   Chrome treats this as a service worker; Firefox reads
│   │                          #   `browser_specific_settings.gecko` and runs the
│   │                          #   `background.scripts` array as a non-persistent event page.
│   ├── background.js          # MV3 background page → chrome.runtime.connectNative(...)
│   │                          #   No ESM imports — works in both service-worker (Chrome)
│   │                          #   and event-page (Firefox) contexts. `chrome.runtime.lastError`
│   │                          #   is captured in the same tick as `connectNative` (Firefox
│   │                          #   clears it on the next runtime API call).
│   ├── content.js             # scans <video>/<source>/<audio>/<a> for media URLs + HLS/DASH hints
│   ├── popup.html / popup.js  # "Grab page media" + "Open DM" + "Test host connection" buttons
│   │                          #   (themed; shows host status + a red error block with the
│   │                          #   actual `chrome.runtime.lastError.message` and a hint).
│   ├── com.app.dm.native.json           # Cross-browser host-manifest template (uses both
│   │                                    #   `allowed_origins` and `allowed_extensions`).
│   ├── com.app.dm.native.chrome.json    # Chrome-only template (`allowed_origins` only,
│   │                                    #   no `REPLACE_WITH_…` placeholder leak).
│   ├── com.app.dm.native.firefox.json   # Firefox-only template (`allowed_extensions` only,
│   │                                    #   no `chrome-extension://…` URL at all).
│   └── INSTALL.md             # Build + register the native host, per-browser notes
├── .pkgconfig-shim/           # *.pc shims for libappindicator3 / ayatana-appindicator3
├── build-with-shim.sh         # Wraps `npm run tauri build`; prepends .pkgconfig-shim to PKG_CONFIG_PATH
├── coredump/                  # Local Tauri/webview crash dumps (gitignored)
├── images/                    # README screenshots (gitignored)
├── dist/                      # Vite build output (gitignored)
├── target/                    # Cargo workspace target dir (unified at root, gitignored)
├── UI_DESIGN_PLAN.md          # Detailed IDM-parity design intent (read before UX changes)
├── notes/session-logs/        # Free-form per-session notes
└── .pi/tasks/                 # Plannotator / agent harness scratch (gitignored)
```

## Engine architecture (Rust)

The engine is the heart of DM. **`crates/engine` has zero Tauri deps** so it is fast to compile and fully unit-testable.

- **`DownloadManager`** owns every active task (`HashMap<id, TaskEntry>`) plus shared `RunContext` (scheduler + global limiter + storage + event sink + max_connections + proxy + spawner). Cheap to clone (wraps `Arc<Inner>`).
- **Runtime injection (`Spawner`)** — the engine never calls `tokio::spawn` directly. It stores a `Spawner = Arc<dyn Fn(BoxFuture<'static, ()>) + Send + Sync>` so callers in non-Tokio contexts (e.g. Tauri's default command workers) can hand in their own. `with_settings_and_spawner()` is the entry point the Tauri shell uses; the Tauri side passes `tauri::async_runtime::handle().clone()` so tasks run on Tauri's runtime. The default constructor falls back to `Handle::current()` so the engine tests (which already run on a Tokio runtime) still work.
- **`DownloadScheduler`** uses two semaphores: global `download_slots` and per-id `per_download` connection map. `set_limits(...)` rebuilds the slot semaphore and clears per-id state on settings change.
- **`CombinedLimiter`** = global token bucket + per-download bucket; `acquire()` spends from both. `TokenBucket` rate `0` = unlimited; capacity = 0.2s burst.
- **`DownloadControl`** (`control.rs`) — `AtomicBool` pause/cancel plus a `tokio::sync::Notify`; `wait_while_paused().await` returns true if cancellation was requested. `pause()` and `resume()` also clear any stale `d.error`.
- **`run_download()`** flow: `acquire_download` slot → mark `Connecting` → probe (HEAD, fallback to range GET) → plan chunks (or single-chunk fallback for unknown size / no-range servers) → `Downloading` → spawn N chunk workers → aggregate progress (throttled every 150ms) → finalise: SHA-256 verify if checksum, mark `Completed`/`Error`/`Canceled`, append to `history` table.
- **`chunk.rs`** opens the file with `tokio::fs::OpenOptions`, `seek`s to `chunk.resume_offset()`, writes each response buffer through the limiter, polls control flags, and surfaces `ChunkError::Canceled` if the user cancelled.
- **`storage.rs`** uses `rusqlite` with the **`bundled` feature** (no system SQLite needed). All writes go through `Mutex<Connection>`; tables are created idempotently by `migrate()` on open. `Storage::open_memory()` exists for tests. `Storage::delete_download(&self, id)` issues a `DELETE FROM downloads WHERE id = ?1` and is called by `DownloadManager::remove()` so "Remove" persists across restarts (the append-only `history` row is intentionally left in place).
- **`protocol.rs`** does filename extraction (RFC 5987 `filename*=UTF-8''...` → quoted `filename=...` → URL path segment → `fallback`), `decide_chunk_count` (~1 chunk per MiB capped at `max_connections`), and `plan_chunks` (inclusive ranges, evenly divided with remainder bytes pushed to earlier chunks).
- **HTTP client (`task.rs`)** — `build_client` / `build_probe_client` construct reqwest with:
  - `user_agent(DEFAULT_USER_AGENT)` where `DEFAULT_USER_AGENT = "Mozilla/5.0 (compatible; DM-DownloadManager/0.1; +https://github.com/dm-project/dm)"` plus default `Accept: */*` and `Accept-Language: en-US,en;q=0.9,*;q=0.8`. Bare `reqwest/x.y.z` is 403'd by many CDNs/mirrors.
  - `.gzip(false).brotli(false).deflate(false)` — transparent decompression is **disabled**. Some servers (notably `mirrors.tuna.tsinghua.edu.cn` directory listings) lie about `Content-Encoding: gzip` while serving non-gzipped bytes, which surfaces as a generic `error decoding response body`. The chunk stream saves the wire bytes verbatim, which is what an IDM-style manager wants anyway.
  - `proxy = effective_proxy_url()` from `Settings` (or a per-download override on `Download.proxy`).
- **Proxy model (`model.rs`)** — `ProxyMode` = `None | System | Manual`. `Settings::effective_proxy_url()`:
  - `None` → `None`.
  - `System` → reads `HTTPS_PROXY` then `HTTP_PROXY` env vars (lets reqwest's auto-detection handle `NO_PROXY`).
  - `Manual` → `Settings.proxy` if non-empty.
  `proxy_mode` uses `#[serde(default)]` so old DB rows deserialize as `System`. Per-download `Download.proxy` always wins over the global one. `DownloadManager::set_global_proxy(mode, url)` is the cheap path that updates just the proxy without touching the rest of `Settings`.

### Status lifecycle

`Queued` → `Connecting` → `Downloading` → (`Paused` ⇄ `Queued`) | `Completed` | `Error` | `Canceled`. The `Scheduled` status parks downloads outside the user's schedule window (see `run_schedule_loop` in `manager.rs` — 15s ticker, pauses active → `Scheduled`, promotes `Scheduled` → `Queued` when the window reopens).

### Events

`manager::DownloadEvent` is converted 1:1 to `events::FrontendEvent` and emitted on Tauri channel **`download-event`** as `{ kind: "added" | "progress" | "statusChanged" | "completed" | "error" | "removed", download: Download }` (the `removed` variant carries `{ id }` only). Frontend listens in `stores/downloads.ts::startEventListener()` and merges into the Svelte store.

## Tauri shell (`src-tauri`)

- **Logging first**: `main.rs` calls `logging::init()` before the Tauri builder so every log line is mirrored to a fresh `~/.local/share/DM/YYYYMMDD-HHMMSS.log` file (with `XDG_DATA_HOME` overrides) **and** to stderr. The returned `LogGuard` exposes `.path()` for "Report a bug" dialogs. The shim is a `log::Log` impl — no new deps, the engine already uses `log`.
- **Plugins registered**: `clipboard-manager`, `notification`, `autostart` (macOS LaunchAgent).
- In `setup()`: open SQLite at `dm.sqlite`, construct `DownloadManager::with_settings_and_spawner(...)` injecting a spawner built from `tauri::async_runtime::handle().clone()`, wire the event sink (clone of `AppHandle`, emits via `Emitter`), call `app.manage(manager.clone())`, spawn the native-host TCP listener on `127.0.0.1:9157`, and start the schedule loop.
- Tray construction is wrapped in `if let Err(e) = ...` and logged; the app keeps running if libayatana-appindicator cannot write its cache.
- **`commands.rs`** is the entire frontend-facing command surface: `ping`, `add_download`, `list_downloads`, `get_download`, `pause_download`, `resume_download`, `cancel_download`, `remove_download`, `set_speed_limit`, `set_global_speed_limit`, `set_proxy`, `get_settings`, `update_settings`, `save_dir_for`, `notify_on_complete`, `probe_native_host`.
- `capabilities/default.json` lists the precise plugin permissions granted to the main window.
- Identifier is **`com.dm.downloadmanager`** (NOT `com.dm.app` — the old id collided with `.app` on macOS).

## Native-messaging bridge (`crates/native-host` + `src-tauri/src/native_host.rs`)

A pure-stdio binary (`dm-native-host`) reads Chrome native-messaging frames (4-byte little-endian length + UTF-8 JSON), runs the HTML through `extract_media_urls` (quoted-URL scan; keeps URLs ending in a media extension, containing `.m3u8`, or containing DASH `manifest`), and `forward_url()` posts `{"url":"..."}\n` to the TCP listener inside the running app. Port defaults to **9157** (override via `DM_NATIVE_HOST_PORT`); the same constant lives in `src-tauri/src/native_host.rs::DEFAULT_PORT`. Two unit tests cover frame round-trip and extraction.

`src-tauri/src/native_host.rs` additionally tracks `NativeHostStatus { bound: AtomicBool, last_event_unix: AtomicU64 }`. The `probe_native_host` Tauri command returns a snapshot; the Settings → Extensions tab polls it every 3s.

The browser extension MV3 background connects via `chrome.runtime.connectNative("com.app.dm.native")` and relays URLs from `chrome.downloads.onCreated`, from the content script's media scrape, and from the popup's manual "Grab" button. **A single `manifest.json` serves both Chrome/Edge/Brave/Arc and Firefox 109+**: Chrome reads `background.scripts` as a service worker (Chrome 121+); Firefox 109+ reads the same array as a non-persistent event page and uses the `browser_specific_settings.gecko.id = dm-grabber@dm-project` block to fix the extension ID. Firefox's `about:debugging` → "Load Temporary Add-on…" requires `background.scripts` and rejects `background.service_worker` (a transitional state in Firefox 109–127), so the array form is mandatory; `background.service_worker` was removed entirely.

**Chrome extension ID must match `allowed_origins` in the host manifest.** Every Chrome extension installed via "Load unpacked" gets a unique random ID. The native-messaging host manifest at `~/.config/google-chrome/NativeMessagingHosts/com.app.dm.native.json` ships with the literal placeholder `chrome-extension://REPLACE_WITH_YOUR_CHROME_EXTENSION_ID/` — the user **must** copy the actual ID from `chrome://extensions` and replace that string in `allowed_origins` (and re-launch Chrome to pick up the manifest change). If they skip this, Chrome refuses to start the host with `Access to the specified native messaging host is blocked`, the badge turns red, the popup shows `Native host not running`, and downloads silently never arrive. INSTALL.md calls this out explicitly but it is the most common install failure. Firefox is fine — the host manifest's `allowed_extensions` already contains the fixed `dm-grabber@dm-project` Gecko ID.

**The native host is a dumb pipe; don't filter URLs by file extension.** The Rust `dm-native-host` binary used to accept only `MEDIA_EXTS` (`.mp4`, `.webm`, `.mkv`, ...) and HLS/DASH manifests, silently dropping everything else — including `.zip`, `.pdf`, `.iso`, `.exe` that the content script finds on a download page. The user would see "I clicked save-as, nothing happened" with no diagnostic. The host now uses a permissive `looks_like_downloadable_url` (must be `http(s)://`, must not be `javascript:` / `data:` / `about:`, must have a non-trivial path) and forwards **every** http(s) URL the extension captures. The Tauri engine's `add_download` is the right place to reject unsupported schemes / 4xx / 5xx — the user gets a real error message there, not a silent drop.

**Content script is opt-in only — no auto-scrape on every page.** The previous version of `content.js` ran `collectMedia()` once on `document_idle` AND kept a `MutationObserver` that re-scanned every 2 s. Every page the user visited (and every DOM change on that page) auto-sent media URLs to DM. The result: opening YouTube, Twitch, a news site with embedded video, or any re-rendering single-page app flooded DM with queued downloads the user never asked for. The current version responds to the `collect` message only — the popup's **Grab page media** button is the *only* trigger. The auto-scrape path is intentionally removed; if a future version wants to add it back, gate it on a user-controlled "auto-grab" preference stored in `chrome.storage.sync`.

**`chrome.downloads.onCreated` is filtered by install time.** Chrome re-fires `onCreated` for downloads that were already in the user's history at the moment the extension was installed (a documented re-fire to help extensions catch up after a crash). For a download manager, that turns "I just installed DM Grabber" into "DM is now clogged with 200 old downloads I never asked for". The service worker records the install time in `chrome.storage.local` (key `dmInstallTime`) on the very first `onInstalled({reason: "install"})` event and only on that event — updates do **not** reset the timestamp, so the filter continues to ignore pre-update downloads. The filter drops any `DownloadItem` whose `startTime` is missing, unparseable, or before `installTimeMs`. **`onCreated` is registered only after `loadInstallTime()` resolves** (the boot sequence is `loadInstallTime().then(registerDownloadsListener).then(connect)`), so there is no race where a download fires before the filter is armed. `chrome.storage.local` survives service-worker restarts and extension reloads; clearing it manually falls back to `Date.now()` at boot.

**The `media` message type is a hard boundary, not a soft one.** The previous content-script auto-scrape path used to send `{type: "media", urls: [...]}` on every page load / DOM mutation, which flooded DM. The content script no longer auto-sends, and the background's `onMessage` handler now **drops `type === "media"` on the floor** — even if a stale cached content script (or a third-party script) tries to send it, the background refuses. Only `type === "grab"` (the popup button's explicit ask) is accepted. This is a deliberate, documented boundary so the next refactor can't accidentally re-enable passive capture by leaving a "harmless" `media` branch in the handler.

**Reloading the extension is required after pulling new code.** Chrome's MV3 service worker is long-lived and **does not pick up file changes on disk** — the user must click **Reload** on the extension card in `chrome://extensions`, or restart Chrome, for new `background.js` / `content.js` to take effect. If the user reports "I pulled the fix but it's still broken", the first thing to check is the service worker's "Inspect views" / "Service worker" link in `chrome://extensions` and confirm the source actually shows the new code.

**The extension **takes over** Chrome's download, it doesn't double it.** The `chrome.downloads.onCreated` listener in `background.js` does two things: forward the URL to DM **and** call `chrome.downloads.cancel(downloadId)` + `chrome.downloads.erase({id, removeFromDisk: false})` on Chrome's own copy. Without the cancel, the upstream — especially a single-connection CDN mirror — sees two racing connections and may truncate one of them, leaving DM with a tiny stub body that the engine then wrongly auto-completes. With the cancel, DM is the only one talking to the mirror and the body stream is the real file. The `removeFromDisk: false` is intentional: Chrome's partial file is not the real file and we don't want to keep it around in the user's Downloads folder.

**Open-ended auto-completion requires a minimum body size.** When the probe couldn't discover `Content-Length` (e.g. the proxy didn't relay it), the engine creates a single chunk with `end = u64::MAX` and treats end-of-stream as completion. **Without a guard, a 879 B body that closes cleanly is marked `Completed`**, which is the user-reported "879 B downloaded, labeled Completed" failure. The engine now requires `d.downloaded >= OPEN_ENDED_MIN_BYTES` (1 KB, set in `task.rs`) before auto-completing an open-ended transfer; below that, the transfer is marked `Error` with a clear "open-ended transfer truncated at N B" message. 1 KB is below the size of any reasonable user-facing download and well above the size of an HTML error page or 302-redirect stub. Regression test: `truncated_body_marks_error_not_completed` in `crates/engine/tests/proxy_strict_range.rs`.

**`fmtBytes` is unit-aware up to YB; open-ended chunks render a hint instead of a fake size.** The byte formatter in `src/lib/utils/formatters.ts` extends its unit array through `YB` and shows `≥1024 YB` for anything larger (so `u64::MAX` no longer renders as the misleading `"16777216.0 TB"` it used to — that came from capping at `TB` and dividing). Additionally, the chunk display in `DownloadRow.svelte` detects open-ended chunks (`chunk.end` not finite or `> Number.MAX_SAFE_INTEGER`) and renders `879 B · unknown total` instead of `879 B / 16 EB`, so the sentinel can never leak into the UI.

## Frontend (`src/`)

- **Composition root** is `App.svelte`. It mounts in `src/main.ts`, wires stores, clipboard monitor, drag-and-drop, global ⌘K shortcuts, the demo data mode, and composes the sidebar + list + status bar from `lib/components/*`. Stores import names: `downloads`, `aggregateSpeed`, `counts`, `totalDownloaded`, `refreshDownloads`, etc.
- **`lib/api.ts`** — one wrapper per Tauri command. Tauri v2 camelCase keys → snake_case Rust args. Use these wrappers, not raw `invoke()`.
- **`lib/types.ts`** — TS mirrors of the Rust model in camelCase, plus the `FrontendEvent` discriminated union. Stay in sync with `crates/engine/src/model.rs` and `src-tauri/src/events.rs`.
- **`lib/design-tokens.ts`** is the single source for color/spacing/radius/etc. Values are CSS-var compatible and consumed by `app.css`.
- **`lib/stores/`** — `downloads.ts` (state + CRUD + live event subscription + 1Hz speed-monitor tick + smart-folder bucketing), `ui.ts` (sidebar, filters, modals, toasts, clipboard, drag, status-bar metrics, theme), `settings.ts` (persisted, with revert-on-error).
- **`lib/hooks/useKeyboardShortcuts.ts`** — register a `Shortcut[]`, gets cleaned up on unmount; inputs/textarea skip non-global combos except `Escape`.
- **`lib/utils/fuzzySearch.ts`** — exact-substring-first scoring, consecutive/word-start bonuses, `<mark>` highlighting. `lib/utils/formatters.ts` — bytes/rate/percent/duration/relative time/date/truncate/url helpers + `debounce`/`throttle`. `lib/utils/env.ts` — `browser` and `isMac` runtime detection.
- **Theme system**: `[data-theme="dark"|"light"]` is applied to `<html>` by an inline script in `index.html` before paint (no FOUC). Source of truth is `$theme` in `src/lib/stores/ui.ts` (`ThemeKey = "dark"|"light"|"system"`), persisted to `localStorage["dm-theme"]`; `effectiveThemeStore` derives the active theme from the system preference when "system" is selected. Toggle UI lives in `SettingsTabs.svelte` (Appearance tab) with three preview cards. All theme-aware variables are in `app.css` `:root` (dark) and `[data-theme="light"]` (light); non-color tokens stay in a separate `:root` block. The CSS uses `color-scheme: dark/light` to opt native form controls in. The sidebar collapse is responsive via `@media (max-width: 900px)` and the explicit `.shell.collapsed` user toggle.
- **DropdownMenu positioning**: `DropdownMenu.svelte` uses `position: fixed` with the trigger's measured rect + viewport dims, flips above the trigger when there isn't enough room below, and clamps horizontally into the viewport. It re-measures on resize/scroll while open, uses `requestAnimationFrame` for non-zero `offsetHeight`, applies a 3px deadband, pauses repositioning while the pointer is over the menu, and resets `pos` on close. A CSS triangle caret (`.caret` pseudo-element) always points at the trigger; its horizontal position is clamped to `[14, menuW-14]` so the tip never pokes past the rounded corners.
- **Overscroll containment in floating panels**: every scrollable region inside a `position: fixed` modal / drawer / popover (Settings' `.tabs` + `.tab-content`, CommandPalette's `.results`, etc.) must declare `overscroll-behavior: contain`. Without it, when the user reaches the top or bottom of the inner scroll region, the browser "scroll-chains" to the next scrollable ancestor — which is the page behind the panel — and the home view scrolls under the modal. `contain` makes the browser consume the event at the panel's boundary.
- **Modal-aware scroll, not pointer-aware**: while a modal-style panel is open, the wheel must operate **only** on the panel — never on the home view underneath, regardless of where the pointer is. The default browser behaviour is pointer-aware (wheel scrolls whatever's under the cursor; the backdrop then cascades to body). `App.svelte` maintains a `lockCount` refcount that sets `overflow: hidden` (via `setProperty('overflow','hidden','important')`) on both `<html>` and `<body>` whenever `$showSettings || $showCommandPalette`, and restores the previous overflow on close. The refcount lets stacked modals work without leaking a locked body. Belt-and-braces: `onDestroy` drains the count, and a 0→1 transition stashes the existing inline overflow values so we never clobber a non-default one. New modal-style components should add their open flag to the reactive statement; do **not** rely on `pointer-events: none` on the backdrop alone, because wheel events still fire on a non-scrollable backdrop and the cascade reaches the document.

The frontend has a **demo mode** (`$demoMode` store + `startDemoSim`) that simulates progress locally so the UI can be exercised without the Rust backend (handy in plain `vite preview`).

## Build, run, test

```bash
# Frontend only (no Tauri)
npm install
npm run dev          # Vite on http://localhost:5173 (Tauri expects this exact URL)
npm run build        # → dist/
npm run check        # svelte-check, must be 0 errors / 0 warnings

# Engine unit + integration tests
cargo test -p dm-engine
# + the multi-thread integration suite in crates/engine/tests/integration.rs

# Full Tauri build
./build-with-shim.sh           # Linux (Fedora 40+ etc.): prepends .pkgconfig-shim to PKG_CONFIG_PATH
# or on stock Debian/Ubuntu:
npm run tauri build

# Native-messaging host standalone
cargo run -p dm-native-host

# Release binary (Cargo workspace unifies target dirs):
ls target/release/dm-tauri     # ~23 MB stripped, dynamically linked to webkit2gtk-4.1
```

### Fedora 40+ / similar Linux

`libappindicator3-0.1.pc` and `ayatana-appindicator3-0.1.pc` are no longer shipped. The bundler needs them. The repo carries `.pkgconfig-shim/` with shim `.pc` files that point at the modern `appindicator` library; `build-with-shim.sh` injects that directory into `PKG_CONFIG_PATH` before invoking `npm run tauri build`. **Always use `build-with-shim.sh` on Fedora.**

## Conventions & gotchas

- **Cargo workspace at the repo root** (`Cargo.toml`) unifies `target/` to the workspace root — not `src-tauri/target/...`. Look for the release binary at `target/release/dm-tauri`. `dist/`, `coredump/`, `images/`, and `target/` are all gitignored.
- **Tauri identifier**: `com.dm.downloadmanager`. Do not change without also updating the macOS bundle name.
- **Rust mutex rules**: never hold a `std::sync::Mutex` guard across an `.await`. Clone the value out, drop the guard, then await.
- **Runtime injection**: the engine never calls `tokio::spawn` itself — it goes through the injected `Spawner`. Sync Tauri command handlers are dispatched on a non-Tokio worker thread, so any engine path that spawns a task MUST go through a manager constructed with `with_settings_and_spawner`. The default constructor only works when called from inside a Tokio runtime.
- **HTTP client quirks**: always use the engine's `build_client`/`build_probe_client` (custom UA, no transparent decompression). A naked `reqwest::Client` will get 403'd by many CDNs and will fail on servers that lie about `Content-Encoding`.
- **Open-ended chunks (unknown total size)**: when the probe returns no `Content-Length` and no `Accept-Ranges`, the engine creates a single chunk with `end = u64::MAX` and falls back to a **plain `GET` with no `Range:` header** (`chunk.rs::download_chunk`). Many CDN proxies (cdn.akaere.online and similar reverse-proxy fronts) advertise no `Content-Length` on HEAD and then **reject** any `Range:` request with 416 because they don't know the upstream's total size — sending `Range: bytes=0-` to one of these surfaces as `error: 416 Range Not Satisfiable` and the download hard-fails. The fix is to omit the `Range:` header entirely for `end == u64::MAX` chunks, truncate the file, and stream the body from offset 0. Trade-off: open-ended downloads are **not resumable across restarts** (a non-zero `chunk.downloaded` is stale and gets overwritten on the next attempt). The single-connection path is gated on `can_resume = false` already. `ChunkState::size()` returns `u64::MAX` for open-ended chunks to avoid an `add with overflow` panic in `remaining()`.
- **Proxy precedence**: per-download `Download.proxy` always wins; otherwise the global `Settings.effective_proxy_url()` (None/System/Manual) is used. `set_global_proxy` is the cheap update path; `update_settings` rebuilds more state.
- **Status throttling**: progress events are emitted at most every 150ms in the engine; the frontend's speed-monitor tick is 1Hz — both numbers are load-bearing for UI performance.
- **Speed limits**: `0` = unlimited, both at the bucket and at the `CombinedLimiter` level.
- **Schedule window**: stored as `(hour, minute)` tuples. Wrap past midnight is supported (`start > end` ⇒ window covers the two halves).
- **Pause/Resume clear stale errors**: `pause()` and `resume()` reset `d.error`. The DownloadRow meta row + expanded panel only render the error pill when `status === "error"`.
- **Remove must persist**: `DownloadManager::remove()` cancels the task, removes the in-memory entry, and calls `Storage::delete_download`. Without the storage call the row would re-appear on next launch via `load_active()`.
- **Progress persists while transferring** (`task.rs`): the aggregator flushes the SQLite row on **either** `PROGRESS_FLUSH_INTERVAL` (1 s) **or** `PROGRESS_FLUSH_BYTES` (1 MiB), whichever fires first. The UI throttle (150 ms) and the persistence throttle are independent. A hard kill mid-transfer therefore loses at most ~1 s of progress. The aggregator also issues a final flush on the stream-end path. **Without this flush, "starts at 7% instead of 30%" is the default behaviour** — only status transitions used to persist, and there are no status transitions during steady-state downloading.
- **Resume math**: chunk workers use `chunk.resume_offset() = chunk.start + chunk.downloaded` to build the `Range:` header. If a chunk is already fully covered (`remaining() == 0`, e.g. one of the eight chunks finished before the kill), the engine **skips spawning it** instead of issuing a malformed `bytes=N-N-1` range. This keeps resume robust against partially-complete chunk layouts.
- **Persistence surface — what survives a restart**: the on-disk SQLite row is the source of truth for `downloaded`, per-chunk `downloaded`, `status`, `filename`, `save_path`, `total_size`, `speed_limit`, `proxy`, `checksum`, `error`, `content_type`, `can_resume`, and the chunk layout. Settings (global + per-category), categories, and history all live in their own tables. The frontend's Svelte stores are *not* persistent — they rehydrate from `get_settings()` / `list_downloads()` on launch.
- **Adding a new Tauri command**: define it in `src-tauri/src/commands.rs`, register in `invoke_handler!`, add the corresponding wrapper in `src/lib/api.ts`, mirror any new types in `src/lib/types.ts`, and update capabilities if it needs a plugin permission.
- **Adding a new download model field**: change `crates/engine/src/model.rs`, update `storage.rs` schema/migration (or `save_download`/`self_row_to_download`), update `src/lib/types.ts`, update any UI consumption, and add or extend the integration test in `crates/engine/tests/integration.rs`. For settings fields, prefer `#[serde(default)]` so old DB rows still load.
- **Before any UX work**, read `UI_DESIGN_PLAN.md` (288 lines) — it is the canonical IDM-parity target.
- **`.pi/` is gitignored** — it's Plannotator/agent harness scratch. Don't commit from it.
- **`.taurignore` (project root) keeps the Tauri CLI's walker from EMFILE-panicking**: the Tauri CLI uses the `ignore` crate (`ignore::WalkBuilder`) to walk the project tree for `tauri dev` file watching and for source-file discovery during `tauri build`. The walker does **not** apply `max_open(1)` back-pressure, so on a project whose `target/` and `node_modules/` together hold tens of thousands of files (this repo's `target/release` alone is 20k+ files), it can exhaust the per-process FD limit and panic with `Too many open files (os error 24)` at `crates/tauri-cli/src/interface/rust.rs` (the `Result::unwrap()` on the walker iterator). The Tauri CLI is a precompiled native Node addon (`node_modules/@tauri-apps/cli-linux-x64-gnu/cli.linux-x64-gnu.node`) so we cannot patch the walker. The supported fix is a project-root `.taurignore` (gitignore-compatible syntax) that excludes `target/`, `**/target/`, `node_modules/`, `dist/` (Tauri reads `dist/` via the `frontendDist` config, not by walking), `coredump/`, `images/`, `notes/`, `.pi/`, and editor metadata. The CLI looks for `.taurignore` in every directory it walks and applies the patterns in addition to its built-in defaults.

## When you're stuck

- Engine behaviour unclear → read `crates/engine/src/manager.rs::run_download` and `task.rs` together; they tell the same story from different angles.
- "Spawn failed" / "no reactor running" errors → the path that needs `tokio::spawn` was reached from a non-Tokio thread; route it through the injected `Spawner` (see `src-tauri/src/main.rs::setup`).
- "error decoding response body" / "Invalid gzip header" mid-download → the server is lying about `Content-Encoding`. Don't re-enable transparent decompression; the engine already disables it and saves raw bytes.
- 403 on a mirror (Tuna, kernel.org, sourceforge, etc.) with a `reqwest/x.y.z` UA → call site is bypassing `build_client`/`build_probe_client`; route through the engine helpers.
- Event/serialisation mismatch → compare `crates/engine/src/model.rs`, `src-tauri/src/events.rs`, `src/lib/types.ts`, and `src/lib/api.ts` in that order.
- "Remove" not surviving a restart → `Storage::delete_download` isn't being called from `DownloadManager::remove`.
- "Starts at 7% instead of 30%" / progress lost on a hard kill → the aggregator's `PROGRESS_FLUSH_*` throttle (`task.rs`) is missing or disabled. Confirm the flush block in the `agg` task still calls `ctx.storage.save_download(&d)` on every `PROGRESS_FLUSH_INTERVAL` / `PROGRESS_FLUSH_BYTES` boundary, plus one final flush on stream end. A regression in the aggregator will silently lose progress because no other code path persists `downloaded` mid-transfer.
- Tray/indicator build failures on Linux → check `.pkgconfig-shim/` and confirm you're running `./build-with-shim.sh`.
- Dropdown menu "following the wrong row" / "two-position jump" → likely a stale `transform` on the row/actions container or a missed `DropdownMenu` reset; see the `DropdownMenu positioning` section above.
- Tests failing on connection caps → scheduler tests use `tokio::time::timeout(100ms, ...)` to assert blocked acquires; do not increase the timeout without understanding the test intent.
- Browser extension: native host not connecting → confirm the bundled `com.app.dm.native.json` path points to the release binary, and that the `127.0.0.1:9157` TCP listener in the app is up (Settings → Extensions tab shows the live `bound` + `last_event_unix` from `NativeHostStatus`).
