# AGENTS.md — working on the DM download manager

This file is for AI coding agents (and human contributors) touching this repo. It records the
conventions, layout, and the exact verification commands that work in this environment.

## What this is

A cross-platform **IDM-style download manager**: Tauri 2 (Rust backend) + Svelte/Vite **web UI**.
The Rust core splits files into parallel HTTP `Range` connections, with pause/resume, queueing,
speed limiting, categories, checksum verification, proxy support, clipboard capture, drag-drop,
system tray, a timed scheduler, and a browser extension + video grabber. See `README.md` and the
approved `PLAN.md` (milestones M0–M7, all done).

## Repository map (where things live)

| Path | Role |
| --- | --- |
| `crates/engine/` | **Core engine. NO Tauri dependency** — keep it that way (headless-testable). `lib.rs` re-exports; `model.rs` is the shared data model; `protocol.rs` (HEAD/range probe), `chunk.rs` (Range GET→seek-write), `task.rs` (state machine + resume plan), `manager.rs` (command channel + schedule loop), `scheduler.rs` (semaphore concurrency), `limiter.rs` (token bucket), `storage.rs` (`rusqlite`), `config.rs` (settings/categories), `control.rs`. |
| `crates/engine/tests/integration.rs` | Integration tests against a local Range server (segmented, single-conn fallback, checksum mismatch). |
| `crates/native-host/` | Standalone **native-messaging host binary** (no Tauri/webview) — Chrome native-messaging frames on stdin/stdout ↔ localhost socket. `DEFAULT_PORT` = 9157. |
| `src-tauri/` | Tauri v2 app. `main.rs` (builder, plugins, `manage(manager)`, tray, schedule loop, native-host listener), `commands.rs` (`#[tauri::command]`), `events.rs` (event consts), `tray.rs`, `native_host.rs` (localhost listener, `DEFAULT_PORT` = 9157). `capabilities/default.json` gates permissions. `icons/` is already generated. |
| `browser-extension/` | MV3 extension: `manifest.json`, `background.js` (connects native host, forwards media), `content.js` (page media scan), `popup.html`/`popup.js`, `com.app.dm.native.json` (registration template — fill `path` + `allowed_origins`). |
| `src/` | Frontend. `App.svelte` (UI + state + speed monitor + event listener), `lib/api.ts` (Tauri `invoke` wrappers), `lib/types.ts` (TS types mirroring the Rust model), `lib/SpeedGraph.svelte` (uplot chart), `main.ts`, `app.css`, `vite-env.d.ts`. |

## Conventions (follow these)

- **JSON serialization is camelCase.** Both the Rust `model.rs` and the `FrontendEvent` struct use
  `#[serde(rename_all = "camelCase")]`. Keep frontend `lib/types.ts` field names in camelCase to match.
- **Engine must stay Tauri-free.** Do not `use tauri::…` inside `crates/engine`. UI/process concerns
  belong in `src-tauri` or the frontend.
- **Live updates** flow over the Tauri event channel `"download-event"` carrying a `FrontendEvent`
  with `kind` ∈ `added|progress|statusChanged|completed|error|removed` and a `download` payload.
  Frontend subscribes via `@tauri-apps/api/event` `listen("download-event", …)`.
- **Native-messaging port `9157`** is shared between `crates/native-host/src/main.rs` and
  `src-tauri/src/native_host.rs`. Keep both `DEFAULT_PORT` in sync if you change it.
- **HTTP/TLS**: `reqwest` uses `rustls-tls` (no OpenSSL). **SQLite** uses `rusqlite` `bundled` (no
  system SQLite). Don't switch these without cause.
- **Commands**: add a `#[tauri::command]` in `commands.rs`, wrap it with `app.handle()` where needed,
  register it in `generate_handler!([…])` in `main.rs`, and (if browser-facing) add a matching
  capability permission in `capabilities/default.json`. Mirror it with a typed wrapper in `lib/api.ts`.

## Verification commands that work HERE (no webview needed)

```bash
cargo test -p dm-engine        # 22 passed (19 unit + 3 integration)
cargo test -p dm-native-host   # 3 passed
npm run check                  # svelte-check: 0 errors / 0 warnings
npm run build                  # production frontend build (EXIT 0; uplot bundled)
```

## Commands that need a WEBVIEW HOST (cannot run in this sandbox)

These require `webkit2gtk` (Linux) / platform webview deps, which are absent here:

```bash
npm run tauri dev
npm run tauri build
cargo build -p dm-tauri        # the Tauri binary (exercises tray.rs / native_host.rs / notification)
```

The **Rust source** for the Tauri side is written and correct-by-inspection, but it is **not
compile-verified** in this environment. Don't assume `src-tauri/src/*.rs` type-checks until built on
a webview host.

## Gotchas / known false positives

- **pi-lens "No Svelte configuration found in vite.config" / "Cannot find module '@tauri-apps/api/event'"
  / "Parameter 'ev' implicitly has an 'any' type"** are **stale cached diagnostics**. They are
  disproven: `npm run check` reports 0 errors/0 warnings and `npm run build` succeeds (the same
  vite+svelte config is used). Do not "fix" the Svelte/Vite config — it is correct
  (`svelte.config.js` exists and `vite.config.ts` registers `svelte()`).
- **Icons** (`src-tauri/icons/*`) are already present; you do NOT need to run `tauri icon` unless you
  replace the artwork.
- **`tauri-plugin-autostart`** is planned but not yet wired. Scheduler (`chrono` timers) and
  notification (`tauri-plugin-notification`) are wired.
- The **video grabber** is heuristic (page `<video>`/`<source>`/`.m3u8`/`.mpd`); site-specific
  HLS/DASH resolution is out of scope for v1.

## Quick "where do I add X?"

- New download feature/option → `crates/engine` (model + logic), then expose via `src-tauri/commands.rs`,
  then call from `src/lib/api.ts`, then render in `src/App.svelte`.
- New browser-extension behavior → `browser-extension/background.js` + `content.js`; new URL source →
  also handled by `crates/native-host/src/main.rs::extract_media_urls`.
- New UI panel → `src/App.svelte` + a component under `src/lib/`.
- New persisted setting → `crates/engine/src/model.rs` (`Settings`) + `src-tauri/src/commands.rs`
  (`get_settings`/`update_settings`) + `App.svelte` settings block.
