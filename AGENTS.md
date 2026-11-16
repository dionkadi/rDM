# AGENTS.md — DM Download Manager

Working contract for **AI coding agents and code contributors**. Read this before touching code. Product pitch, install, and usage live in [`README.md`](README.md) — never here.

## Documentation map — what lives where

| File | Audience | Owns |
|---|---|---|
| `README.md` | humans evaluating / using DM | product pitch, install, quick start, usage, releasing, contributing |
| `AGENTS.md` (this) | agents & contributors in the repo | exact commands, conventions, invariants, gotchas, verification |
| `docs/UI_DESIGN_PLAN.md` | anyone touching UX | visual system + information-architecture target |
| `docs/UI_REVIEW.md` | anyone auditing UX | first-round dated issue list, severity/priority, v2.1 change record |
| `docs/UI_REVIEW_ROUND2.md` | anyone auditing UX | second-round issue list (failure paths, keyboard, lifecycle, contrast), reproductions and fix order |
| `docs/CODE_REVIEW.md`, `docs/EXTENSION_AUDIT.md`, `docs/FIX_VERIFICATION.md`, `docs/bench/BENCHMARK_REPORT.md` | anyone auditing code | dated audit / benchmark snapshots — the evidence behind past decisions, **not** the live contract |
| `CHANGELOG.md` / `RELEASE_NOTES.md` | users & maintainers | version history, bug narratives |
| `TODO.md` | contributors | roadmap by maturity tier |

Rules: a fact has **one canonical home**; the others link to it. User-facing prose (feature lists, install steps, screenshots, release instructions) is `README.md` only. Commands may appear in both files — verbatim command lines here, narrative there. Never duplicate a story (e.g. "why 0.4.2 broke X") outside `CHANGELOG.md`.

## Moving parts

Tauri 2 shell (`src-tauri`) → UI-agnostic Rust engine (`crates/engine`) → Svelte 4 + TS + Vite SPA (`src/`), fed by an MV3 browser extension (`browser-extension/`) through `crates/native-host` and a local TCP/WebSocket bridge (`127.0.0.1:9157`). The user-level picture is the diagram in `README.md`.

## Repo map (non-obvious only)

- **Cargo workspace at repo root** unifies `target/` — the release binary is `target/release/dm-tauri`, not `src-tauri/target/...`.
- **`browser-extension/manifest.json` is the Firefox 109+ template.** Chromium manifests are derived at package time (`browser-extension/scripts/package.sh` / `dev-unpacked.sh`). Loading the repo folder in Chrome yields **no background context at all** — always ship/load the derived build.
- **A production binary needs the `custom-protocol` feature.** `tauri`'s build script derives dev mode from it (`let dev = !has_feature("custom-protocol")` → `cargo:dev`, read by `tauri-build::is_dev()`), so a plain `cargo build --release -p dm-tauri` produces a **dev-configured** binary that loads `devUrl` (`localhost:5173`) and never reads `dist/` — it dies on "Could not connect to localhost" without a Vite server. Use `npm run tauri build`, or `cargo build --release -p dm-tauri --features tauri/custom-protocol`.
- `.taurignore` (root) keeps the Tauri CLI file-walker from EMFILE-panicking on `target/` + `node_modules/`. Keep those patterns.
- Gitignored scratch: `.pi/`, `coredump/`, `dist/`, `images/`, `target/`. Never commit from them.
- Tauri identifier is **`com.dm.downloadmanager`** (the old `com.dm.app` collided with `.app` on macOS). Do not change without updating the macOS bundle name.

## Commands (verbatim)

```bash
# Frontend
npm install
npm run dev            # Vite on http://localhost:5173 (Tauri expects this exact URL)
npm run build          # → dist/
npm run check          # svelte-check — must be 0 errors / 0 warnings

# Engine tests (unit + integration + HLS)
cargo test -p dm-engine
cargo test -p dm-engine --test integration remove_while_downloading   # CI gate for the remove/cancel race

# Full build (frontend bundle + embedded assets + installers)
npm run tauri build   # → target/release/dm-tauri and target/release/bundle/*

# Release body / version bump (docs first — the bump verifies both files)
node scripts/changelog-section.mjs --self-test
node scripts/changelog-section.mjs v<version>    # exactly what release.yml publishes
./scripts/release.sh <version>                   # bumps every manifest, tags v<version>

# Extension regression suite (no npm install needed)
cd browser-extension && node --test test/*.test.mjs    # + node test/mutation-check.mjs (must catch all)

# Lint the CSS-var surface (silent-failure guard)
node scripts/check-css-vars.mjs

# Probe the WebSocket listener without the GUI (dev only, port 9158)
# src-tauri/probe-listener/ — excluded from the workspace, never ships
```

**Before claiming done:** `npm run check` clean, `cargo test -p dm-engine` green, and — if you touched extension code — `node --test test/*.test.mjs` green. If you fixed an extension bug, add **both** a regression test and a mutation in `test/mutation-check.mjs`.

## Engine architecture (compressed)

`crates/engine` has **zero Tauri deps**. Read `manager.rs::run_download` + `task.rs` together when behaviour is unclear.

- `DownloadManager` (cheap `Arc` clone) owns tasks + `RunContext` (scheduler, global limiter, storage, event sink, proxy, spawner).
- **`Spawner` injection** — the engine never calls `tokio::spawn` directly. Tauri commands run on non-Tokio workers; always construct the manager with `with_settings_and_spawner()` (see `src-tauri/src/main.rs::setup`). The default constructor only works inside a Tokio runtime.
- `DownloadScheduler`: global slot semaphore + per-download connection cap. `CombinedLimiter`: global + per-download token buckets, more-restrictive wins. `0` rate = unlimited.
- `DownloadControl`: `AtomicBool` pause/cancel + `Notify`; `remove()` sets a `removed` flag (see invariant 2).
- Status lifecycle: `Queued → Connecting → Downloading → (Paused ⇄ Queued) | Completed | Error | Canceled`; `Scheduled` parks outside the schedule window (15 s ticker in `manager.rs`). `pause()`/`resume()` clear stale `d.error`.
- Events: `DownloadEvent` → `FrontendEvent` 1:1 on channel **`download-event`** as `{ kind: "added"|"progress"|"statusChanged"|"completed"|"error"|"removed", download }` (`removed` carries only `{ id }`). Frontend merges in `stores/downloads.ts`.
- Persistence (`storage.rs`, rusqlite **bundled**): tables `downloads`, `history`, `categories`, `settings`. The `status` column holds **JSON-serialized** `DownloadStatus` (`"completed"` with quotes) — compare with `status_as_json`, never a bare SQL string. `Storage::delete_download` is the hard-remove path used by `DownloadManager::remove()`.
- Proxy precedence: per-download `Download.proxy` → global `Settings.effective_proxy_url()` (`None | System | Manual`). `set_global_proxy` is the cheap update path.
- **HLS** (`hls.rs` + `task::run_hls_download`): a `.m3u8` is a playlist, never the payload. One master hop to the highest-bandwidth variant; segments fetched concurrently but appended in order; per-segment AES-128; optional ffmpeg remux. Refused with a reason: live playlists, SAMPLE-AES/DRM, encrypted fMP4 init, masters with separate audio renditions. Naming at add time: `.m3u8` → `.ts` (`Download::normalize_media()`), fMP4 → `.mp4`, remux retargets the row. Resume is **per segment** via `<save_path>.part.hls.json` (honoured only when URL + first segment + count still match).
- **Media merging** (`media.rs`): DASH video+audio halves. `remux()` is `ffmpeg -c copy` with order-independent `-map`, output verified via ffprobe (≥1 video **and** ≥1 audio) or it is deleted and refused. `.ts` segments are never paired. Output name avoids inputs (`<stem>.merged.<ext>` on collision), `.mp4` first then `.mkv` fallback, `kill_on_drop(true)` for the 15-min timeout. ffmpeg is looked up on `PATH`, never bundled. A successful merge calls `set_output_file()` so the row follows the file.

## Invariants — do not break

1. **Nothing is taken from the browser unless DM received it.** `onCreated` forwards **first** and only calls `takeOverChromeDownload()` when `send()` returned `true`. Undeliverable clicks fall back to `chrome.downloads.download()` with `ignoredUrls` ping-pong suppression and `safeFilename()`; the content script fails **open** (browser downloads when in doubt). Browser-side cleanup uses `downloads.removeFile()` only when the item is **not** `complete`.
2. **`remove()` must not emit `StatusChanged(canceled)`.** It calls `control.remove()` (not `cancel()`), drops the entry, `Storage::delete_download(id)`, emits only `Removed(id)`. The in-flight worker's finalize + progress flushes also bail on the `removed` flag — otherwise the UPSERT resurrects the row and the frontend shows a "canceled ghost". Pinned by `cargo test -p dm-engine --test integration remove_while_downloading` (the lib test `remove_does_not_emit_canceled_status_change` alone is **not** sufficient — it has no worker).
3. **Content script is opt-in only.** `type: "media"` messages are **dropped on the floor** in `background.js`; only `type: "grab"` (popup button) is accepted. No auto-scrape, no `MutationObserver` re-scan. If auto-grab ever returns it must be behind an explicit user preference.
4. **`chrome.downloads.onCreated` listener registers synchronously at top level**, filtered by `dmInstallTime` recorded only on `onInstalled({reason:"install"})`. `state.installTimeMs === 0` means "not read yet" — forward nothing until `loadInstallTime()` resolves (never substitute boot time).
5. **Transparent decompression stays off** (`.gzip(false).brotli(false).deflate(false)`). Some mirrors lie about `Content-Encoding`; the engine saves wire bytes verbatim. Always use `build_client`/`build_probe_client` (custom UA — bare `reqwest/x.y.z` gets 403'd by CDNs).
6. **Open-ended transfers** (`end == u64::MAX`, no `Content-Length`): send a plain GET with **no `Range:` header** (proxies 416 on ranges they can't satisfy), truncate the file, stream from 0. Not resumable across restarts. Auto-complete requires `downloaded >= OPEN_ENDED_MIN_BYTES` (1 KB) — below that it is `Error: open-ended transfer truncated at N B`. `ChunkState::size()` returns `u64::MAX` (no overflow in `remaining()`).
7. **Progress persists mid-transfer**: the aggregator flushes SQLite every `PROGRESS_FLUSH_INTERVAL` (1 s) / `PROGRESS_FLUSH_BYTES` (1 MiB) + once at stream end. Without this, "starts at 7% instead of 30%" is the default behaviour.
8. **Event/serialisation order** when adding fields: `crates/engine/src/model.rs` → `storage.rs` → `src-tauri/src/events.rs` → `src/lib/types.ts` → `src/lib/api.ts` → UI. Settings fields get `#[serde(default)]` so old rows load.
9. **CSS custom properties must all resolve.** An undefined `var()` silently drops the whole declaration. Short-name aliases (`--surface`, `--border`, `--text-muted`, …) live in the **theme-agnostic** `:root` of `src/app.css` and point at `--color-*` tokens — never inside a single theme block. `scripts/check-css-vars.mjs` is the gate.
10. **`window.__playinfo__`-style page expandos are invisible to content scripts** (isolated world). Firefox needs `window.wrappedJSObject` (`pageGlobal()`). Cross-frame media is observed in `background.js` via an **observational** `webRequest.onHeadersReceived` (never `webRequestBlocking`), bounded per-tab log persisted to `chrome.storage.session`.
11. **Resource-timing buffer keeps the OLDEST entries.** Hence `content.js` runs at `document_start`, mirrors into a 400-entry ring via `PerformanceObserver`, and calls `clearResourceTimings()` in the callback.
12. **Modal scroll containment is idempotent, not reference-counted.** `App.svelte`'s `setBodyScrollLocked(locked)` is driven by a level-triggered `$:` on `showSettings || showCommandPalette`: it returns early when `locked === scrollLocked` and otherwise sets `overflow: hidden !important` on `<html>`+`<body>`; `onDestroy` unlocks. A counter double-counts here — the reactive statement re-runs on every relevant store change, so the palette's "Open settings" path (settings opens while the palette is still up) increments twice and leaves the page frozen after both close. Every scrollable region inside a fixed panel declares `overscroll-behavior: contain`. New modal panels must join the same reactive condition.

## Gotchas (symptom → cause → rule)

- **"Spawn failed" / "no reactor running"** → a path that spawns was reached off the Tokio runtime; route through the injected `Spawner`.
- **"error decoding response body" / "Invalid gzip header"** → server lying about `Content-Encoding`. Don't re-enable transparent decompression.
- **403 from Tuna/kernel.org/sourceforge** → call site bypassed `build_client`; route through the engine helpers.
- **"Remove" not surviving restart** → `Storage::delete_download` isn't called from `DownloadManager::remove`.
- **Style/border/hover mysteriously missing** → undefined CSS custom property (silent). Run `node scripts/check-css-vars.mjs`.
- **Progress resets after hard kill** → aggregator `PROGRESS_FLUSH_*` missing; only that block persists `downloaded` mid-transfer.
- **Resume math**: workers use `chunk.resume_offset() = start + downloaded`; a fully-covered chunk (`remaining() == 0`) is **skipped**, never given a malformed `bytes=N-N-1` range.
- **Row stuck on "Connecting" during HLS** → the HLS branch must set `Downloading` itself; `run_download` returns before its own transition.
- **HLS progress jumps / reads 0 B/s** → `downloaded` must be on-disk **+ live** (`AtomicU64` for in-flight fetches).
- **`fmtBytes`** is unit-aware to `YB` (`≥1024 YB` past that); open-ended chunks render `879 B · unknown total`, never the `u64::MAX` sentinel. `DownloadRow` detects non-finite `chunk.end`.
- **Tray/indicator link failure on Linux** → the `libayatana-appindicator3` dev files are missing at build time. Install the distro package (`libayatana-appindicator3-dev` on Debian/Ubuntu, `libayatana-appindicator-gtk3-devel` on Fedora) — this is the *loud* version; at runtime Tauri only logs tray failures.
- **`cargo build --release` succeeds but the window stays blank / says "Could not connect to localhost"** → the binary was built without the `custom-protocol` feature, so it is dev-configured and loading `devUrl`. Build with `npm run tauri build` (or add `--features tauri/custom-protocol`).
- **Dropdown menu "follows the wrong row"** → stale `transform` or missed `DropdownMenu` reset; it re-measures on resize/scroll, flips above, clamps horizontally, caret clamps to `[14, menuW-14]`.
- **Scheduler tests time out** → they deliberately use `tokio::time::timeout(100ms, ...)` to assert blocked acquires. Don't widen without understanding the intent.
- **Extension "won't connect" (WS close 1006 is undecidable)** → layer it: (1) `node browser-extension/scripts/check-host.mjs` distinguishes `refused` / `no-response` (pre-0.4.2 app: line-JSON listener) / `bad-accept` / `ok`; (2) popup error `Receiving end does not exist` = **no background worker** (repo folder loaded in Chrome — build with `browser-extension/scripts/dev-unpacked.sh`); (3) to test without the GUI use `src-tauri/probe-listener/` (real `ws.rs`, port 9158).
- **"I pulled the fix but it's still broken"** → MV3 service worker doesn't reload from disk; click **Reload** on the extension card (check "Inspect views" shows the new source).
- **Firefox media links do nothing** → `nativeMessaging` permission missing (it was dropped for a whole release in 0.4.2): `connectNative is not a function` **and** the click was already `preventDefault()`ed. `node --test test/*.test.mjs` catches this.
- **A download vanished from `chrome://downloads`** → cancelled a browser download DM never received. See invariant 1; mutation `P0-3` reproduces it.
- **`scripts/*.sh` do nothing on a symlinked checkout** → `import.meta.url === process.argv[1]` fails when `pwd` is logical. Always `isMainModule()` from `scripts/lib/is-main.mjs`; `test/manifest.test.mjs` runs the tooling through a symlink to keep it that way.
- **Captures must notify** (desktop notification + in-app toast) — skipped only when `document.hasFocus()`, and that focus test lives in the event listener (`stores/downloads.ts`), not the dialog.
- **`Storage::load_active` must derive both sides of the comparison from the same serializer** — a SQL literal `'completed'` never matches the JSON `"completed"` (historical bug: every finished row came back active and got stuck `Queued`).

## Extension curation contract ("Grab page media")

`collectMedia(hints)` scans in descending trust: media elements (`currentSrc` → `.src` → raw attribute; run through `absoluteUrl()` — relative `src` is real) → HLS/DASH hints + downloadable `<a>` → Open Graph video → same-origin iframes' DOM **and their `performance` timeline** → top-frame resource timing → background `webRequest` hints (newest first). `add()` drops non-http(s) (`blob:`/`data:`), pathname-only HTML docs, duplicates.

Curation is one logical download, in priority order: `.m4s` pair (range stripped, signing params kept) → direct file → manifest → lone segment. Episode grouping may **reorder/prefer but never empty the set** (`dominantCid()` votes only among held bases; empty intersection falls back to all; unparseable cid bases are kept).

Empty results return `diag` + `note` + machine `reason` (`no_content_script` / `player_without_url` / `filtered_out` / `no_media_observed`) — keep those three distinguishable; never flatten to "no media found".

## Shell & frontend quick rules

- `main.rs`: `logging::init()` **first** (per-session log under `XDG_DATA_HOME`, `LogGuard.path()` for bug reports), then builder + plugins (`clipboard-manager`, `notification`, `autostart`), then `setup()` (storage → manager → event sink → manage → native-host TCP `127.0.0.1:9157` → schedule loop). Tray failures are logged, not fatal.
- `commands.rs` is the whole frontend-facing command surface; add there + `invoke_handler!` + `lib/api.ts` wrapper + `lib/types.ts` mirror + capabilities if a plugin permission is needed.
- Frontend stores are **not** persistent — rehydrate from `get_settings()` / `list_downloads()`. `$theme` (`dark|light|system`) persists to `localStorage["dm-theme"]`; `<html data-theme>` is set pre-paint by an inline script in `index.html`.
- Progress event throttle: engine 150 ms, frontend speed tick 1 Hz — both load-bearing for UI perf.
- UX work: read `docs/UI_DESIGN_PLAN.md` first.

## When you're stuck

1. Engine behaviour → `manager.rs::run_download` + `task.rs` together.
2. Serialisation mismatch → `model.rs` → `events.rs` → `types.ts` → `api.ts`.
3. Extension transport → `scripts/check-host.mjs` first, popup error text second, never guess from close code 1006.
4. Mystery CSS → `scripts/check-css-vars.mjs`, then the theme-agnostic alias block in `app.css`.
