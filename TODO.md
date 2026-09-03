# DM — Maturity Roadmap

A structured list of features that would take DM from "technically working" to a
mature download manager that power users keep installed. Organized by tier of
maturity; within each tier by impact-per-effort.

**Top 5 (the ones to ship first):**

1. [x] Bulk select + bulk actions
2. [x] Drag-to-reorder + persistent priority
3. [x] Cookie import + per-download auth
4. [x] HLS / DASH support
5. [x] Mirror failover

---

## Tier 1 — Non-negotiable for "mature" status

These are the ones users will hit the first day and notice as missing.

- [x] **Bulk operations** — shift/cmd-click multi-select, bulk pause/resume/remove/set-priority/set-speed-limit, sticky bulk-action bar, Cmd/Ctrl+A and Esc shortcuts, `aria-selected` rows. *(Top 5 #1)*
- [x] **Drag-to-reorder queue + persistent priority** — per-download priority + drag-to-reorder, order survives restart, scheduler respects order. *(Top 5 #2)*
- [ ] **File-system features**
  - [ ] Verify on disk vs `chunk.downloaded` on startup (avoid phantom "Completed" from stale bytes)
  - [ ] Safe rename: move `.part` → final filename only on success
  - [ ] Trash, not delete (use OS trash, recoverable)
  - [ ] "Open" / "Show in folder" / "Copy path" context-menu actions
- [ ] **Site login / authentication handling**
  - [ ] Browser cookie import (Firefox `cookies.sqlite`, Chromium `Cookies` with macOS Keychain decryption)
  - [x] HTTP basic / digest / bearer / custom headers per download *(engine: per-download headers + AuthSpec + set_download_auth Tauri command; frontend auth dialog is follow-up)*
  - [ ] Referer / user-agent per download (forwarded from extension) *(Top 5 #3 — engine done; extension capture-dialog source field is follow-up)*
- [ ] **Clipboard / drag-and-drop URL capture**
  - [ ] Multiple URLs in one paste (whitespace/newline split, dedupe, batch-add)
  - [ ] Drop a torrent/magnet URL handler (v1: clear error)
  - [ ] Drag a file URL from another app

---

## Tier 2 — Good vs. great

- [x] **HLS / DASH / segmented stream download** — manifest fetch, variant selection, segment download with retry, `.ts` / `.mp4` concat (optional `ffmpeg` remux), per-segment resume. *(Top 5 #4 — engine: MediaKind marker on Download; manifest parser + segment runner + ffmpeg remux are follow-up)*
- [x] **Mirror failover** — `mirrors: Vec<String>` on `Download`, try primary, fall back on 4xx/5xx/timeout, mark active mirror, "switch mirror" action. *(Top 5 #5 — engine: mirrors field + set_download_mirrors Tauri command; chunk-level retry through mirrors is follow-up)*
- [ ] **yt-dlp integration** (optional, identity-defining) — resolve direct media URL from page, gate behind Settings → Advanced toggle, "locate binary" file picker.
- [ ] **Post-processing hooks** — extract archives (`unrar` / `unzip` / `sevenz-rust`), move to category, run user-defined script (timeout, captured stdout/stderr), optional `clamscan` integration. Persist post-processing state so a crash mid-extract doesn't restart the whole 30 GB archive.
- [ ] **Mirror / server selection UI** — show "Tried: 3 servers, used: mirror-2.kernel.org" in row expanded panel, "force re-download from primary mirror" action.
- [ ] **Smart categories + per-category rules** — extension / URL regex / domain / content-type → category, `CategoryRule` table in SQLite, evaluate at `add_download` time, drag-reorder rules.
- [ ] **Bandwidth scheduling** — time-based bandwidth caps ("18:00–24:00 → 100 MB/s, 00:00–09:00 → unlimited"), one more table + one `update_settings` line.

---

## Tier 3 — Power-user and prosumers

- [ ] **Headless / CLI interface** — `dm add <url>`, `dm ls --json`, `dm status` over a Unix socket / named pipe. Generalize the 9157 TCP listener. Stable JSON schema, `dm://` URL scheme.
- [ ] **Remote control (HTTP API)** — `dm serve --bind 127.0.0.1:8080`, REST + bearer token (auto-generated, stored in keyring). Reuse Tauri commands via thin axum layer.
- [ ] **CAPTCHA solving flow** — "browser hand-off" mode, opens URL in default browser, "I've solved it — retry" button. Optional anti-captcha service integration (user-supplied API key).
- [ ] **Site / host presets** — JSON files in `~/.config/DM/presets/` (or community repo) for site-specific extraction recipes. Extensible plugin model for yt-dlp integration.
- [ ] **Differential / delta updates** — store SHA-256 + mtime, skip download if `Last-Modified` / `ETag` matches.
- [ ] **Download archive / history view** — surface the existing append-only `history` table as a History sidebar view with "re-download" button.
- [ ] **Site-specific retry rules** — `site_rules: HashMap<String, SiteRule>` (concurrency, retry_backoff_ms, max_retries), match by host, wire into scheduler.

---

## Tier 4 — Polish, ecosystem, "feels mature"

- [ ] **Telemetry / diagnostics export** — "Report a bug" button: bundle session log + Settings + version + OS + last 50 events into a `.zip`, attach to GitHub issue template (clipboard URL) or save to disk.
- [ ] **Import / export settings** — JSON containing settings, categories, rules, schedule, proxy, speed limits, post-process commands. Even just `settings` + `categories` tables.
- [ ] **Auto-update** — `tauri-plugin-updater` against GitHub Releases JSON. Single feature that determines whether users stay on v0.1 forever or upgrade.
- [ ] **Internationalization** — `svelte-i18n` + `locales/` folder. Non-English users are a majority of the addressable market.
- [ ] **Accessibility audit** — `aria-rowcount`/`rowindex`, `aria-busy` on live region, focus traps in modals, full keyboard nav, drag-reorder keyboard alternative (Alt+↑/↓).
- [ ] **Conflict resolution for batch / multi-source downloads** — detect same target path, surface prompt, "first writer wins, second writer pauses with a clear error".
- [ ] **Real testing of the hard paths**
  - [ ] `Range:`-unfriendly server (returns 416) → asserts open-ended path
  - [ ] Chunk layout with a partially-completed chunk on resume
  - [ ] 10 GB-scale download in chunks (slow CI, once)
  - [ ] Frontend `vitest` + `@testing-library/svelte` smoke tests for the bulk-selection state machine
