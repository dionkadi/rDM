# DM — Download Manager

A cross-platform, IDM-style download manager built with **Tauri 2 (Rust + WebView)** and a **Svelte 4 + TypeScript** SPA frontend. Resumable, segmented, rate-limited downloads with a native browser-extension bridge.

## Highlights

- **Segmented / resumable downloads** — HTTP Range requests, up to N connections per file, automatic resume across restarts.
- **Global + per-download speed caps** — token-bucket rate limiting (more-restrictive bucket wins).
- **Schedule window** — park downloads outside the configured time range; auto-promote when it opens.
- **Categories** — auto-route by extension (Videos, Music, Documents, …) into per-category folders.
- **Browser integration** — MV3 extension grabs media URLs from pages and hands them to DM.
- **Native messaging shim** — separate `dm-native-host` binary, no Tauri runtime required.
- **System tray** with show / hide / quit; clipboard URL monitor; drag-and-drop.
- **SQLite persistence** (bundled, no system dep) of downloads, history, categories, settings.
- **Cross-platform** — Linux, macOS, Windows. A ~23 MB stripped Linux binary linking to `webkit2gtk-4.1`.

## Documentation map

| Document | For | Contents |
|---|---|---|
| **README.md** (this) | users & evaluators | what DM is, install, quick start, usage, releasing |
| **[AGENTS.md](AGENTS.md)** | contributors & coding agents | exact commands, conventions, invariants, gotchas, verification |
| **[docs/UI_DESIGN_PLAN.md](docs/UI_DESIGN_PLAN.md)** | UX work | visual system and information-architecture target |
| **[CHANGELOG.md](CHANGELOG.md)** | everyone | version history and bug narratives |
| **[TODO.md](TODO.md)** | contributors | roadmap by maturity tier |

Boundary rule: a fact has one canonical home — user-facing prose (features, install, usage, releasing) lives here; working-contract material (commands, invariants, gotchas) lives in AGENTS.md. Commands may appear in both, verbatim in AGENTS.md and narrative here.

## Architecture at a glance

```
                  ┌─────────────────────┐
   Browser ─────► │  MV3 extension      │
   (media URLs)   │  (background.js)    │
                  └──────────┬──────────┘
                             │ Chromium: ws://127.0.0.1:9157
                             │ Firefox:  connectNative (stdio)
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
   │  Control / Protocol / Chunk / HLS / Media /  │
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

Engine internals, event shapes, and invariants live in [`AGENTS.md`](AGENTS.md).

## Quick start

### Prerequisites

- **Rust** stable (1.74+ recommended; edition 2021).
- **Node.js 18+** and **npm**.
- **Tauri 2 system deps** for your platform — see <https://tauri.app/start/prerequisites/>.
- **On Fedora 40+**: nothing extra; `build-with-shim.sh` handles the missing `libappindicator3` / `ayatana-appindicator3` `.pc` files via a local shim.

### Run the frontend alone (demo mode, no Rust)

```bash
npm install
npm run dev      # http://localhost:5173
```

Demo mode simulates progress locally so you can exercise the layout, filters, command palette, and settings without the engine.

### Run the full app

```bash
# Linux (Fedora 40+ and similar)
./build-with-shim.sh

# Stock Debian/Ubuntu or macOS/Windows
npm run tauri dev
```

The first build is slow (Tauri webview bindings + engine + native host); later builds are incremental.

### Run tests

```bash
cargo test -p dm-engine        # engine unit + integration + HLS suites
npm run check                  # svelte-check (0 errors / 0 warnings)
cd browser-extension && node --test test/*.test.mjs
```

The engine integration suite spins up an in-process HTTP/1.1 server with `Range` support and drives the real `DownloadManager` end-to-end.

### Build a release binary

```bash
./build-with-shim.sh                   # Linux
# or: npm run tauri build              # Other platforms

ls target/release/dm-tauri             # ~23 MB stripped
```

The Cargo workspace at the repo root unifies `target/`, so the binary lives at **`target/release/dm-tauri`**.

## Releasing

Tagged commits ship ready-to-install binaries for all three OSes plus the browser extension, attached to a GitHub Release (pipeline: [`.github/workflows/release.yml`](.github/workflows/release.yml)).

```bash
# 1. Bump versions in lockstep, commit, create an annotated tag.
./scripts/release.sh 0.2.0
# → edits package.json, tauri.conf.json, every Cargo.toml, manifest.json
# → regenerates Cargo.lock, commits the bump, creates tag v0.2.0
#   (--no-tag leaves the commit/tag to you)

# 2. Push — the release workflow picks up the tag.
git push origin main --follow-tags
```

- The GitHub release body is generated by `scripts/changelog-section.mjs <tag>` from the matching `## [x.y.z]` section of `CHANGELOG.md` (or `RELEASE_NOTES.md`) — **a tag whose version has no changelog section fails the release**, by design.
- Tagging `v0.2.0-rc.1` produces a **pre-release** (same artifacts, yellow in the GitHub UI, excluded from `/releases/latest`).
- The workflow runs three desktop jobs (Linux `.deb`/`.rpm`/`.AppImage`, Windows `.msi`/`.exe`, macOS `.dmg`/`.app`) plus extension packaging (`.zip` for Chromium, `.xpi` for Firefox). The release is created as a **draft**.
- The release script self-tests in CI and before every release.

### macOS: Gatekeeper workaround

The current build is **ad-hoc signed** (no Developer ID). On first launch macOS says *"DM" is damaged and can't be opened.* Fix: right-click `DM.app` → **Open** → confirm once; double-click works from then on. Permanent fix needs an Apple Developer ID and the `APPLE_*` secrets in `release.yml` (then `tauri-action` notarizes the `.dmg`).

### Pre-flight CI

Every push to `main` and every PR runs [`.github/workflows/ci.yml`](.github/workflows/ci.yml) (`cargo check` + `svelte-check` + extension manifest/syntax checks + CSS-var lint).

### Local packaging (extension only)

```bash
./browser-extension/scripts/package.sh 0.2.0          # Linux/macOS
pwsh ./browser-extension/scripts/package.ps1 -Version 0.2.0   # Windows
```

Output: `browser-extension/dist/dm-grabber-<version>.zip` (Chromium) and `dm-grabber-<version>.xpi` (Firefox).

## Browser extension

`browser-extension/` is a Manifest V3 extension for Chromium browsers and Firefox 109+:

- a content script that finds media on a page (`<video>`, `<source>`, `<audio>`, `<a>`, HLS `*.m3u8`, DASH `manifest` hints) and, when you click a download-looking link, sends it to DM instead of the browser;
- a service worker that reaches the running app over `ws://127.0.0.1:9157/` (Chromium) or `com.app.dm.native` native messaging (Firefox);
- a popup with **Grab page media**, **Test host connection**, and two switches for what the extension is allowed to take over.

**Install (Chromium — no native binary, no host manifest):** start the DM app, unzip the released `dm-grabber-<version>.zip`, and load it unpacked from `chrome://extensions` → Developer mode → Load unpacked.

> Load the **`.zip`**, not the repo's `browser-extension/` folder: the repo manifest is the Firefox template, and Chrome ignores `background.scripts` under MV3 — the extension would have no background context at all.

**Install (Firefox):** build the host (`cargo build --release -p dm-native-host`), copy `browser-extension/com.app.dm.native.firefox.json` to `~/.mozilla/native-messaging-hosts/com.app.dm.native.json`, and edit its `"path"` to the built binary. Then install the `.xpi`. The extension ID is fixed (`dm-grabber@dm-project`).

Long version: [`browser-extension/INSTALL.md`](browser-extension/INSTALL.md). Invariants the extension must not break: [`AGENTS.md`](AGENTS.md).

## Project layout

```
DM/
├── AGENTS.md                  ← contributor/agent working guide
├── docs/UI_DESIGN_PLAN.md     ← UX target (read before UX work)
├── Cargo.toml                 ← Cargo workspace
├── package.json               ← Svelte + Vite frontend
├── build-with-shim.sh         ← Tauri build wrapper (Fedora pkg-config shim)
├── src/                       ← Svelte frontend (App.svelte, lib/{api,types,stores,…})
├── src-tauri/                 ← Tauri shell (commands, events, tray, native-host listener)
├── crates/engine/             ← UI-agnostic core: manager, scheduler, limiter, chunk, storage…
├── crates/native-host/        ← stdio native-messaging shim
├── browser-extension/         ← MV3 extension (Chromium + Firefox)
├── .pkgconfig-shim/           ← Fedora 40+ libappindicator .pc shims
└── notes/session-logs/        ← free-form per-session notes
```

## Contributing

1. Read [`AGENTS.md`](AGENTS.md) — conventions, invariants, and the exact verification commands.
2. UX work: read [`docs/UI_DESIGN_PLAN.md`](docs/UI_DESIGN_PLAN.md) first.
3. Engine changes: extend the integration tests in `crates/engine/tests/` and mirror any new field end-to-end (Rust model → storage → Tauri events → TS types → UI).
4. Before pushing: `npm run check` (0 errors / 0 warnings), `cargo test -p dm-engine`, and — for extension changes — `node --test browser-extension/test/*.test.mjs`.

## License

No license file is present yet. Until one is added, treat this repository as **all rights reserved** by its authors.
