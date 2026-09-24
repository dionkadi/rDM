// Content script: two responsibilities, both opt-in.
//
// 1. **Click interception** — when the user left-clicks a link to a
//    downloadable URL (e.g. `<a href="archive.zip">`, `<a download>`,
//    or anything matching the `MEDIA_RE` heuristic), we suppress
//    the browser's default "start a download" behaviour and forward
//    the URL to the background so it can be pushed to DM. The
//    result: the browser never creates a "canceled" download row in
//    the downloads library — the user only sees the DM entry.
//
//    Three deliberate restrictions keep this from being hostile to
//    the browser it runs in:
//
//    * **Host-liveness gate.** We only intercept when DM is known to
//      be reachable (`hostAlive`). If the DM app isn't running, the
//      link is left completely alone and the browser downloads it
//      normally. Interception used to be unconditional, which meant
//      a dead host turned every media link into a silent no-op.
//    * **`e.isTrusted`.** Only real user clicks are intercepted, so
//      the page's own programmatic clicks are never swallowed.
//    * **No `stopPropagation`.** We call `preventDefault()` and
//      nothing else, so the page's own click handlers still run.
//      Interception used to call `stopImmediatePropagation()`, which
//      broke any site whose download flow is JS-driven. We still
//      suppress the browser's default action: `preventDefault()` in
//      a capture-phase listener on `documentElement` marks the event
//      cancelled for the whole dispatch, which is what stops the
//      browser from starting its own download.
//
//    Modifier keys (Ctrl, Cmd, Shift, Alt, Meta), middle-click, and
//    right-click are NOT intercepted — those keep the browser's
//    normal "open in new tab" / "save as" behaviour. "Save link as"
//    from the right-click context menu reaches the background's
//    `onCreated` path instead, which now only takes the download
//    over when it has actually handed the URL to DM.
//
// 2. **Manual media scan** — the popup's "Grab page media" button
//    explicitly asks the content script to scan via a `collect`
//    message. Only that path triggers an outbound `sendMessage`.
//
// We deliberately do NOT auto-scrape on every page load or every DOM
// mutation — that would flood the user's DM queue with media URLs
// from every page they visit (and from every re-render of those
// pages), which is the opposite of "you wanted a download manager,
// not a passive capture-everything tap on the browser".
const MEDIA_RE =
  /\.(m3u8|mp4|webm|mkv|mov|flv|avi|ts|m4v|ogg|mp3|m4a|wav|flac|zip|7z|rar|pdf|iso|exe)(\?|#|$)/i;

// Extensions that mean "this network request was media". Wider than
// MEDIA_RE because it has to cover DASH: `.m4s` segments and `.mpd`
// manifests are what MSE players (bilibili, and anything similar) fetch,
// and they are never links the user clicked — so `MEDIA_RE` would never
// see them.
//
// `.ts` is deliberately absent even though MPEG-TS exists: every
// TypeScript dev server serves `.ts` files, and tagging those as video
// would be worse than missing an occasional transport stream.
const RESOURCE_MEDIA_RE =
  /\.(m3u8|mpd|mp4|m4s|m4v|m4a|webm|mkv|mov|flv|avi|mp3|wav|flac|aac|ogg|opus)(\?|#|$)/i;

// Best-first ordering for what we hand back. Rank 0 (a real video file
// you can download as-is) beats a manifest, which beats a segment.
const MEDIA_EXT_RANK = [
  /\.(mp4|m4v|webm|mkv|mov|flv|avi)(\?|#|$)/i,
  /\.(m4a|mp3|flac|wav|aac|ogg|opus)(\?|#|$)/i,
  /\.(m3u8|mpd)(\?|#|$)/i,
  /\.(m4s|ts)(\?|#|$)/i,
];

// The grab feeds DM's capture dialog, which asks the user to confirm
// one URL at a time. A page like bilibili can expose dozens of segment
// requests, so cap the list and keep the best ones. `captureQueue` in
// the app is capped at 10, so stay under it.
const MAX_GRAB_URLS = 8;

function mediaRank(url) {
  for (let i = 0; i < MEDIA_EXT_RANK.length; i++) {
    if (MEDIA_EXT_RANK[i].test(url)) return i;
  }
  return MEDIA_EXT_RANK.length;
}

// ─── Mirrored background state ──────────────────────────────────
//
// `hostAlive`       — the DM host is reachable right now. Starts
//                     **false** on purpose: until the background
//                     tells us otherwise we must behave as if DM
//                     isn't installed. Failing open (letting the
//                     browser download) is always better than
//                     failing closed (swallowing the click).
// `interceptOnClick` — the user's popup toggle. When off, we never
//                     handle clicks; the popup's manual "Grab page
//                     media" button still works.
let hostAlive = false;
let interceptOnClick = true;

/**
 * True if this content script can still talk to its extension. After
 * the extension is reloaded or updated, already-open tabs keep
 * running the *old* content script with an invalidated runtime; any
 * `chrome.runtime.*` call then throws. Checking this up front means
 * an orphaned content script stops intercepting entirely instead of
 * calling `preventDefault()` and then failing to deliver.
 */
function runtimeAlive() {
  try {
    return !!(chrome && chrome.runtime && chrome.runtime.id);
  } catch (_e) {
    return false;
  }
}

function applyState(state) {
  if (!state) return;
  hostAlive = !!state.connected;
  if (state.settings) interceptOnClick = !!state.settings.interceptOnClick;
}

/**
 * Ask the background for the current state. The background pushes
 * every transition to us afterwards (see `broadcastState` in
 * background.js), so this only needs to run once at load.
 */
function refreshState() {
  if (!runtimeAlive()) {
    hostAlive = false;
    return;
  }
  try {
    chrome.runtime.sendMessage({ type: "get-status" }, (res) => {
      // Read `lastError` in the same tick as the callback — both
      // Chromium and Firefox clear it on the next runtime API call.
      if (chrome.runtime.lastError) {
        hostAlive = false;
        return;
      }
      if (res && res.state) applyState(res.state);
    });
  } catch (err) {
    hostAlive = false;
    console.warn("DM Grabber: get-status failed:", err);
  }
}

/** Walk the event's `composedPath()` to find the closest anchor. */
function findAnchor(e) {
  const path = e.composedPath ? e.composedPath() : [e.target];
  for (const node of path) {
    if (node && node.tagName === "A" && node.href) return node;
  }
  return null;
}

/**
 * Decide whether this click should be intercepted as a
 * "send-to-DM" download. Bails on:
 *   - non-primary mouse button (middle/right);
 *   - any modifier key (Ctrl/Cmd/Shift/Alt/Meta);
 *   - the event was already preventDefault'd (e.g. by the page);
 *   - a synthetic (non-trusted) click — only a real user gesture
 *     gets to trigger a takeover;
 *   - the resolved target is not an `<a>`;
 *   - the URL is not http(s) or doesn't look downloadable;
 *   - the user turned click handling off, or DM isn't reachable.
 *
 * "Looks downloadable" = the `<a>` has a `download` attribute OR
 * its URL matches `MEDIA_RE`. We deliberately keep this heuristic
 * narrow so that "click an HTML article link" doesn't accidentally
 * become a download.
 */
function shouldIntercept(e, anchor) {
  if (e.defaultPrevented) return false;
  if (e.button !== 0) return false; // primary only
  if (e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return false;
  if (e.isTrusted === false) return false; // synthetic clicks are not ours
  if (!anchor) return false;
  const url = anchor.href;
  if (!/^https?:/i.test(url)) return false;
  // ── The gate that keeps a dead host from breaking the browser ──
  if (!interceptOnClick) return false;
  if (!hostAlive) return false;
  if (!runtimeAlive()) return false;
  if (anchor.hasAttribute("download")) return true;
  if (MEDIA_RE.test(url)) return true;
  return false;
}

function onClickCapture(e) {
  const anchor = findAnchor(e);
  if (!shouldIntercept(e, anchor)) return;
  // Suppress the browser's default "start a download" behaviour.
  // `capture: true` on the listener (see below) plus the
  // synchronous preventDefault is what stops the browser creating
  // a download entry in the first place. We intentionally do NOT
  // stop propagation — the page's own handlers keep working.
  e.preventDefault();
  // Forward the URL to the background. This is async but that's
  // fine: the click has already been suppressed, and if the
  // background can't hand the URL to DM it re-issues the download
  // through `chrome.downloads` itself (see `handBackToBrowser` in
  // background.js), so the download is never lost.
  try {
    chrome.runtime.sendMessage({
      type: "download-click",
      url: anchor.href,
      filename: anchor.getAttribute("download") || anchor.textContent || "",
      // Forward the source page's URL as the Referer hint
      // and the browser's current User-Agent. The native
      // host relays these to the Tauri side, which
      // pre-fills the per-download headers so the user
      // can confirm them in the CaptureDialog. Some
      // servers (paywalled content, mobile-only mirrors)
      // gate downloads on these headers, and clicking a
      // link is the moment we know what the page Referer
      // is. Both fields are best-effort: if the browser
      // doesn't expose them (rare, but possible on a
      // hardened Firefox build), the native host falls
      // back to empty strings and the Tauri side shows
      // an empty input in the dialog.
      referer: location.href,
      userAgent: navigator.userAgent,
    });
  } catch (err) {
    // The runtime is gone (e.g. the extension was reloaded
    // mid-click). We can't undo the preventDefault, but we do
    // latch `hostAlive` off so every *subsequent* click in this
    // tab is left to the browser.
    hostAlive = false;
    console.warn("DM Grabber: sendMessage(download-click) failed:", err);
  }
}

/**
 * Find the media on this page.
 *
 * Used only by the popup's "Grab page media" button.
 *
 * Sources, most trustworthy first. A URL found by several heuristics
 * keeps its earliest (best) slot:
 *
 *   1. `<video>` / `<audio>` / `<source>` — `currentSrc` first, which is
 *      the URL the element actually resolved and loaded (`src` may be
 *      relative, or one of several `<source>` candidates).
 *   2. HLS/DASH hints and `<a>` links that look downloadable.
 *   3. Open Graph / Twitter card video.
 *   4. Same-page `<iframe>` srcs, **only if the URL itself looks like
 *      media**. An iframe pointing at `player.html` is an HTML document;
 *      handing that to a download manager is how "grab" used to return
 *      `iframe.html` / `player.html` instead of video. Media fetched
 *      *inside* a cross-origin iframe is still invisible from here —
 *      the honest answer for those pages is source 5 or nothing.
 *   5. Network activity — `performance.getEntriesByType("resource")`.
 *      This is the one that matters for MSE players: bilibili &co. load
 *      the stream with `fetch`/XHR into a `<video src="blob:…">`, so the
 *      DOM only ever shows `blob:` (which is not downloadable) while the
 *      real `.m4s` / `.mp4` / `.m3u8` URLs are sitting in the resource
 *      timing buffer. Newest first, because the currently-playing stream
 *      is the one the user wants.
 */
function collectMedia() {
  const ordered = [];
  const seen = new Set();
  const sources = { player: 0, link: 0, meta: 0, iframe: 0, network: 0 };

  const add = (raw, source) => {
    if (!raw || typeof raw !== "string") return;
    const url = raw.trim();
    // `blob:` (MSE), `data:`, `javascript:` and relative URLs are all
    // unusable as a download target.
    if (!/^https?:/i.test(url)) return;
    if (seen.has(url)) return;
    seen.add(url);
    ordered.push(url);
    sources[source]++;
  };

  // 1. Media elements.
  document
    .querySelectorAll("video, audio, source, picture source, track")
    .forEach((el) => {
      add(el.currentSrc, "player");
      add(el.getAttribute("src"), "player");
    });

  // 2. Manifests referenced in any form, then downloadable-looking links.
  document
    .querySelectorAll('a[href*=".m3u8"], a[href*=".mpd"], a[href*="manifest"]')
    .forEach((el) => add(el.href, "link"));
  document.querySelectorAll("a[href]").forEach((el) => {
    if (MEDIA_RE.test(el.href)) add(el.href, "link");
  });

  // 3. Open Graph / Twitter card video.
  document
    .querySelectorAll(
      'meta[property="og:video"], meta[property="og:video:url"], meta[name="twitter:player:stream"]',
    )
    .forEach((el) => add(el.getAttribute("content"), "meta"));

  // 4. Iframes, filtered — see the doc comment.
  document.querySelectorAll("iframe[src]").forEach((el) => {
    const src = el.getAttribute("src");
    if (src && RESOURCE_MEDIA_RE.test(src)) add(src, "iframe");
  });

  // 5. Network activity. `performance` is always present in a page
  //    context, but guard anyway: a grab that throws would look like
  //    "found nothing".
  try {
    const entries =
      typeof performance !== "undefined" &&
      typeof performance.getEntriesByType === "function"
        ? performance.getEntriesByType("resource")
        : [];
    // Reverse: the most recent requests are the most likely to be what
    // the user is watching right now.
    for (let i = entries.length - 1; i >= 0; i--) {
      const name = entries[i] && entries[i].name;
      if (name && RESOURCE_MEDIA_RE.test(name)) add(name, "network");
    }
  } catch (_e) {
    // Nothing to add; the other sources still count.
  }

  // Rank, then cap. Discovery order is preserved within a rank, so the
  // first `<video>` beats a later one and the newest network entry beats
  // an older one.
  const urls = ordered
    .map((url, index) => ({ url, index, rank: mediaRank(url) }))
    .sort((a, b) => a.rank - b.rank || a.index - b.index)
    .slice(0, MAX_GRAB_URLS)
    .map((x) => x.url);

  // Say where the URLs came from. Without this, a grab that finds
  // nothing (or finds segments instead of a single file) is
  // indistinguishable from a broken button.
  const total = Object.values(sources).reduce((a, b) => a + b, 0);
  let note;
  if (urls.length === 0) {
    note =
      total === 0
        ? "No media found in this page's DOM or network activity. Media " +
          "loaded inside a cross-origin iframe or fetched by a service " +
          "worker can't be seen from here."
        : "Found media, but none of it is a URL a download manager can use.";
  } else {
    const parts = [];
    if (sources.network) parts.push(`${sources.network} from network activity`);
    if (sources.player) parts.push(`${sources.player} from media elements`);
    if (sources.link) parts.push(`${sources.link} from links`);
    if (sources.meta) parts.push(`${sources.meta} from page metadata`);
    if (sources.iframe) parts.push(`${sources.iframe} from iframes`);
    note =
      `${urls.length} media URL(s)` +
      (parts.length ? ` — ${parts.join(", ")}` : "") +
      (ordered.length > urls.length
        ? ` (${ordered.length - urls.length} more not shown)`
        : "");
  }

  return {
    type: "media",
    pageUrl: location.href,
    // The same page-level hints the click path sends, so a grab
    // from the popup lands in DM with the Referer / User-Agent
    // pre-filled too.
    referer: location.href,
    userAgent: navigator.userAgent,
    urls,
    note,
  };
}

// Capture-phase click interceptor. Registered once at document
// load; event delegation means it picks up clicks on dynamically
// added anchors without an observer.
//
// **CRITICAL**: only register on real http(s) pages. In Firefox
// 109+, content scripts DO run on `about:newtab` / `about:home`
// when the manifest's matches pattern includes `<all_urls>` —
// and on those pages a capture-phase click listener on
// `documentElement` breaks the new-tab page itself (the search
// bar / address bar / tile clicks all stop working, IME input
// drops events, etc). We bail here if we're on a non-http(s) URL
// so we never register a listener on privileged pages.
const _loc = typeof location === "undefined" ? null : location;
const _proto = _loc ? _loc.protocol : "";
if (_proto === "http:" || _proto === "https:") {
  const install = () => {
    if (!document.documentElement) return;
    if (document.documentElement.__dmClickBound) return;
    document.documentElement.__dmClickBound = true;
    document.documentElement.addEventListener("click", onClickCapture, {
      capture: true,
      passive: false, // we call preventDefault, so passive must be false
    });
  };
  if (document.documentElement) {
    install();
  } else {
    // The page is still being parsed and `documentElement` isn't
    // there yet — wait for it.
    document.addEventListener("readystatechange", install, { once: true });
  }
  // Learn whether DM is reachable before the user's first click.
  refreshState();
}

// Respond to explicit "collect" messages from the popup / background,
// and accept state pushes from the background.
// We do **not** call `collectMedia()` ourselves and we do **not** start
// a MutationObserver — see the top-of-file comment for the rationale.
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (!msg) return;
  if (msg.type === "collect") {
    sendResponse(collectMedia());
    return;
  }
  if (msg.type === "state") {
    applyState(msg.state);
  }
});
