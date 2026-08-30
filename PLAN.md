# Plan: Rust + Tauri GUI Download Manager (IDM equivalent)

> Status: DRAFT v2 — architecture confirmed: **Tauri 2 (web UI)**, **cross-platform**,
> **SQLite**, and all four extras (clipboard+drag-drop, system tray, scheduler, browser extensions).
> Checksum verify + proxy support are baked into the core engine by default.

## Context

Greenfield repo (empty git). Goal: a **direct equivalent of Internet Download Manager (IDM)** —
a desktop app that accelerates downloads by splitting a file into parallel HTTP Range connections,
with pause/resume, queueing, speed limiting, categories, and a live progress dashboard.

Per the scoping decisions:

- **Frontend = web UI inside Tauri** (HTML/JS/TS webview); Rust is the backend.
- **Cross-platform** (Linux/macOS/Windows) from one codebase; OS-specific bits (tray, notifications,
  native-messaging registration) live behind small per-OS shims.
- **SQLite** for history/queue/resume.
- v1 includes: core engine + **clipboard capture + drag-drop**, **system tray**, **scheduler**,
  **browser extensions + video grabber**. Checksum + proxy are core-engine defaults.

## Approach (recommended)

**Tauri 2.x application.** The download engine is a Rust library living in the Tauri backend, held in
Tauri managed state. The web frontend (Svelte + Vite — *see Open decision*) calls Rust via Tauri
`invoke` commands and receives live updates via Tauri `emit`/`listen` events. The engine itself is
UI-agnostic and unit-testable without the webview, so we can validate it headless first.

### Architecture

```
 Browser/Clipboard/Tray ──► Web frontend (Svelte+Vite, webview)
                                │ invoke() / listen()
                                ▼
 Tauri backend (Rust, tokio)  DownloadManager  ◄── managed state
   ├─ commands.rs   add / pause / resume / cancel / set_limit / list / history
   ├─ events.rs     emit("download:progress" | "download:done" | "download:error")
   ├─ engine/
   │    manager.rs  owns tasks map, global concurrency, command channel
   │    task.rs     per-file state machine + resume-offset plan
   │    chunk.rs    one Range GET → seek-write at file offset
   │    scheduler.rs semaphore: max concurrent downloads + conns/download
   │    limiter.rs  token bucket: global + per-download speed cap
   │    protocol.rs HEAD probe, Accept-Ranges/length, range negotiation
   ├─ storage/db.rs SQLite: downloads, chunks, history, categories, settings
   ├─ config.rs     settings + categories (serde, platform config dir)
   └─ tray.rs / scheduler.rs / clipboard.rs / native_host.rs (extras)
        │
 Native messaging socket (localhost) ◄── browser-extension host binary
```

- **DownloadManager**: `Arc<Mutex<HashMap<Id, TaskRuntime>>>` + `tokio::sync::mpsc` command channel.
  A background pump applies commands and broadcasts throttled progress events (~5–10 Hz) to the webview.
- **DownloadTask**: state machine `Queued → Connecting → Downloading → Paused → Completed → Error`.
  Probes `Content-Length` + `Accept-Ranges`; if range unsupported → single-connection fallback.
- **Chunk**: one range connection; writes to seekable async file at its offset; reports bytes.
- **Scheduler**: `tokio::sync::Semaphore` for global download concurrency + per-task connection count.
- **SpeedLimiter**: per-download and global token bucket throttling chunk writes.
- **Storage**: `rusqlite` (bundled) for tasks/chunks/history/categories/settings; resume by reading
  completed chunk offsets on startup.
- **Config**: `serde` + `directories` crate for platform config/data paths.
- **Checksum**: `sha2` streamed hash; verify against optional provided hash; stored on history row.
- **Proxy**: `reqwest` proxy (HTTP/HTTPS/SOCKS), global or per-download from config.

### Tech stack

| Concern            | Choice |
|--------------------|--------|
| App shell          | Tauri 2.x (`@tauri-apps/cli`, `tauri` crate) |
| Frontend           | **Svelte + Vite + TypeScript** (recommend; React/Vue alternative) |
| Charts             | `uplot` (tiny, streaming-friendly) for the speed graph |
| Async runtime      | `tokio` (Tauri's runtime) |
| HTTP client        | `reqwest` (Range, redirect, cookies, proxy, TLS) |
| Persistence        | `rusqlite` (bundled SQLite) |
| Config paths       | `directories` |
| Serialization      | `serde`, `serde_json` |
| Clipboard          | `@tauri-apps/plugin-clipboard-manager` |
| Tray               | Tauri v2 tray API + `@tauri-apps/plugin-*` (tray/autostart/notification/opener) |
| Scheduling         | `chrono` timers in backend |
| Checksum          | `sha2` |
| Browser ext        | MV3 extension + native-messaging host binary (localhost socket IPC) |

### Proposed layout

```
DM/
  package.json, vite.config.ts, tsconfig.json, index.html   # frontend
  src/ (frontend)  App.svelte, lib/, components/{List,Details,Graph,AddUrl,Settings}.svelte
  src-tauri/
    Cargo.toml  tauri.conf.json  build.rs  capabilities/*.json  icons/
    src/
      main.rs            # Tauri builder: register plugins, manage(Manager), commands, tray, events
      commands.rs        # #[tauri::command] add/pause/resume/cancel/set_limit/list/history/settings
      events.rs          # event name consts + emit helpers
      config.rs          # Settings, Category load/save
      model.rs           # Download, Chunk, Status, Category, Config (serde, shared with frontend types)
      engine/{manager,task,chunk,scheduler,limiter,protocol}.rs
      storage/db.rs      # SQLite schema + queries
      tray.rs            # system tray menu + minimize-to-tray
      scheduler.rs       # timed start/stop
      clipboard.rs       # clipboard URL monitor (plugin-backed)
      native_host.rs     # localhost IPC listener for browser-extension host
  browser-extension/     # manifest.json, background.js, content.js (Chrome + Firefox MV3)
  native-host/           # small Rust bin: stdio native-messaging <-> localhost socket
  tests/                 # cargo integration tests for engine
```

## Files to create (high level)

- Frontend scaffold: `package.json`, `vite.config.ts`, `index.html`, `src/**`
- `src-tauri/Cargo.toml`, `tauri.conf.json`, `build.rs`, `capabilities/*.json`
- `src-tauri/src/main.rs`, `commands.rs`, `events.rs`, `config.rs`, `model.rs`
- `src-tauri/src/engine/{manager,task,chunk,scheduler,limiter,protocol}.rs`
- `src-tauri/src/storage/db.rs`
- `src-tauri/src/tray.rs`, `scheduler.rs`, `clipboard.rs`, `native_host.rs`
- `browser-extension/*`, `native-host/*`
- `tests/*`

## Reuse

Greenfield — no existing code. Lean on:

- Tauri built-ins: `tauri::State`, `tauri::async_runtime`, `Emitter`/`Manager` for events.
- `reqwest` Range requests for segmentation; `tokio::sync::Semaphore` for concurrency.
- `rusqlite` (bundled feature) so no system SQLite needed.
- `@tauri-apps/plugin-clipboard-manager`, tray/notification/autostart/opener plugins.

## Build milestones (incremental, each independently verifiable)

- [x] **M0 — Scaffold**: `npm create tauri`, Svelte+Vite frontend, `invoke('ping')` round-trip.
      Confirm `tauri dev` opens the window. **[DONE:0]** (frontend builds + type-checks clean;
      `tauri dev` itself cannot run in this sandbox — no webkit2gtk — but all Rust/TS source is
      in place and the engine is headless-testable).
- [x] **M1 — Core engine (headless-testable)**: `model.rs`, `engine/protocol.rs` (HEAD probe +
      range plan), `engine/chunk.rs` (range GET→seek-write), `engine/task.rs` (state machine +
      resume + single-conn fallback), `engine/manager.rs` (command channel, spawn tasks),
      `storage/db.rs` (persist + resume). Add `tests/` for range math & offset writes. **[DONE:1]**
- [x] **M2 — Concurrency + speed**: `engine/scheduler.rs` (global + per-download limits),
      `engine/limiter.rs` (token bucket). `commands.rs` exposes add/pause/resume/cancel/set_limit. **[DONE:2]**
- [x] **M3 — UI + categories + checksum + proxy**: frontend List/Details/Graph/AddUrl/Settings;
      `emit` progress/done/error events; categories auto-route by extension; sha2 verify; proxy config.
      **[DONE:3]** Backend validated headless via `tests/integration.rs` (full probe→chunk→write→
      checksum pipeline against a local Range server: segmented download, single-connection fallback,
      and deliberate checksum-mismatch→Error all pass). Frontend shows live List + AddUrl + progress
      events + a category field. Category-management and checksum-entry panels were later fleshed out
      (see M7 note): `App.svelte` AddUrl accepts an optional sha256 expected hash and the Settings
      panel CRUDs Category rows (name/extensions/directory, persisted via `updateSettings`).
- [x] **M4 — Clipboard + drag-drop + tray**: clipboard-manager monitor → suggest download; window
      drag-drop of `text/uri-list`; system tray + minimize/close-to-tray; completion notifications.
      **[DONE:4]** Frontend (clipboard auto-monitor + "Add clipboard URL", window drag-drop of
      `text/uri-list`/`text/plain`, settings panel) builds + type-checks clean. `tray.rs` is written
      and `main.rs` wires `run_schedule_loop`; the Tauri binary (tray + completion notifications)
      cannot be compiled in this sandbox (no webkit2gtk) — verify with `tauri build` on a host
      with the webview deps. A `notify_on_complete` command + notification plugin is the one
      remaining wiring item (see M7).
- [x] **M5 — Scheduler**: timed start/stop of queued downloads (`chrono` timers in backend).
      **[DONE:5]** Engine enforces the schedule window: `Settings::in_schedule_window` (handles
      midnight-wrap) is unit-tested; `DownloadManager::add` parks outside-window downloads as
      `Scheduled`, `start` respects the window on resume, and `run_schedule_loop` (wired in
      `main.rs`) pauses active downloads when the window closes and promotes `Scheduled` ones
      when it reopens. Frontend has a schedule enable + start/end panel bound to `update_settings`.
- [x] **M6 — Browser extensions + video grabber**: MV3 extension (`browser-extension/`:
      `manifest.json`, `background.js`, `content.js`, `popup.html`, `popup.js`) + native-messaging
      registration manifest (`com.app.dm.native.json`), and a standalone **native-messaging host
      binary** in `crates/native-host/` (no Tauri/webview → compiles + tests in this sandbox). The
      host speaks Chrome native-messaging frames on stdin/stdout, heuristically extracts media URLs
      from captured HTML (the video grabber: `<video>`/`<source>`/`.m3u8`/DASH), and forwards each to
      the running app over a localhost TCP socket. The app-side listener `native_host.rs` is wired
      in `main.rs` (`start_native_host_listener`) and enqueues forwarded URLs exactly like
      `add_download`. **[DONE:6]** `crates/native-host` is verified headless: `cargo test -p
      dm-native-host` → 3 passed (frame round-trip, media/HLS extraction, ignore-non-media). The MV3
      manifests validate as JSON. The Tauri binary (`native_host.rs`) cannot be compiled here (no
      webkit2gtk) — verify with `tauri build` on a webview host; the registration manifest's `path`
      and `allowed_origins` must be filled in per OS/extension-id.
- [x] **M7 — Polish + ship**: live **uplot speed graph** (`src/lib/SpeedGraph.svelte`) sampling
      aggregate speed each second, rendered into a dedicated card in `App.svelte`; completion
      notification via `tauri-plugin-notification` (`notify_on_complete` command + capability +
      plugin wired in `main.rs`); system-tray already in place (M4). **[DONE:7]** Autostart
      (`tauri-plugin-autostart`) was wired after M7: plugin + capability permissions + a "Launch at
      login" toggle in `App.svelte`, and the checksum-entry / category-management panels were added. Frontend
      build-verifiable: `npm run check` → 0 errors/0 warnings, `npm run build` → BUILD_EXIT=0 (uplot
      bundled). The Rust-side notification wiring is written but unverifiable in this sandbox
      (no webkit2gtk). Remaining host-only steps: run `tauri build` per OS
      (Linux/macOS/Windows) for end-to-end smoke; the `com.app.dm.native.json` `path`/`allowed_origins`
      must still be filled per OS/extension-id.

## Verification

- **Headless engine tests** (`cargo test`): range math, chunk offset writes, limiter rate, resume
  offset planning, single-connection fallback decision.
- **Local server integration**: `python -m http.server` over a multi-MB file (supports Range) and a
  non-range server (fallback) — assert byte-exact output and resume correctness.
- **`tauri dev`** manual: add URL → observe N parallel connections + speed graph; pause/resume
  mid-download; restart app → resume from SQLite; enforce global speed cap; clipboard capture; tray;
  timed scheduler; browser-extension capture.
- **`tauri build`**: produce platform bundles; smoke-test on each target OS.

## Open decisions / risks

- **Frontend framework**: plan assumes **Svelte + Vite**. Tell me if you'd prefer React/Vue/vanilla —
  it only changes the `src/` frontend, not the Rust backend.
- **Browser video grabber** is heuristic (page `<video>`/HLS detection); full site-specific grabbing
  is out of scope for v1.
- **Tauri plugin names** (tray/notification/autostart) may shift between v2 minor versions — pin
  exact versions at M0.
- **Native messaging host** needs per-OS manifest registration (registry on Windows, JSON on macOS/Linux);
  plan ships a small companion binary rather than reusing the main app for stdio.
