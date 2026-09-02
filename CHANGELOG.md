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
