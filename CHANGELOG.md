# Changelog

All notable changes to DM will be documented in this file.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Note:** Versions before 1.0.0 (i.e. `0.x.y`) are pre-1.0 and may
> ship breaking changes between minor versions. Once we hit 1.0.0 we
> commit to the SemVer stability guarantees.

## [Unreleased]

### Fixed — "Test host connection" blamed a booting service worker

- **The popup's Test button (and "Grab page media") reported a cold MV3
  service worker as "the extension has no background — reinstall it."**
  `chrome.runtime.sendMessage` answers `Could not establish connection.
  Receiving end does not exist.` while a service worker is still being
  launched, and that string is byte-for-byte identical to the one a
  Chromium install with no service worker at all produces. The status
  poll has always retried it; the two buttons sent a single message, so
  the control whose entire purpose is a definitive answer was the one
  that lied — and it recommended reinstalling, about a worker that came
  up moments later. Both buttons now share `sendWithColdStartRetry`, with
  a longer budget than the poll (~2.7 s; the poll stays at 680 ms so it
  still fits inside its 1.5 s interval).
- **The two causes are no longer collapsed into one message.** The popup
  reads `chrome.runtime.getManifest().background`: with no
  `service_worker` it says the background isn't running and points at the
  install; with one it says the background isn't *answering* and points at
  `chrome://extensions` → Errors / the service worker console. A JSON dump
  (`probe returned no result: {"ok":false,…}`) is no longer shown in place
  of the error string.

### Fixed — Chrome had no icon of its own

- **The Chrome build shipped the Firefox template's SVG icon.** Chrome
  does not support SVG in `icons` ("SVG files are not supported for any
  icons declared in the manifest"). It does not refuse the load either —
  so the extension ran with a generic letter tile in the toolbar and a
  "Could not load icon" entry in `chrome://extensions`' error list, on the
  very screen a user opens when something is wrong. The Chrome transform
  now rewrites `icons` **and** `action.default_icon` to
  `icons/icon-{16,32,48,128}.png` (rasterised from the same SVG and
  committed), fails loudly if any referenced asset is missing, and
  `verify-package.sh` asserts the zip's icons are raster and present. The
  Firefox `.xpi` keeps the SVG.

### Fixed — Chrome extension could not connect on Windows (`disconnected (code 1006)`)

- **On Windows, every connection from the Chromium extension was accepted
  and then dropped.** The listener's `accept()` loop runs on a
  non-blocking `TcpListener`; POSIX hands back a *blocking* socket anyway,
  but **Winsock makes the accepted socket inherit the non-blocking mode**
  ("The newly created socket … has the same properties as socket s"). So
  on Windows the first read returned `WSAEWOULDBLOCK`, the connection was
  mis-dispatched to the legacy line-JSON reader, and that reader's
  `unwrap_or(0)` read the failure as EOF — the browser's
  `GET / HTTP/1.1` was never answered and Chrome reported `close code
  1006`, with nothing in the log. Linux and macOS were never affected,
  which is why it survived to a release.
  - `ws::accept_ready()` now puts accepted connections into blocking mode
    explicitly (both the app and the probe harness use it), and the
    first-byte peek moved into the per-connection thread so a client that
    connects and sends nothing can no longer stall the accept loop.
  - A read error in the line-JSON path is logged instead of being treated
    as EOF, which is what made the failure silent.
- **A failed `bind()` was terminal and invisible.** It was `eprintln!`-ed
  — and a Windows release build has no console, while `eprintln!` also
  bypasses the file logger — so a port that could never be bound looked
  exactly like "DM isn't running". The listener now retries every 3 s
  (first failure at ERROR, later ones at DEBUG) and records the reason:
  Settings → Extensions shows it as **"Port unavailable"** with the OS
  message. On Windows a reserved port range is the usual cause
  (`netsh int ipv4 show excludedportrange protocol=tcp`).
- **`check-host.mjs` gained a `closed` verdict**: "accepted the connection
  and hung up without a byte of HTTP" is now distinguished from
  "listening but silent" (the pre-0.4.2 line-JSON listener). They are
  identical to a browser and need different fixes. The self-test covers
  the new case, and the mutation suite fails if the two are collapsed.
- `probe-listener` can reproduce the Windows condition on any platform:
  `cargo run -- --winsock --no-blocking-reset` (broken) vs `--winsock`
  (fixed). It also no longer carries a copy of the app's dispatch logic —
  both call `ws::accept_ready` / `ws::peek_is_websocket`.

## [0.5.0] - 2026-09-26

### Fixed — completed task turning "Queued at 0 %" after a restart

- **A completed task could come back as "Queued" at 0 % after an app
  restart while the file sat intact on disk.** Chain: the merged row was
  resumed/re-downloaded from its ORIGINAL part URL (the ▶ button
  dispatched `resume` unconditionally — for a merged row that overwrites
  the merged file and usually ends in `403` because the part URL's CDN
  signature is stale), persisted as `error` with zeroed progress, and on
  restart the error row loaded as active → `Queued`, with the on-disk
  verification zeroing the progress (no `.part` file). Fixes:
  - `resume()` ignores completed rows (a completed row is final);
  - startup **self-heals** any active row whose final file is already on
    disk at the full expected size — it is restored to Completed with
    matching progress instead of being re-queued (your stuck
    "美国女孩….mkv" row heals itself on the next start);
  - `Storage::open` sets a 5 s SQLite **busy timeout** — two writers on
    one database (second instance / crash-recovery) no longer fail
    immediately and strand a resumed download as "Queued".

### Fixed — drag & drop + capture notification

- **Task cards are no longer draggable — the app is the drop target.** The
  row was `draggable` for a reorder-by-drag feature, and its handlers
  `preventDefault()`-ed every dragover while reading only their private
  payload type: dragging the card hauled it around like a ghost AND
  dropping a link from the browser onto a row was silently swallowed
  (the app-level "drop a URL to download" handler never fired). Rows are
  no longer draggable; external drops now bubble to the app handler and
  add the download, with the existing drop overlay as the affordance.
  Reordering stays on the keyboard (Alt+↑/↓).
- **The "New download captured" notification uses the page title**
  (system-wide and the in-app toast) instead of the raw stream id.

### Fixed — task name + title suffix

- **The single task now shows its final name from the start.** The
  visible pair row displayed the engine's part filename
  (`41xxx-1-100022.m4s`) until the merge renamed it; the display now
  carries the name typed in the capture dialog (`<title>.mkv`) from
  confirm to finish, with combined progress — and while one half is
  still downloading the task no longer flips to "Completed" and back.
- **The `_哔哩哔哩_bilibili` suffix is trimmed from harvested titles.**
  The og/title metas repeat bilibili's decorated document title; only
  TRAILING decoration is stripped (a title mentioning bilibili
  mid-sentence keeps it), so the file is `…取景地？.mkv` and the
  container's `title` tag matches. The metadata harvest now sticks to
  what content scripts can actually see (DOM title + meta tags — page
  JS globals like `__INITIAL_STATE__` are invisible from the isolated
  world), and the uploader meta was added.
- Notification timing for pairs: the halves' completions are silent;
  one "Download Complete — <title> is ready" fires when the merge
  succeeds.

### Added — page metadata + meaningful filenames

- **The merged file now carries the page's metadata, and the filename is
  the video's title.** The grab harvests the source page's title, upload
  date and uploader (Bilibili's structured page state, with og:/itemprop
  metas as fallback) and rides them through the capture payload to the
  app. The CaptureDialog pre-fills `<title>.<ext>` (sanitized) instead
  of the raw stream id, and `merge_downloads` applies
  `-metadata title/date/artist` to the merged file — ffprobe-verified.
- The Rust `remux` gained an optional `OutputMeta` parameter; the merge
  command accepts `metadata` from the paired-capture flow (persisted
  with the pair so an app restart mid-download keeps it).

### Fixed — one task, sane progress, working category picker

- **A confirmed pair is now ONE task in the list from start to finish.**
  The two engine rows still exist underneath (both halves download in
  parallel), but the second half is hidden and its progress folds into
  the visible row (downloaded/total are the sum) — no more "two tasks
  that become one after merging". Control actions (pause / resume /
  cancel / remove / trash, single and bulk) fan out to the hidden half,
  a hidden-half failure surfaces on the visible task, and the pair
  state is persisted so restarting the app mid-download keeps the
  single-task view and the auto-merge arming.
- **Progress no longer runs past 100 % after a merge.** `set_output_file`
  left the row's `total_size`/chunks describing the old part while the
  file on disk was the merged result — the UI read 258–300 %. It now
  makes the row self-consistent: total = downloaded = merged file size,
  one completed chunk.
- **"Download complete" notification spam fixed.** The engine emits a
  completed status more than once per row (task `Completed`, then the
  merged file's retargeting `StatusChanged`) and the frontend notified
  on *every* emission. Notifications are now once per row (and never
  for the hidden halves).
- **The CaptureDialog category dropdown keeps the user's choice.** The
  form re-initialiser ran on every `captureQueue` invalidation (the
  array is replaced on each store update even when the capture is the
  same object), resetting the selection between picking and confirming;
  it is now keyed on the capture nonce. The category options also
  re-apply once settings finish loading, and the dialog backdrop no
  longer uses `backdrop-filter` (WebKitGTK composites native `<select>`
  popups incorrectly over blurred layers — the dropdown could render
  detached/unclickable).

### Fixed — wrong stream + leftover parts

- **The grab picked a 14-second promo clip instead of the 33-minute
  video the user was watching.** Stream selection keyed on "newest
  observed segment request", and a hover preview / promo clip's segments
  are fetched *most recently* — two requests out-shouted the watched
  video's hundreds. Selection is now **request-count based**: the cid
  whose segments dominate the resource log (raw request count, ties to
  the newest) is the episode the user is actually watching; previews and
  preloads cannot out-shout it. The heuristic and the play-info
  enhancer share the same selection.
- **The video part's `.m4s` lingered in the save directory after the
  auto-merge.** The merge retargets the first row at the merged file,
  but its ORIGINAL part file stayed on disk (only the second part was
  cleaned up). The auto-merge now also moves the first part's original
  file to the OS trash (`trash_paths` command), leaving exactly one
  file — the merged one — for a confirmed pair.

### Fixed — the pair collapsed to a lone half

- **The popup said "2 URLs sent", DM queued one — whichever half came
  first.** The extension's pair flag was ignored downstream:
  `media::plan` paired two `.m4s` only when they shared **origin and
  directory**, but bilibili's playurl hands the video and audio from
  *different mirrors* — the check failed, the plan degraded to
  `Single { first_url }`, and the other half was silently dropped.
  Fixes across the whole chain:
  - the extension stamps grab batches with `pair: true` when it grouped
    the URLs as one logical download (by content id);
  - the app honours the flag: a flagged two-URL batch becomes one
    paired capture regardless of mirrors (`plan_captures`);
  - `media::plan` additionally pairs two DASH streams that share a cid
    (`{cid}-1-{codec}.m4s`) even across mirrors, and refuses to pair
    streams of different cids (two episodes);
  - the Firefox native host forwards a grab batch **as one payload**
    (with the `pair` flag and page credentials) instead of one message
    per URL, which also un-paired cross-mirror grabs.
  End state for a confirmed pair is unchanged and is what the user
  asked for: two temporary part rows → automatic merge → **one file
  with both video and audio**, leftover part moved to the OS trash.

### Fixed — the lone video half again

- **A grab late in a video's lifetime again offered a lone video half**
  (correct episode, but no audio). The playurl *request* had fallen out
  of the resource buffer — the audio half buffers completely early in a
  video, so only video ranges keep flowing — and the enhancer's only
  source was gone, silently. Now:
  - the enhancer first reads **`window.__playinfo__`** — the playurl
    payload bilibili embeds in every watch page. No network, and it can
    never fall out of a buffer;
  - buffer playurl requests remain a secondary source (up to five,
    newest first);
  - a dash response that carries video but **no audio track** (Bilibili
    previews) is skipped with a reason instead of silently ending the
    enhancement;
  - when no source produces a pair, the grab note now **says why** —
    e.g. `(play info: play-info fetch failed)` or `(play info: play
    info has no audio track for stream …)` — instead of leaving a
    half-offer unexplained.

### Fixed — one episode at a time

- **The grab could pick the wrong episode (and offer a video without its
  audio).** Playlist/season pages accumulate several playurl responses in
  the resource buffer (preloaded next parts, previously watched parts),
  and the play-info enhancement trusted "the newest response" — which
  can describe a *different* episode than the one the player is
  streaming. Both sides now group observed streams by their embedded
  content id (`{cid}-1-{codec}.m4s`):
  - the heuristic pairs only bases belonging to the **newest-observed
    cid** (the stream the player is actually fetching) — a cross-episode
    pair is impossible;
  - the enhancer fetches play-info responses newest-first (up to three)
    and uses the first whose dash **contains that cid**, skipping
    responses that lack an audio track (e.g. previews);
  - the popup note names the chosen stream (`stream {cid}`) so a
    mismatch with an already-downloaded half is visible before
    confirming.

### Added — the grab uses the page's play info

- **The grab offers the complete video+audio pair even when the page's
  network buffer doesn't.** A grab on a bilibili watch page offered the
  audio half alone: the resource-timing buffer is finite (~250 entries,
  a long session drops the newest media requests), so one stream's
  requests simply weren't observable anymore. The content script now
  recognizes the page's **playurl API request** in the buffer,
  re-fetches it with the page's cookies, and reads the stream list from
  its JSON — `dash.video[]` / `dash.audio[]` name BOTH halves explicitly,
  signed, on a normal CDN. Observed halves are matched to their API
  entries by path tail (the representation actually playing); an
  unobserved half falls back to the API's best entry. P2P edge URLs
  (`mcdn`/`pcdn`, including via `backupUrl`) are only used when nothing
  else exists. Legacy non-DASH pages (`durl`) offer their direct file.
  A failed/timeout play-info fetch falls back to the pure heuristics.
- The resource-timing buffer is raised to 1024 entries at content-script
  start, keeping the rest of a long session observable.
- The grab now answers asynchronously only when a play-info enhancement
  is possible; every other page keeps the synchronous path.

### Fixed — the "150 B completed m4s" bug

- **The download "completed" with a 150-byte file that contained nothing.**
  Root cause chain, diagnosed against the real bilibili PCDN URL: the
  download **probe** did not carry the per-download credentials (Referer /
  User-Agent), so an anti-leech CDN answered it with `403 + text/html +
  Content-Length: 150`; the probe then **read that error page's headers as
  a successful probe** — it never checked the response status after its
  HEAD→ranged-GET fallback — and planned `total_size = 150`. The chunk
  worker (which *did* carry the Referer) requested `Range: bytes=0-149`,
  got a legitimate `206` with the first 150 real bytes of the stream, and
  the task renamed it done. Three fixes:
  - the probe now sends the same per-download headers (Referer,
    User-Agent) and auth as the chunk workers;
  - a non-success probe response (403/401/404…) fails the download with
    `HTTP <status>` instead of describing an error page;
  - on a `206` probe response the total comes from `Content-Range`, never
    from the partial `Content-Length` (which is the 1-byte probe range —
    the same class of bug behind the earlier "1 B chunk" report).
  Regression-tested with an anti-leech test server (403 without Referer)
  and a HEAD-rejecting server (405, 206 probe): with Referer → full byte-
  exact download; without → loud `HTTP 403` error, no file created.

### Fixed — DASH URL reconstruction

- **`400 Bad Request` (with a 1-byte "chunk") when downloading a grabbed
  `.m4s`.** The grab stripped the *entire* query string from DASH stream
  URLs to turn byte-range requests into whole-file URLs — but that
  destroyed the URL's signing parameters (`upsign`, `deadline`, …), and
  bilibili's PCDN edge (`mcdn`) rejected the anonymous request outright.
  Now only the player's `range` parameter is removed and every other
  query parameter survives, which is exactly how the full-file URL
  relates to the observed range requests.
- **P2P edge copies are demoted.** When the player fetches a stream from
  an `mcdn`/`pcdn` node, that node often refuses whole-file requests.
  If a normal-CDN copy of the same stream is also observable, the grab
  prefers it; an unavoidable P2P pick is offered with a warning in the
  popup note ("let the video play a bit and grab again").

### Added — paired captures

- **One grab, one download — the IDM contract.** "Grab page media" no
  longer floods DM with every media-shaped request a page made. The
  content script now curates the findings down to at most two URLs that
  form **one logical download**: the two most recent distinct `.m4s`
  stream bases (query stripped — the base serves the whole stream) for a
  DASH page, otherwise the best direct media file, otherwise a manifest.
  Loose `.ts` segments, duplicate byte-range requests and
  extensionless URLs are never offered. The popup note says what was
  selected and what was deliberately left out.
- **Paired captures with automatic merge.** When a grab yields a DASH
  pair, the app shows **one** confirmation dialog ("video + audio —
  merged when both finish"); confirming queues both halves with the
  same category / headers / speed limit, and the merge (`ffmpeg -c
  copy`, the same verified remux as the manual Merge action) runs
  automatically when both parts complete. The merged file takes the
  name typed in the dialog (`merge_downloads` gained an optional
  `output_name`), and the leftover second part moves to the OS trash.
  A part that fails or is cancelled cancels the auto-merge with a
  clear toast instead of merging half a video.
- Fixed primary-button **hover wash-out** in `CaptureDialog` and
  `AuthDialog`: the generic `.btn:hover` rule outranked `.btn.primary`
  and swapped the accent for a translucent near-white surface while the
  text stayed white — the "Download" button became unreadable on hover.

### Changed — grab curation + rebuild discipline

- Grab link heuristics no longer match `a[href*="manifest"]` — the
  substring appears in ordinary page URLs and produced junk captures
  (e.g. rows named "web"). Real manifests are matched by their
  `.m3u8` / `.mpd` extension.
- The grab is answered by the top frame only; an embed iframe can no
  longer win the multi-frame response race with its own URL as the
  page context (a second `player.html` leak path) or with an empty
  result that masked the top frame's media. Same-origin iframe DOMs
  are now scanned from the top frame instead.
- `crates/native-host`: a grab batch (`{"urls":[…]}`) no longer also
  forwards the message's `url` field — that field is the *page* the
  grab ran on, and forwarding it sent DM the embed/watch page itself.
  Batch captures are additionally grouped through `media::plan` so the
  Firefox native-messaging path pairs DASH streams identically to the
  WebSocket path.

Browser-extension correctness pass. The extension had two
interception paths and **neither was gated on DM being reachable**:
interception was unconditional, only *delivery* was conditional.
When the DM app was closed, a click on a `.zip` link did nothing at
all, and a download the browser started on its own was cancelled and
erased with nothing to replace it. Full audit:
[`EXTENSION_AUDIT.md`](EXTENSION_AUDIT.md).

### Added

- **Merging a DASH video + audio pair into one playable file.** Sites on
  MPEG-DASH (bilibili and friends) serve video as **two independent
  streams**, so no single URL was ever the whole video: you got a picture
  with no sound, or sound with no picture, and no amount of grabbing could
  fix that. Select two finished downloads → **Merge** → one file, via
  `ffmpeg -c copy` (no re-encode, so fast and lossless; `.mp4`, falling
  back to `.mkv` when the codecs are not MP4-compatible).
  `crates/engine/src/media.rs` owns the grouping heuristic and the remux;
  `probe_ffmpeg` / `merge_downloads` expose it to the UI.
  The design leans on verifying the *result* rather than the guess — URL
  shape cannot tell a video half from an audio half (bilibili names both
  `…-30280.m4s` / `…-30232.m4s`) — so both inputs are mapped optionally
  (`-map 0:v? -map 0:a? -map 1:v? -map 1:a?`), which makes their order
  irrelevant, and the merged file is then required to contain at least one
  video **and** one audio stream. A wrong pairing fails with a message and
  removes its own half-written output, instead of leaving a picture-less
  "success". The two sources stay on disk, and the first row is retargeted
  at the merged file so the result is visible in the list rather than
  orphaned on disk.
  ffmpeg is not bundled — `probe_ffmpeg` lets the UI disable the button
  *with a reason* instead of failing after the click. Still open (see
  TODO.md): manifest parsing / segment download for `m3u8` / `mpd`, and
  automatic pairing at capture time so this becomes one click instead of
  "select the two rows".
- **`browser-extension/test/`** — a dependency-free regression suite
  (48 tests) on `node:test`, with a fake `chrome.*`, a controllable
  clock, a minimal DOM and a fake `WebSocket`. `background.js`,
  `content.js` and `popup.js` are loaded through `node:vm` into a
  *shared* context, so messages really travel between them. Plus
  `test/mutation-check.mjs`, which re-introduces each fixed defect and
  requires the suite to catch it (9/9 today) — a regression test that
  cannot fail is decoration.
- **`browser-extension/scripts/check-host.mjs`** — answers "is 9157
  actually speaking WebSocket?" by doing what a browser does (TCP
  connect, Chrome-shaped upgrade request, verify
  `Sec-WebSocket-Accept`, exercise a masked close frame) and reporting
  which layer is broken: nothing listening, listening but silent (the
  pre-0.4.2 build signature), listening but not WebSocket, wrong accept
  key, or OK. Sends no URLs to DM. `--self-test` runs it against fake
  listeners of each kind and is wired into CI.
- **`src-tauri/probe-listener/`** — runs the **real** `ws.rs` listener as
  a plain terminal program on port 9158, so the extension's Chromium
  transport can be tested without building or launching the Tauri app.
  Excluded from the workspace; never shipped.
- **`scripts/check-css-vars.mjs`** — fails the build when a `var(--x)`
  has no definition and no fallback. CSS custom properties fail
  *silently* (the whole declaration is dropped at computed-value time),
  and neither `svelte-check` nor the Vite build looks inside `var()` —
  which is how the invisible Download button shipped. It strips comments
  first, so a commented-out definition doesn't count as one. Wired into
  CI, and used to find the 91 dead declarations above.
- **`browser-extension/scripts/dev-unpacked.sh`** — builds
  `dist/chrome-unpacked/`, a Chrome-loadable *directory* for "Load
  unpacked" built from your working tree. Editing the repo's `.js` files
  does nothing to a `.zip` install, and pointing "Load unpacked" at the
  repo folder gives Chrome an extension with **no background service
  worker** (Chrome ignores `background.scripts` under MV3), which
  presents as `Could not establish connection. Receiving end does not
  exist.` — indistinguishable from "DM isn't running" unless you have the
  probe. The script reuses the packaging path, so the dev directory is
  byte-identical to what users install, and it fails hard if the staged
  manifest would leave Chrome without a background.
- **`browser-extension/scripts/make-chrome-manifest.mjs`** — the
  Firefox→Chromium manifest transform, extracted from `package.sh` so
  packaging and the dev build share one implementation (and one set of
  guards) instead of drifting.
- **Two behaviour switches in the popup** (`dmSettings` in
  `chrome.storage.local`, both default on so nothing changes for an
  existing user): *Take over media link clicks* and *Take over all
  browser downloads*. The second one is the escape hatch that did not
  exist — previously every download in the browser profile was hijacked
  after install with no way to opt out.
- `WS_QUEUE_TTL_MS`: a payload that can't be delivered is dropped after
  30 s instead of being replayed as a surprise dialog whenever DM
  eventually starts.

### Fixed

- **A dead host no longer breaks the browser** ([`EXTENSION_AUDIT.md`](EXTENSION_AUDIT.md) P0-2).
  `content.js` caches host liveness (pushed on every transport
  transition) and skips interception entirely when DM is unreachable or
  when the extension runtime was invalidated by a reload. It fails
  *open*: if in doubt, the browser does the download.
- **A download is no longer cancelled unless DM received it**
  (P0-3). `chrome.downloads.onCreated` forwarded the URL and then called
  `takeOverChromeDownload()` unconditionally — so with DM down, a
  Ctrl-click or right-click "Save link as" was cancelled mid-transfer,
  erased from `chrome://downloads`, and lost. The takeover now happens
  only when `send()` returned `true`.
- **Firefox never could connect, but still swallowed links** (P0-1).
  `nativeMessaging` had been dropped from the manifest in 0.4.2 while
  `chrome.runtime.connectNative` was still called for Firefox, so
  `runtime.connectNative` was not a function — the transport could never
  connect, while clicks were still `preventDefault()`ed. The permission
  is declared again, and `scripts/package.sh` strips it from the
  Chromium `.zip` (which doesn't need it) with `verify-package.sh`
  asserting both shapes.
- **A click that can't be delivered is handed back to the browser**
  rather than dropped (`handBackToBrowser`), with an `ignoredUrls` guard
  so the fallback can't loop back through `onCreated`, and a
  `safeFilename()` check because `downloads.download` throws on an
  invalid filename.
- **`downloads.onCreated` is registered synchronously at the top level**
  (P1-4), as MV3 requires; the install-time filter it used to wait for is
  applied inside the handler. `installTimeMs` uses `0` to mean "not read
  yet" and nothing is forwarded while it is — a worker woken *by* a
  download would otherwise compare that download against a boot time
  later than its own `startTime` and silently drop it.
- **The truncated partial is actually removed** (P1-5).
  `downloads.erase()` never deletes bytes; the file is now removed with
  `downloads.removeFile()` — but only when the item is not `complete`,
  so a finished file is never destroyed.
- **The popup reports the truth** (P1-6). "Grab page media" hardcoded
  `{connected: true, lastSentCount: 1}` and ignored both
  `chrome.runtime.lastError` and the background's answer, so the one
  screen that exists to say "DM is unreachable" said the opposite.
- **Interception no longer hijacks the page** (P2-7). The content script
  called `stopImmediatePropagation()`, which broke every site whose
  download flow is JS-driven. It now calls `preventDefault()` only, and
  additionally requires `e.isTrusted`, so a page's own programmatic
  clicks are never swallowed.
- **Referer and User-Agent now reach DM on the click path.** The content
  script had always sent them; the background discarded them, so
  `CaptureDialog` never pre-filled the per-download headers.
- **`probeHost()` can no longer resolve twice** with a stale result, and
  the Firefox transport now calls back so "Test host connection" doesn't
  always burn the full timeout.
- **Packaging: `zip` *updates* an existing archive.** Stale entries from
  previous builds (a whole `test/` tree, an old manifest) survived into
  the release because `dist/dm-grabber-<v>.zip` was never deleted first.
  Both package scripts remove the outputs up front, the staging list
  excludes `test/`, and `verify-package.sh` fails if development files
  ship.
- **The popup no longer blames the DM host for everything.**
  "Not connected" always rendered as *"✗ DM host not running"*, which is
  wrong in the most common case: a Chromium MV3 extension with no
  service worker at all (the repo folder loaded instead of the `.zip`)
  reports `Could not establish connection. Receiving end does not exist.`
  and has nothing to do with DM. The popup now distinguishes a missing
  background from a blocked socket from an unreachable host, and the
  host case points at `check-host.mjs` instead of leaving the user to
  guess between "the app is closed" and "the app is too old".
- **The build tooling could silently do nothing.** Node resolves the
  entry module to its *real* path for `import.meta.url` but leaves
  `process.argv[1]` exactly as given, so the usual
  `import.meta.url === process.argv[1]` main-module check never fires
  when a script is reached through a symlink — and `pwd`, which
  `package.sh` uses to compute its own directory, returns the logical
  path. On a symlinked checkout the manifest transform therefore ran
  nothing, exited 0, and `zip` shipped the **untransformed Firefox
  manifest** into the Chromium `.zip`: an extension that installs fine
  and then does nothing, with `set -e` perfectly happy. Tooling scripts
  now use `isMainModule()` from `scripts/lib/is-main.mjs`, which compares
  realpaths, and `test/manifest.test.mjs` invokes both tooling scripts
  through a symlink so a regression can't pass silently again.
- **A superseded WebSocket can no longer knock out the live one.** The
  `close` handler nulled the shared `ws` reference unconditionally, so
  the "Test host connection" button — which closes the current socket
  and opens a replacement in the same tick — would let the *old*
  socket's `close` event flip the state to "disconnected" while a
  perfectly good socket was open, and schedule yet another connect,
  leaking a connection per click. Each socket's handlers are now bound
  to their own socket.

### Fixed — app-side correctness

- **The capture dialog's "Download" button was invisible.** It is
  `color:#fff` on `background: var(--color-accent)`, and the
  `--color-accent*` aliases lived under `[data-theme="light"]` only —
  so dark mode was already falling back to `.btn`'s elevated
  background, and an unrelated edit that deleted those three lines
  broke light mode too: white text on a near-white surface, with no
  error anywhere. The aliases now live in the theme-agnostic `:root`
  with the rest of the back-compat map (custom properties are
  substituted at use time, so one definition resolves in both themes).
  Completing that map uncovered **13 more names nothing defined**
  (`--surface`, `--border`, `--text-faint`, `--success`, `--danger`,
  `--warning`, `--info`, …): **91 declarations across the app were
  silently dead**, costing borders, hover backgrounds and status
  colours on every screen. All aliased now, and enforced by CI.
- **A removed download could reappear as a "canceled ghost"** and only
  disappeared when some later action refreshed the list. Removing a
  *running* download deleted its SQLite row and emitted `Removed`, and
  then the in-flight chunk worker — which only notices the cancel flag
  when it next polls — ran its finalize path, which `save_download`s the
  row (an UPSERT, so the row really came back, including after a
  restart) and emitted `StatusChanged(Canceled)`, which the frontend
  re-inserted because it had already dropped the row. `DownloadControl`
  now carries a `removed` flag distinct from `cancel`; the aggregator's
  progress flushes and the finalize path both skip persisting and
  emitting once it is set, and the frontend additionally ignores events
  for ids it has removed. The pre-existing test for this never started
  the download, which is why it passed while the bug lived — the new
  `remove_while_downloading_leaves_no_ghost_row` integration test removes
  a task mid-transfer and is now a CI gate.
- **Nothing told the user a URL had been captured.** The `CaptureDialog`
  is a modal inside a window the user is almost certainly not looking at
  (they just clicked a link in their *browser*), so a capture was
  silent. A capture now raises a desktop notification as well as the
  in-app toast — the notification is suppressed while the window has
  focus, so it doesn't duplicate what is already on screen.
- **"Grab page media" returned the player's HTML instead of video.**
  Two causes. The `<iframe>` branch added *every* iframe URL with no
  filter, so `player.html` / `iframe.html` came back as "media"; it now
  only offers iframes whose URL itself looks like media. And an MSE
  player never puts the stream in the DOM — bilibili loads it into
  `<video src="blob:…">` via `fetch` — so the grab now also scans
  `performance.getEntriesByType("resource")`, which is where the real
  `.m4s` / `.mp4` / `.m3u8` URLs live. Results are ranked (a
  downloadable file beats a manifest beats a segment), capped at 8 (the
  app's capture queue holds 10), deduped, and come with a note saying
  where they came from — or why nothing usable was found (media inside
  a cross-origin iframe is not visible from the page).

### Changed

- `manifest.json` permissions are now `downloads`, `tabs`,
  `nativeMessaging`, `storage`. `scripting` and `activeTab` were
  declared but never used (verified by a test that derives the required
  permission set from the source), and `web_accessible_resources` — which
  let any site fingerprint that DM Grabber is installed — is gone; the
  popup never needed to be reachable from a page.
- `com.app.dm.native.firefox.json` no longer ships a developer-machine
  absolute path; `path` is a placeholder the user edits.

### Documentation

- Corrected `AGENTS.md`'s claim that "Chrome reads `background.scripts`
  as a service worker" (Chrome refuses the key before 121 and ignores it
  after; the repo manifest is the Firefox template and the Chromium
  manifest is generated at package time), removed the references to the
  host-manifest templates deleted in 0.4.2, and documented the
  "nothing is taken from the browser unless DM received it" invariant.
- `INSTALL.md`: per-manifest permission table, the Firefox
  `nativeMessaging` prerequisite, the behaviour switches, and two
  troubleshooting rows that pointed at the wrong mechanism.

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
