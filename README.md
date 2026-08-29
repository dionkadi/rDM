# DM — a cross-platform download manager (IDM equivalent)

**DM** is a desktop download manager that accelerates downloads by splitting a file into multiple
parallel HTTP `Range` connections (segmented downloading), with pause/resume, queueing, speed
limiting, categories, a live speed graph, checksum verification, proxy support, clipboard capture,
drag-and-drop, a system tray, a timed scheduler, and browser extensions with a video grabber.

It is built with **[Tauri 2](https://v2.tauri.app/)** (Rust backend + Svelte/Vite **web UI**) so a
single codebase targets Linux, macOS, and Windows.

---

## Features

| Feature | Notes |
| --- | --- |
| **Segmented downloads** | Splits a file into parallel `Range` GETs; falls back to a single connection when the server lacks `Accept-Ranges`. |
| **Pause / resume** | Resume offset computed from completed chunk bytes; persists across app restarts via SQLite. |
| **Queueing + concurrency** | Global max concurrent downloads and per-download connection count (`tokio::sync::Semaphore`). |
| **Speed limiting** | Token-bucket limiter for global and per-download caps. |
| **Categories** | Downloads can be tagged with a category; auto-routed storage path by extension. |
| **Live speed graph** | `uplot` chart in the UI, sampling aggregate in-flight speed once per second. |
| **Checksum verification** | Streamed `sha2` hash; verifies against an optional provided hash. |
| **Proxy support** | Global or per-download HTTP/HTTPS/SOCKS proxy via `reqwest`. |
| **Clipboard capture** | Monitors the clipboard for URLs and offers to add them. |
| **Drag & drop** | Drop `text/uri-list` / `text/plain` URLs onto the window to enqueue. |
| **System tray** | Minimize/close-to-tray with a tray menu. |
| **Scheduler** | Timed start/stop window; downloads outside the window are parked as `Scheduled`. |
| **Browser extensions + video grabber** | MV3 extension + a native-messaging host binary that captures page media URLs (`<video>`/`<source>`/`.m3u8`/DASH) and forwards them to the app. |

---

## Architecture

```
 Browser / Clipboard / Tray  ──►  Svelte + Vite web UI (Tauri webview)
                                       │  invoke()  /  listen("download-event")
                                       ▼
                          Tauri backend (Rust, tokio)  —  DownloadManager (managed state)
   ├─ commands.rs     add / pause / resume / cancel / set_global_speed_limit / list / settings
   ├─ events.rs       emit("download-event"): added|progress|statusChanged|completed|error|removed
   ├─ tray.rs         system tray + close/minimize-to-tray
   ├─ native_host.rs  localhost TCP listener (port 9157) for the browser-extension host
   └─ (wires engine scheduler loop + notification plugin)
                                       │
                       crates/engine   (zero Tauri dependency — headless-testable)
   ├─ manager.rs   task map + command channel + schedule loop
   ├─ task.rs      per-file state machine + resume-offset plan
   ├─ chunk.rs     one Range GET → seek-write at file offset
   ├─ scheduler.rs semaphore: global + per-download concurrency
   ├─ limiter.rs   token bucket: global + per-download speed cap
   ├─ protocol.rs  HEAD probe, Accept-Ranges/length, range negotiation
   ├─ storage.rs   rusqlite (bundled): downloads, chunks, history, categories, settings
   ├─ config.rs    settings + categories (serde + directories)
   └─ model.rs     Download / Chunk / Status / Settings (serde, camelCase)

   crates/native-host  —  standalone binary (no Tauri/webview): native-messaging frames
                          ↔ localhost socket (port 9157) to the running app
```

The **engine** (`crates/engine`) has **no Tauri dependency**, so it can be unit- and
integration-tested headlessly. The **native-messaging host** (`crates/native-host`) is also a
standalone binary (no Tauri/webview), so it compiles and tests even where the webview toolchain is
unavailable.

---

## Repository layout

```
DM/
  Cargo.toml                     # workspace: members = [crates/engine, crates/native-host, src-tauri]
  crates/
    engine/                     # core download engine (no Tauri dep) — lib + tests/integration.rs
    native-host/                # Chrome native-messaging host binary (stdin/stdout ↔ localhost)
  src-tauri/                    # Tauri v2 app: Cargo.toml, tauri.conf.json, build.rs,
                                #   capabilities/default.json, icons/, src/{main,commands,events,
                                #   tray,native_host}.rs
  browser-extension/            # MV3 extension: manifest.json, background.js, content.js,
                                #   popup.html, popup.js, com.app.dm.native.json (registration template)
  src/                          # frontend: main.ts, App.svelte, app.css,
                                #   lib/{types.ts, api.ts, SpeedGraph.svelte}, vite-env.d.ts
  index.html  package.json  vite.config.ts  svelte.config.js  tsconfig*.json
  PLAN.md                       # approved build plan (milestones M0–M7)
```

---

## Prerequisites

- **Rust** toolchain (edition 2021) — `cargo`, `rustc`.
- **Node.js** 18+ and **npm**.
- **Tauri 2 CLI**: `npm install -g @tauri-apps/cli` (or use `npm run tauri ...`).
- **Platform webview dependencies** (required only for `tauri dev` / `tauri build`):
  - Linux: `webkit2gtk-4.1`, `libsoup-3.0`, `libjavascriptcoregtk`, `build-essential`, `pkg-config`, `librsvg2-dev`, etc.
  - macOS: Xcode Command Line Tools.
  - Windows: WebView2 (preinstalled on Win11; the Tauri VS build tools otherwise).

> The sandbox used to develop this repo did **not** have the webview toolchain, so the Tauri binary
> (`tauri dev` / `tauri build`) could not be executed there. The frontend, engine, and native-host
> were still fully build- and test-verified (see below).

---

## Build & run

```bash
# 1. Frontend dependencies
npm install

# 2. Frontend dev server (Vite) — for UI work without the webview
npm run dev

# 3. Full desktop app (requires webview deps on the host)
npm run tauri dev        # dev with hot-reload
npm run tauri build      # produce a platform bundle

# Frontend type-check / production build (no webview needed)
npm run check            # svelte-check: 0 errors / 0 warnings
npm run build            # production build into dist/

# Engine + native-host tests (no webview needed)
cargo test -p dm-engine        # 19 unit + 3 integration tests
cargo test -p dm-native-host   # 3 tests (native-messaging frame + media extraction)
```

### Browser extension + video grabber

1. Build the native host binary: `cargo build -p dm-native-host` (release: `cargo build -p dm-native-host --release`).
2. Load `browser-extension/` as an unpacked **MV3** extension (Chrome/Edge: `chrome://extensions` →
   Developer mode → Load unpacked; Firefox: temporary add-on).
3. Register native messaging. Fill in `browser-extension/com.app.dm.native.json`:
   - `path` → absolute path to the `dm-native-host` binary.
   - `allowed_origins` → `chrome-extension://<YOUR_EXTENSION_ID>/` (copy the ID from `chrome://extensions`).
   Install that manifest to the OS location for `com.app.dm.native`:
   - Linux: `~/.config/google-chrome/NativeMessagingHosts/` (or the Chromium/Brave equivalent).
   - macOS: `~/Library/Application Support/Google/Chrome/NativeMessagingHosts/`.
   - Windows: registry `HKCU\Software\Google\Chrome\NativeMessagingHosts\com.app.dm.native` → path to JSON.
4. Start DM. The extension captures media URLs on pages and forwards them to the app over
   `127.0.0.1:9157`; the app-side listener (`src-tauri/src/native_host.rs`) enqueues them like a
   normal `add_download`.

> The video grabber is **heuristic** (detects page `<video>`/`<source>` elements and `.m3u8`/`.mpd`
> links). Site-specific HLS/DASH manifest resolution is out of scope for v1.

---

## Verification status

| Check | Command | Result |
| --- | --- | --- |
| Engine unit + integration tests | `cargo test -p dm-engine` | ✅ 22 passed |
| Native-messaging host tests | `cargo test -p dm-native-host` | ✅ 3 passed |
| Frontend type-check | `npm run check` | ✅ 0 errors / 0 warnings |
| Frontend production build | `npm run build` | ✅ succeeds (uplot bundled) |
| Tauri desktop app (`tauri dev`/`build`) | `npm run tauri dev` | ⚠️ requires webview host (not run in sandbox) |

---

## Roadmap / known gaps

- **Autostart plugin** (`tauri-plugin-autostart`) is listed in the plan but not yet enabled.
- **End-to-end smoke test** with `tauri build` per OS (Linux/macOS/Windows) — run on a webview host.
- **Browser-extension packaging** (zip/crx) and per-OS native-messaging registration docs.
- Richer category management / checksum-entry / proxy-entry panels in the UI (the engine already
  supports checksum + proxy; the UI panels are simplified).

---

## License

TBD. (Specify a license before distributing.)
