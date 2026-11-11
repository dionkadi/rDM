# Release Notes

Short, user-facing highlights for each release. The detailed history
lives in [CHANGELOG.md](CHANGELOG.md) and stays the source of truth;
this file is the distilled view kept in sync with it — every entry here
summarises the matching changelog section, and nothing appears here that
the changelog does not document.

`scripts/changelog-section.mjs` (used by the GitHub release workflow)
prefers this file and falls back to the changelog section for versions
without notes here, so a release body is concise when notes exist and
complete when they don't.

## [Unreleased]

### Added

- **Multi-algorithm checksums.** Downloads can be verified against
  MD5, SHA-1, SHA-256 or SHA-512; common tool spellings (`sha256sum`,
  `sha-256`, …) are accepted, and unsupported algorithms are rejected
  when the download is added instead of at 100 %.
- **Launch at login** toggle in Settings → General → Behavior
  (Registry Run key / LaunchAgent / `.desktop` autostart).

### Fixed

- **Windows: adding a task no longer wedges the UI.** Event-emitting
  Tauri commands run on the async runtime instead of the main thread.

## [0.7.0] - 2026-11-11

### Fixed

- **The global speed limit is actually global now.** It applied per
  download, so 4 concurrent downloads ran 4× the configured cap.
- **SHA-256 verification works.** It read the not-yet-renamed file and
  failed every checksummed download with "checksum read failed".
- **"No proxy" means no proxy.** Ambient `HTTP_PROXY`/`HTTPS_PROXY` env
  vars no longer hijack downloads when proxy mode is None.
- **Mirror failover is implemented.** When the primary URL fails to
  probe, mirrors are tried in order and the first working one becomes
  the primary URL for the rest of the download.

### Changed

- **SQLite opens in WAL mode.** Concurrent downloads no longer queue on
  storage writes and single-stream transfers are no longer capped by
  per-commit fsync — measured ~20× faster writes, 18× higher
  single-stream throughput and 12× faster 100-download batches.

### Added

- **Engine benchmark harness** (`cargo run --release -p dm-engine
  --example bench`): 12 scenarios with fault injection, HLS/AES-128,
  storage A/B and a 120 s soak; full reports in `BENCHMARK_REPORT.md`
  and `FIX_VERIFICATION.md`.

## [0.6.0] - 2026-11-10

- HLS `.m3u8` grabs download the actual video: playlist parsing,
  AES-128 decryption, per-segment retry and resume, optional ffmpeg
  remux to `.mp4` (the `.ts` is kept as the source).
- Media grabs are far more reliable: cross-origin player iframes,
  resource-buffer eviction and the page's playurl API are all used, so
  "no media found" is rare — and says why when it happens.
- A grabbed DASH pair arrives as one task, merges automatically when
  both halves finish, and is named/tagged with the page's metadata.
- Fixed completed tasks resurrecting as a stuck "Queued" row after a
  restart.

## [0.5.0] - 2026-09-26

- One grab = one task: DASH video+audio pairs download as a single row
  and auto-merge; filenames and metadata come from the page.
- Wrong-episode and promo-clip mispicks fixed (request-count based
  stream selection, grouped by content id).
- The page's playurl API backs the grab when the network buffer no
  longer holds the streams.
- Completed tasks self-heal after a restart instead of re-queueing at
  0 %; SQLite gets a 5 s busy timeout.

## [0.4.2] - 2026-09-05

- Chrome installs the extension again: MV3 manifests are split per
  browser at package time (service worker for Chromium, scripts for
  Firefox), with a packaging self-check.
- The popup's unclosable alert is replaced by a "Re-check host"
  button; Settings → About/Extensions show real versions.

## [0.4.1] - 2026-09-05

- Chromium browsers no longer need the native host: the extension
  talks to DM over WebSocket on `127.0.0.1:9157`. Firefox keeps the
  native-messaging shim.
- 3-step install for Chromium; `.rpm` joins `.deb`/`.AppImage` on
  Linux.

Versions older than 0.4.1 have no separate notes — see the matching
sections in [CHANGELOG.md](CHANGELOG.md).
