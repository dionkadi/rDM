# Changelog

All notable changes to DM will be documented in this file.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** Versions before 1.0.0 (i.e. `0.x.y`) are pre-1.0 and may
> ship breaking changes between minor versions. Once we hit 1.0.0 we
> commit to the SemVer stability guarantees.

## [Unreleased]

### Added

- Cross-platform release pipeline: tag-driven GitHub Actions workflow
  that builds Linux `.deb`/`.AppImage`, Windows `.msi`/`.exe`, and
  macOS `.dmg`/`.app` installers and attaches them to a GitHub
  Release (see `.github/workflows/release.yml`).
- Browser-extension packaging job that produces both
  `dm-grabber-<version>.zip` (Chrome/Edge/Brave/Arc) and
  `dm-grabber-<version>.xpi` (Firefox) and uploads both to the
  same release.
- Pre-flight CI: `cargo check` + `svelte-check` + extension
  manifest/syntax checks run on every push to main and every PR
  (`.github/workflows/ci.yml`).
- `scripts/release.sh` — local script that bumps `package.json`,
  `tauri.conf.json`, every `Cargo.toml`, and `browser-extension/manifest.json`
  in lockstep, regenerates `Cargo.lock`, and creates an annotated
  `v<version>` tag in one step.
- `browser-extension/scripts/package.sh` / `package.ps1` — local
  packaging for the extension only (used by the CI packaging job and
  for ad-hoc dev builds).
- `.github/CODEOWNERS` — replace `@your-github-handle` with your
  GitHub user/team to wire up auto-reviewers for the CI and bundle
  config paths.
- This CHANGELOG and a new **Releasing** section in `README.md`
  documenting the tag → release flow, the macOS Gatekeeper
  workaround, and the local packaging commands.
- **File-system features** (TODO Tier 1):
  - **Trash, not delete** — new `trash_download` Tauri command
    moves the on-disk file (and its `.part` sibling if any)
    to the OS trash via the `trash` crate (macOS Finder
    Trash, Linux XDG Trash, Windows Recycle Bin) before
    dropping the engine state. Wired into the per-row
    dropdown menu and the bulk action bar so the user
    can batch-trash completed downloads.
  - **"Open" / "Show in folder" / "Copy path"** — new
    `open_file` command launches the OS default handler
    for the download's final filename, the existing
    `open_folder` opens the parent directory (or reveals
    the file with `select_file=true`), and a new
    `copy_text` command routes clipboard writes through
    the Tauri plugin so they work inside the embedded
    webview. All three are wired into the per-row
    dropdown menu; "Copy path" is new.
- **Site login / authentication handling** (TODO Tier 1):
  - **Browser cookie import** — new `cookies` module in
    the engine reads Firefox `cookies.sqlite`
    (cross-platform, plaintext) and Chromium `Cookies`
    (Linux only; macOS/Windows encrypted → clear
    error). Returns a list of `BrowserCookie` records
    the frontend can toggle, plus a pre-formatted
    `Cookie: a=1; b=2` header value. New
    `import_browser_cookies` Tauri command + new
    `AuthDialog` component (singleton, controlled by a
    writable store) with a "Import from browser…"
    button, per-cookie checklist, and "Apply as Cookie
    header" action.
  - **Per-download auth dialog** — new `AuthDialog`
    component lets the user attach HTTP basic / bearer
    credentials, custom headers (Referer, User-Agent,
    Cookie, X-Requested-With, …) or imported browser
    cookies to a single transfer. All auth data is
    in-memory only (a restart clears it) — auth secrets
    don't sit in plain-text SQLite. The dialog is
    triggered from a new "Auth…" entry in the per-row
    dropdown menu.
  - **Referer / User-Agent per download** — the
    browser extension's content script now forwards
    `referer: location.href` and `userAgent:
    navigator.userAgent` to the background, the
    `dm-native-host` binary relays them in the JSON
    payload, the Tauri `CapturedUrl` carries them, and
    the `CaptureDialog` pre-fills the per-download
    headers with toggles for "Send Referer from
    source page" and "Send browser User-Agent". The
    engine's existing `set_download_auth` path handles
    the request side (per-chunk `apply_extra_headers` +
    `Authorization` injection).

### Notes

- macOS builds are **ad-hoc signed** for now. End users have to
  right-click → Open on the first launch. See the **Releasing**
  section in `README.md` for the workaround and the secrets
  required to enable proper signing + notarization.

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
