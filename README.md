# DM — Download Manager

A cross-platform, IDM-style download manager built with **Tauri 2 (Rust + WebView)** and a **Svelte 4 + TypeScript** SPA frontend. Resumable, segmented, rate-limited downloads with a native browser-extension bridge.

## Highlights

- **Segmented / resumable downloads** — HTTP Range requests, up to N connections per file, automatic resume across restarts.
- **Global + per-download speed caps** — token-bucket rate limiting (more-restrictive bucket wins).
- **Schedule window** — park downloads outside the configured time range; auto-promote when it opens.
- **Categories** — auto-route by extension (Videos, Music, Documents, …) into per-category folders.
- **Browser integration** — Chromium MV3 extension grabs media URLs from pages and forwards them to DM.
- **Native messaging shim** — separate `dm-native-host` binary, no Tauri runtime required.
- **System tray** with show / hide / quit; clipboard URL monitor; drag-and-drop.
- **SQLite persistence** (bundled, no system dep) of downloads, history, categories, settings.
- **Cross-platform** — Linux, macOS, Windows. A 23 MB stripped Linux binary linking to `webkit2gtk-4.1`.

## Architecture at a glance

```
                  ┌─────────────────────┐
   Browser ─────► │  MV3 extension      │
   (media URLs)   │  (background.js)    │
                  └──────────┬──────────┘
                             │ chrome.runtime.connectNative
                             ▼
                  ┌─────────────────────┐
                  │ dm-native-host bin  │  stdio ↔ 4-byte-LE frames
                  │ (extract_media_urls │
                  │  → forward_url)     │
                  └──────────┬──────────┘
                             │ TCP 127.0.0.1:9157  (newline-JSON)
                             ▼
   ┌──────────────────────────────────────────────┐
   │ src-tauri (Tauri 2 shell)                    │
   │   • commands.rs → DownloadManager            │
   │   • events.rs   → "download-event" channel   │
   │   • tray.rs     → system tray (best-effort)  │
   └──────────────────────┬───────────────────────┘
                          │ uses
                          ▼
   ┌──────────────────────────────────────────────┐
   │ crates/engine (UI-agnostic, no Tauri)        │
   │  DownloadManager / Scheduler / Limiter /     │
   │  Control / Protocol / Chunk / Task /         │
   │  Storage (rusqlite bundled) / Config         │
   └──────────────────────────────────────────────┘
                          ▲
                          │ Tauri events (camelCase JSON)
                          │
   ┌──────────────────────────────────────────────┐
   │ src/  Svelte 4 + TS + Vite SPA               │
   │   App.svelte → stores → components           │
   │   (lib/api.ts wraps invoke("...") commands)  │
   └──────────────────────────────────────────────┘
```

For a deep dive (engine internals, event shape, build gotchas, conventions), see **[`AGENTS.md`](AGENTS.md)**.

## Quick start

### Prerequisites

- **Rust** stable (1.74+ recommended; edition 2021).
- **Node.js 18+** and **npm**.
- **Tauri 2 system deps** for your platform — see <https://tauri.app/start/prerequisites/>.
- **On Fedora 40+**: nothing extra; the build script handles the missing `libappindicator3` / `ayatana-appindicator3` `.pc` files via a local shim.

### Run the frontend alone (demo mode, no Rust)

```bash
npm install
npm run dev      # http://localhost:5173
```

The UI ships with a **demo mode** that simulates progress locally so you can exercise the layout, filters, command palette, and settings without launching the engine.

### Run the full app

```bash
# Linux (Fedora 40+ and similar)
./build-with-shim.sh

# Stock Debian/Ubuntu or macOS/Windows
npm run tauri dev
```

The first build is slow (compiles Tauri's webview bindings + the engine + the native host). Subsequent builds are incremental.

### Run tests

```bash
cargo test -p dm-engine   # 22 unit + integration tests
```

The integration suite spins up an in-process HTTP/1.1 server that supports `Range` requests and runs the real `DownloadManager` end-to-end.

### Build a release binary

```bash
./build-with-shim.sh                   # Linux
# or: npm run tauri build              # Other platforms

ls target/release/dm-tauri             # 23 MB stripped
```

The Cargo workspace at the repo root unifies `target/`, so the binary lives at **`target/release/dm-tauri`** (not `src-tauri/target/...`).

## Releasing

Tagged commits ship ready-to-install binaries for all three OSes plus
the browser extension, attached to a GitHub Release. The full pipeline
is in [`.github/workflows/release.yml`](.github/workflows/release.yml).

### Cut a release

```bash
# 1. Bump versions in lockstep, commit, and create an annotated tag.
./scripts/release.sh 0.2.0
# → edits package.json, tauri.conf.json, every Cargo.toml, manifest.json
# → regenerates Cargo.lock
# → commits the bump and creates tag v0.2.0

# 2. Push the tag — the release workflow picks it up.
git push origin main --follow-tags
```

The workflow runs three desktop jobs in parallel (Linux `.deb`/`.AppImage`,
Windows `.msi`/`.exe`, macOS `.dmg`/`.app`) and a fourth job that
packages the extension as both `dm-grabber-<version>.zip` (Chrome /
Edge / Brave / Arc) and `dm-grabber-<version>.xpi` (Firefox). All
artifacts are attached to a single GitHub Release; the release is
created as a **draft** so you can edit notes before publishing.

Tagging `v0.2.0-rc.1` produces a **pre-release** instead of a stable
release — the install scripts and URLs work the same way, but the
GitHub UI marks it yellow and excludes it from `/releases/latest`.

### macOS: Gatekeeper workaround

The current `tauri.conf.json` builds **ad-hoc signed** (no Developer
ID). On first launch macOS will refuse to open the app with:

> "DM" is damaged and can't be opened. You should move it to the Trash.

The fix is one right-click in Finder:

1. Open Finder, navigate to Applications.
2. **Right-click** `DM.app` → **Open**.
3. Click **Open** in the "unidentified developer" dialog.
4. From now on, double-click works normally.

To get rid of the warning permanently, you need an Apple Developer ID
($99/yr) and the secrets listed in
[`.github/workflows/release.yml`](.github/workflows/release.yml) (`APPLE_CERTIFICATE`,
`APPLE_ID`, etc.). When those are set, the workflow passes
`signingIdentity` to `tauri-action` and Apple notarizes the .dmg.

### Pre-flight CI

Every push to `main` and every PR runs
[`.github/workflows/ci.yml`](.github/workflows/ci.yml) (`cargo check` +
`svelte-check` + extension manifest/syntax checks). Tagging a broken
main is impossible without that CI run going red first.

### Local packaging (extension only)

```bash
# Linux / macOS
./browser-extension/scripts/package.sh 0.2.0

# Windows
pwsh ./browser-extension/scripts/package.ps1 -Version 0.2.0
```

Output: `browser-extension/dist/dm-grabber-<version>.zip` and
`dm-grabber-<version>.xpi`.

## Browser extension

`browser-extension/` is a Manifest V3 Chromium extension with:

- a content script that scrapes `<video>`, `<source>`, `<audio>`, and `<a>` tags for media URLs plus HLS (`*.m3u8`) and DASH (`manifest`) hints,
- a service worker that connects to the `com.app.dm.native` host and forwards URLs,
- a popup with **Grab page media** and **Open DM** buttons.

To install:

1. Build the native host: `cargo build --release -p dm-native-host`.
2. Edit `browser-extension/com.app.dm.native.json` to point `"path"` at the absolute path of the produced binary, and replace `REPLACE_WITH_EXTENSION_ID` with the extension's id once loaded into Chrome.
3. Load the extension unpacked from `browser-extension/`.

See the [native messaging docs](https://developer.chrome.com/docs/apps/nativeMessaging/) for the per-platform manifest install location.

## Project layout

```
DM/
├── AGENTS.md                  ← agent/human working guide
├── UI_DESIGN_PLAN.md          ← IDM-parity UX target (read before UX work)
├── Cargo.toml                 ← Cargo workspace
├── package.json               ← Svelte + Vite frontend
├── build-with-shim.sh         ← Tauri build wrapper (Fedora pkg-config shim)
├── src/                       ← Svelte frontend (App.svelte, lib/{api,types,stores,…})
├── src-tauri/                 ← Tauri shell (commands, events, tray, native-host listener)
├── crates/engine/             ← UI-agnostic core: manager, scheduler, limiter, chunk, storage…
├── crates/native-host/        ← stdio native-messaging shim
├── browser-extension/         ← MV3 Chromium extension
├── .pkgconfig-shim/           ← Fedora 40+ libappindicator .pc shims
└── notes/session-logs/        ← Free-form per-session notes
```

## Contributing

1. Read [`AGENTS.md`](AGENTS.md) for architecture, conventions, and gotchas.
2. UX work: read [`UI_DESIGN_PLAN.md`](UI_DESIGN_PLAN.md) first.
3. Engine changes: update the integration test in `crates/engine/tests/integration.rs` and mirror any new fields end-to-end (Rust model → storage → Tauri events → TS types → UI).
4. Run `npm run check` (must be 0 errors / 0 warnings) and `cargo test -p dm-engine` before pushing.

## License

No license file is present yet. Until one is added, treat this repository as **all rights reserved** by its authors.
