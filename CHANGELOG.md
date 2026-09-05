# Changelog

All notable changes to DM will be documented in this file.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** Versions before 1.0.0 (i.e. `0.x.y`) are pre-1.0 and may
> ship breaking changes between minor versions. Once we hit 1.0.0 we
> commit to the SemVer stability guarantees.

## [Unreleased]

## [0.4.2] — 2026-09-05

A focused patch release that fixes three regressions introduced
by 0.4.1's WebSocket direct-transport work.

### Fixed

- **Chrome silently rejected the browser extension** ([#4]).
  The 0.4.1 manifest used `background.scripts`, which Chrome's
  MV3 parser strictly rejects with `'background.scripts'
  requires manifest version of 2 or lower`. The extension
  failed to install on every Chromium browser. The
  `scripts/package.sh` and `scripts/package.ps1` now produce
  **two distinct manifests** at packaging time: the `.zip`
  ships `background.service_worker: "background.js"` (Chrome /
  Edge / Brave / Arc / Vivaldi / Opera), the `.xpi` keeps the
  source's `background.scripts` + `browser_specific_settings.
  gecko.id` (Firefox 109+). A new self-verify step in the
  packaging script fails loudly if either archive ends up
  with the wrong shape, so the regression can't recur silently.
  The source `manifest.json` keeps the Firefox shape so devs
  can sideload on Firefox without running the package script.

- **"Open DM app" button showed an unclosable alert** ([#4]).
  The previous version called `alert(...)` from the popup,
  which in MV3 is a synchronous, unclosable native dialog
  that takes focus from the popup and (in some Chromium
  versions) can't be dismissed without killing the popup or
  the tab. The button is now repurposed as a **"Re-check
  host"** control: when the host is down, clicking it
  re-issues the status probe (the next 1.5 s tick picks up
  the result and re-renders the status pill); when the host
  is up, the button is disabled with a `title` explaining
  that DM is already running and the extension can't launch
  a Tauri app from a browser.

- **Settings → About and Extensions were stale** ([#4]).
  The About panel hard-coded `Version: 0.1.0` and
  `Engine: dm-engine 0.1.0` since the very first commit.
  New Rust `app_info` Tauri command returns
  `{appVersion, engineVersion, tauriVersion}` from
  `env!("CARGO_PKG_VERSION")`, `dm_engine::VERSION` (a new
  `pub const` mirroring `crates/engine/Cargo.toml`), and
  `tauri::VERSION`. Wired through `src/lib/api.ts` and
  rendered in the About tab (with `—` placeholders if the
  call fails, so the user sees an honest empty state
  instead of a stale `0.1.0`). Also fixed the misleading
  `Frontend: Svelte 5` line (the project is on Svelte 4).
  The Extensions tab is rewritten with the new 3-step
  WebSocket flow for Chromium and a trimmed 3-step Firefox
  flow.

### Drive-by

- `release.yml`: the long `python3 -c "..."` one-liner in the
  `Verify packages` step is now a heredoc, and the two long
  `files:` entries are wrapped to fit the project's 80-col
  convention. Same behaviour, no functional change.

## [0.4.1] — 2026-09-05

A focused patch release that fixes the browser-extension install
flow and the popup's first-open error flash. The big change is
that **Chromium-based browsers (Chrome, Edge, Brave, Arc, Vivaldi,
Opera) no longer need a native-messaging host at all** — the
extension talks to the running DM app directly over WebSocket on
`ws://127.0.0.1:9157/`. Firefox users still ship the small
`dm-native-host` binary because Firefox MV3 cannot reliably open
`ws://127.0.0.1` connections (Firefox upgrades insecure `ws://`
to `wss://` and the connection fails silently).

### Changed

- **Browser-extension install is now 3 steps for Chromium**
  (was 6+):
  1. Install the DM desktop app and launch it once
  2. Download `dm-grabber-<version>.zip` from the GitHub release
  3. `chrome://extensions` (or `brave://extensions`, etc.) →
     Developer mode → **Load unpacked** → pick the unzipped folder
  No host binary, no manifest copy, no extension-ID paste, no
  Windows-registry edits, no per-browser config directory. The
  Tauri app already binds `127.0.0.1:9157`; the extension
  piggybacks on the same socket.
- **Linux release artifacts now include `.rpm`** in addition to
  `.deb` and `.AppImage`. The release workflow's install table
  lists `sudo dnf install -y ./DM-*_amd64.rpm` for RPM-based
  distros (Fedora, RHEL, openSUSE, etc.).

### Added

- **Direct WebSocket transport for the browser extension**
  ([#3]). The Tauri native-host listener on `127.0.0.1:9157` is
  now a dual-protocol server: connections that start with `GET
  / HTTP/1.1` go through a hand-rolled RFC 6455 WebSocket
  handshake; everything else falls through to the existing
  line-delimited JSON path (used by the Firefox native-messaging
  host). Both paths emit the same `Captured` frontend event with
  the same wire payload (`{"url":"…","type":"…","referer":"…",
  "userAgent":"…"}`).
  - `src-tauri/src/ws.rs` (new, ~570 LoC) — handshake,
    text-frame read/write, close-frame, no new deps beyond
    `sha1` and `base64` (already in the workspace). 9 unit tests
    - 1 real-listener round-trip test.
  - `src-tauri/src/native_host.rs` — peeks the first byte of
    each accept to dispatch to the right reader.
  - `browser-extension/background.js` — new
    `makeWebSocketTransport()` with persistent connection,
    exponential-backoff reconnect (200ms / 500ms / 1s / 2s / 5s),
    and a 64-entry send queue so a click during a brief
    disconnect is captured rather than dropped. The old
    `makeNativeTransport()` is kept verbatim for Firefox. Picks
    the right one at module load via `typeof browser?.runtime?.
    getBrowserInfo === "function"`.
  - `browser-extension/manifest.json` — drop `nativeMessaging`
    from `permissions`, add `ws://127.0.0.1:9157/*` to
    `host_permissions`.
  - `hostState.transportKind` (`"websocket"` | `"native"`) is
    surfaced to the popup so the user can see which path is
    in use.

### Fixed

- **MV3 service-worker cold-start race in the popup**
  ([#2], [#1]). The popup's status probe used to call
  `chrome.runtime.sendMessage` immediately on open, racing the
  service worker registering its `onMessage` listener. The
  browser threw `Could not establish connection. Receiving
  end does not exist.` — a cold-start blip, not a real
  failure — and the popup surfaced it as a permanent red error.
  The probe now retries with exponential backoff (80 / 200 /
  400 ms; total ~680 ms) and only shows the error after the
  budget is exhausted. Non-cold-start errors (real host-lookup
  failures) are surfaced immediately with no retry. A
  `probeInFlight` guard prevents overlapping `setInterval`
  ticks from piling up retries.

### Removed

- `browser-extension/com.app.dm.native.chrome.json` and
  `browser-extension/com.app.dm.native.json` (the cross-browser
  "shared" template) — both are orphans now that Chromium
  browsers use WebSocket. Only the Firefox-specific template
  (`com.app.dm.native.firefox.json`) remains.

### Security

- **Attack surface change (improvement)**. The TCP listener on
  `127.0.0.1:9157` was previously open to any local process;
  the WebSocket listener is gated by the extension's
  `host_permissions` so only the extension's background script
  can connect. Page-context WebSockets are still subject to the
  page's own CSP and cannot reach `127.0.0.1:9157` directly.
  The Tauri confirmation dialog (`CaptureDialog.svelte`)
  remains the user-facing gate that prevents auto-download of
  any URL the extension forwards.

## [0.4.0] — 2026-09-04

File-system features (trash, open, copy path) + per-download
auth/cookie import. See `git log 2bc6b1a` for the full list of
commits.

## [0.3.0]

Top-5 maturity features: bulk select, drag-to-reorder, auth/headers,
HLS/DASH marker, mirror failover. See `git log 9399e1c`.

## [0.2.1]

Bug-fix patch. See `git log 87fd638`.

## [0.2.0]

Initial cross-platform release pipeline + extension packaging. See
`git log 1a957fd`.

## [0.1.0] — initial release

First public version of DM, including:

- Segmented / resumable downloads via HTTP `Range`, with global
  and per-download rate limiting.
- Categories, schedule window, system tray, clipboard URL monitor,
  drag-and-drop.
- SQLite persistence of downloads, history, categories, and
  settings.
- Chromium MV3 browser extension that intercepts media /
  download URLs and forwards them via the `dm-native-host` shim
  to the running app on `127.0.0.1:9157`.

See `git log` for the full list of commits.
