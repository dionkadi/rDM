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

// A URL whose path is a document is never media. `og:video` tags
// routinely point at *player pages* (…/player.html?vid=…) rather than
// streams, and a "grab media" scan must not offer an HTML document to
// a download manager — that is exactly how `player.html` / `iframe.html`
// rows used to land in DM. Checked after the query/fragment is
// considered, so `movie.mp4?player=1` is unaffected.
const DOCUMENT_RE = /\.(html?|xhtml)(\?|#|$)/i;

// Best-first ordering for what we hand back. Rank 0 (a real video file
// you can download as-is) beats a manifest, which beats a segment.
const MEDIA_EXT_RANK = [
  /\.(mp4|m4v|webm|mkv|mov|flv|avi)(\?|#|$)/i,
  /\.(m4a|mp3|flac|wav|aac|ogg|opus)(\?|#|$)/i,
  /\.(m3u8|mpd)(\?|#|$)/i,
  /\.(m4s|ts)(\?|#|$)/i,
];

function mediaRank(url) {
  for (let i = 0; i < MEDIA_EXT_RANK.length; i++) {
    if (MEDIA_EXT_RANK[i].test(url)) return i;
  }
  return MEDIA_EXT_RANK.length;
}

/**
 * Resolve `raw` against the document's base URL.
 *
 * A content script sees whatever the page wrote into the attribute:
 * `<video src="/v/clip.mp4">`, `<source src="../a.m4a">`, or a
 * protocol-relative `//cdn/…`. Those are perfectly good media URLs,
 * but they are not `^https?:` strings, and the old `add()` rejected
 * them outright — which is why a page whose player had not started
 * buffering (so `currentSrc` was still empty, leaving only the
 * relative attribute) reported "no media" while the same page worked
 * after the user pressed play.
 *
 * `blob:` / `data:` / `javascript:` stay non-http(s) after
 * resolution, so the caller's scheme check still filters them.
 */
function absoluteUrl(raw) {
  if (!raw || typeof raw !== "string") return null;
  const value = raw.trim();
  if (!value) return null;
  let base;
  try {
    base =
      (typeof document !== "undefined" && document.baseURI) ||
      (typeof location !== "undefined" ? location.href : undefined);
  } catch (_e) {
    base = undefined;
  }
  try {
    return new URL(value, base).href;
  } catch (_e) {
    return null;
  }
}

/** Resource-timing URL list for a window, in the browser's own order. */
function entriesFrom(perf) {
  try {
    if (!perf || typeof perf.getEntriesByType !== "function") return [];
    return perf
      .getEntriesByType("resource")
      .map((e) => (e && e.name) || "")
      .filter(Boolean);
  } catch (_e) {
    return [];
  }
}

// ─── Our own resource-request log ───────────────────────────────
//
// `performance.setResourceTimingBufferSize` only raises the cap; once
// the buffer is full the browser stops recording **new** entries (it
// does not evict the oldest). On a long video the requests made while
// the user is watching are therefore the ones that go missing — the
// buffer keeps the page-load preloads and loses the stream. Raising
// the cap at `document_start` shrinks the window but cannot close it,
// so we mirror every entry into our own bounded ring buffer as it
// arrives and clear the browser's buffer to keep it from filling.
const OBSERVED_MAX = 400;
const observed = [];
const observedSeen = new Set();

function rememberResource(name) {
  if (!name || typeof name !== "string") return;
  if (observedSeen.has(name)) return;
  observedSeen.add(name);
  observed.push(name);
  if (observed.length > OBSERVED_MAX) {
    observedSeen.delete(observed.shift());
  }
}

/**
 * All resource URLs this frame knows about, newest first: the
 * browser's live buffer, then our ring for whatever already fell out
 * of it. Duplicates are dropped by the caller's `seen` set.
 */
function resourceNames() {
  const names = entriesFrom(typeof performance !== "undefined" ? performance : null);
  names.reverse(); // getEntriesByType is oldest-first
  for (let i = observed.length - 1; i >= 0; i--) names.push(observed[i]);
  return names;
}

function startResourceObserver() {
  if (typeof PerformanceObserver !== "function") return;
  const onEntries = (list) => {
    try {
      for (const entry of list.getEntries()) rememberResource(entry && entry.name);
      // Keeping the browser's buffer empty is what stops it from
      // reaching its cap and silently dropping the newest entries.
      if (typeof performance.clearResourceTimings === "function") {
        performance.clearResourceTimings();
      }
    } catch (_e) {
      // A throw here would kill the observer: swallow it.
    }
  };
  try {
    new PerformanceObserver(onEntries).observe({
      type: "resource",
      buffered: true,
    });
  } catch (_e) {
    // Older syntax (`entryTypes`) or no observer support at all:
    // `resourceNames` still reads the live buffer, which is what the
    // code did before.
    try {
      new PerformanceObserver(onEntries).observe({ entryTypes: ["resource"] });
    } catch (_inner) {
      // No observation available.
    }
  }
}

/**
 * True when two URLs live on the same host in the same directory —
 * the layout sites use when a DASH pair has explicit extensions
 * (`…/video.m4a` + `…/video.mp4` side by side). Two random files from
 * different corners of a CDN are not a pair.
 */
function sameDirAndHost(a, b) {
  try {
    const ua = new URL(a);
    const ub = new URL(b);
    if (ua.origin !== ub.origin) return false;
    const dir = (p) => p.replace(/[^/]*$/, "");
    return dir(ua.pathname) === dir(ub.pathname);
  } catch (_e) {
    return false;
  }
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
 * P2P edges (bilibili's `mcdn`, generic `pcdn` hosts) serve byte ranges
 * to the player and routinely refuse whole-file requests — the 400 in
 * the "1 B chunk" report was such a node. When a normal CDN copy of the
 * same stream is also observable, prefer it. Shared by the grab
 * heuristics and the play-info enhancer.
 */
function isP2pUrl(u) {
  let host = "";
  try {
    host = new URL(u).hostname;
  } catch (_e) {
    return false;
  }
  return /(^|\.)(mcdn|pcdn)\./i.test(host);
}

/**
 * `41875472475-1-30216.m4s` → `41875472475`. The stream path embeds the
 * content id (cid); grouping by it is how the grab tells "the stream the
 * user is watching" apart from the other episodes/preloads that playlist
 * pages accumulate in the resource buffer. `null` when the tail doesn't
 * follow that shape.
 */
function cidOfMediaUrl(u) {
  const tail = pathTail(u);
  const m = tail.match(/^(.+)-\d+-\d+\.m4s$/i);
  return m ? m[1] : null;
}

/**
 * cid → number of RAW resource-timing requests (no dedup). The video the
 * user is actually watching generates the most segment traffic by far —
 * a 33-minute video means hundreds of range requests — which out-shouts
 * hover previews / promo clips / preloaded episodes that fetch a handful
 * of segments. Pure "newest wins" selection picked such a clip over the
 * watched video; request counts do not.
 *
 * `allowed` (a set of base URLs) restricts the vote to streams we can
 * actually offer. Without it the counter could elect a cid whose bases
 * never made it into `dashBases`, and the caller's episode filter then
 * discarded every real base.
 */
function observedCidCounts(allowed) {
  const counts = new Map();
  try {
    const names = resourceNames();
    for (const name of names) {
      if (!name || !/\.m4s(\?|#|$)/i.test(name)) continue;
      if (allowed && !allowed.has(name.split("#")[0].split("?")[0])) continue;
      const cid = cidOfMediaUrl(name);
      if (!cid) continue;
      counts.set(cid, (counts.get(cid) || 0) + 1);
    }
  } catch (_e) {
    // Fall through.
  }
  return counts;
}

/**
 * The cid with the most segment requests; ties go to the newest
 * (entries are scanned newest-first). `null` when nothing observable.
 */
function dominantCid(allowed) {
  const counts = observedCidCounts(allowed);
  let max = 0;
  for (const n of counts.values()) {
    if (n > max) max = n;
  }
  if (max === 0) return null;
  try {
    const names = resourceNames();
    for (const name of names) {
      if (!name || !/\.m4s(\?|#|$)/i.test(name)) continue;
      if (allowed && !allowed.has(name.split("#")[0].split("?")[0])) continue;
      const cid = cidOfMediaUrl(name);
      if (cid && counts.get(cid) === max) return cid;
    }
  } catch (_e) {
    // Fall through.
  }
  return null;
}

/**
 * Strip the site's title decorations from a harvested title. Bilibili
 * pages carry `<title>…_哔哩哔哩_bilibili` and og/title metas repeat
 * the suffix; only TRAILING decoration is removed (a title that
 * legitimately mentions bilibili mid-sentence stays intact).
 */
function cleanPageTitle(t) {
  return (t || "")
    .replace(/\s*[-_]\s*(哔哩哔哩\s*)?bilibili\s*$/i, "")
    .replace(/\s*[-_]\s*哔哩哔哩\s*$/i, "")
    .trim();
}

/**
 * Harvest the source page's metadata for the merged file: title,
 * upload date, uploader. Content scripts run in an isolated world and
 * cannot read page JS globals (`window.__INITIAL_STATE__` is invisible
 * here), so this reads what IS visible to us: the document title and
 * the page's meta tags. Every field is optional.
 */
function collectPageMeta() {
  const meta = {};
  const metaContent = (sel) => {
    try {
      const el = document.querySelector(sel);
      const v = el ? (el.getAttribute("content") || "").trim() : "";
      return v || null;
    } catch (_e) {
      return null;
    }
  };
  const rawTitle =
    cleanPageTitle(metaContent('meta[property="og:title"]')) ||
    cleanPageTitle(metaContent('meta[name="title"]')) ||
    cleanPageTitle(
      typeof document !== "undefined" ? document.title : null,
    );
  if (rawTitle) meta.title = rawTitle;
  const rawDate =
    metaContent('meta[itemprop="uploadDate"]') ||
    metaContent('meta[itemprop="datePublished"]') ||
    metaContent('meta[property="og:release_date"]') ||
    metaContent('meta[name="date"]');
  if (rawDate) meta.date = rawDate;
  const rawUploader =
    metaContent('meta[property="og:video:owner"]') ||
    metaContent('meta[name="author"]') ||
    metaContent('meta[itemprop="author"]');
  if (rawUploader) meta.uploader = rawUploader;
  return meta;
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
 *
 * Everything `collectMedia` finds is then *curated* down to at most two
 * URLs — one logical download, IDM-style (see the selection block at
 * the bottom of this function). The raw list can be dozens of byte-
 * range requests for the same two streams; the user wants the video
 * they are watching, not its fragments.
 *
 * Sources 1–4 run against every document we can legally read: the top
 * document **plus same-origin child iframes** (`contentDocument` is
 * null for cross-origin iframes, so those are skipped silently). That
 * keeps media embedded in a same-origin player visible without any
 * cross-frame races.
 */
function collectMedia(hints) {
  const ordered = [];
  const seen = new Set();
  const hintKind = new Map();
  const sources = {
    player: 0,
    link: 0,
    meta: 0,
    iframe: 0,
    network: 0,
    frame: 0,
    log: 0,
  };
  let players = 0;
  let blobPlayers = 0;
  let unusablePlayers = 0;
  let networkSeen = 0;

  const add = (raw, source, kind) => {
    // Resolve first: a relative `src` is a real media URL, not junk.
    const url = absoluteUrl(raw);
    if (!url) return;
    // `blob:` (MSE), `data:`, `javascript:` are unusable as a
    // download target.
    if (!/^https?:/i.test(url)) return;
    // HTML documents are not media (see DOCUMENT_RE). Test the
    // *path* only: a query string may legitimately contain ".html"
    // (redirect/tracking params) without making the response a page.
    let pathname = url;
    try {
      pathname = new URL(url).pathname;
    } catch (_e) {
      pathname = url;
    }
    if (DOCUMENT_RE.test(pathname)) return;
    if (seen.has(url)) return;
    seen.add(url);
    ordered.push(url);
    sources[source]++;
    if (kind === "file" || kind === "manifest") hintKind.set(url, kind);
  };

  // Documents to scan for sources 1–4. `contentDocument` access is
  // wrapped per-iframe: on exotic setups (sandboxed frames) it can
  // throw, and a throw here would look like "found nothing".
  //
  // Each entry keeps the frame's `window` too: `performance` is
  // per-window, so a same-origin player iframe has its own resource
  // timeline that the top document's scan can never see.
  const docs = [
    { doc: document, win: typeof window !== "undefined" ? window : null },
  ];
  try {
    document.querySelectorAll("iframe").forEach((el) => {
      let inner = null;
      let win = null;
      try {
        inner = el.contentDocument;
      } catch (_e) {
        inner = null;
      }
      try {
        win = el.contentWindow;
      } catch (_e) {
        win = null;
      }
      // Dedupe: nested iframes are visited once via their own parent
      // scan only when we push them here — we deliberately do NOT
      // recurse deeper than one level to bound the work.
      if (inner) docs.push({ doc: inner, win });
    });
  } catch (_e) {
    // The top document scan below still runs.
  }

  for (const { doc } of docs) {
    // 1. Media elements. `.currentSrc` is the URL the element actually
    //    resolved and loaded; `.src` resolves a relative attribute;
    //    `getAttribute("src")` is the last resort (and is what an
    //    un-started `<video preload="none">` leaves us with).
    doc
      .querySelectorAll("video, audio, source, picture source, track")
      .forEach((el) => {
        const tag = String(el.tagName || "").toUpperCase();
        if (tag === "VIDEO" || tag === "AUDIO") players++;
        const raw = el.getAttribute("src") || "";
        const resolved = el.currentSrc || el.src || raw;
        const hasBlob = /^blob:/i.test(resolved) || /^blob:/i.test(raw);
        if (hasBlob) blobPlayers++;
        // No URL at all, or only a `blob:` handle: the element proves
        // there IS media here even though we cannot name it.
        if (!resolved || hasBlob) unusablePlayers++;
        add(resolved, "player");
      });

    // 2. Manifest and media links. The old extra `a[href*="manifest"]`
    //    selector is gone on purpose: "manifest" appears in ordinary
    //    page URLs too (web-app manifests, routes, query params), and
    //    each such link became a junk capture row — the "some named
    //    `web`" report. The extension tests below pin real manifests
    //    via their `.m3u8`/`.mpd` extension instead.
    doc
      .querySelectorAll('a[href*=".m3u8"], a[href*=".mpd"]')
      .forEach((el) => add(el.href, "link"));
    doc.querySelectorAll("a[href]").forEach((el) => {
      if (MEDIA_RE.test(el.href)) add(el.href, "link");
    });

    // 3. Open Graph / Twitter card video. The DOCUMENT_RE guard in
    //    `add()` is what keeps a player-page `og:video` out.
    doc
      .querySelectorAll(
        'meta[property="og:video"], meta[property="og:video:url"], meta[name="twitter:player:stream"]',
      )
      .forEach((el) => add(el.getAttribute("content"), "meta"));

    // 4. Iframes, filtered — see the doc comment.
    doc.querySelectorAll("iframe[src]").forEach((el) => {
      const src = el.getAttribute("src");
      if (src && RESOURCE_MEDIA_RE.test(src)) add(src, "iframe");
    });
  }

  // 5. Network activity. `performance` is always present in a page
  //    context, but guard anyway: a grab that throws would look like
  //    "found nothing". `resourceNames` merges the live timeline with
  //    our own ring buffer, newest first — the most recent requests
  //    are the ones describing what the user is watching right now.
  try {
    const names = resourceNames();
    for (let i = 0; i < names.length; i++) {
      if (!RESOURCE_MEDIA_RE.test(names[i])) continue;
      networkSeen++;
      add(names[i], "network");
    }
  } catch (_e) {
    // Nothing to add; the other sources still count.
  }

  // 5b. Same-origin child frames. Their requests live in *their*
  //     resource timeline, so an embedded same-origin player used to
  //     be invisible even though its DOM was scanned. `contentWindow`
  //     is readable for same-origin frames only; a cross-origin one
  //     throws on `.performance` and is skipped by `entriesFrom`.
  for (let i = 1; i < docs.length; i++) {
    const names = entriesFrom(docs[i].win && docs[i].win.performance);
    for (let j = names.length - 1; j >= 0; j--) {
      if (!RESOURCE_MEDIA_RE.test(names[j])) continue;
      add(names[j], "frame");
    }
  }

  // 6. Background hints — the extension's own `webRequest` log. This
  //    is the only source that covers media fetched inside a
  //    cross-origin iframe, MSE players with extensionless URLs, and
  //    streams whose resource entries were already dropped. The
  //    background classifies each by `Content-Type` and only reports
  //    "file" or "manifest" (never a fragment).
  for (const hint of Array.isArray(hints) ? hints : []) {
    if (hint && hint.url) add(hint.url, "log", hint.kind);
  }

  // ── One grab, one download (the IDM contract) ────────────────────
  //
  // A DASH page fetches the same two streams over and over as byte-
  // range requests, so the resource buffer holds hundreds of `.m4s`
  // URLs that differ only by `?range=…` — sending them raw is what
  // flooded DM with a "bunch of downloads". What the user actually
  // wants is the ONE thing they are watching:
  //
  //   * a **pair** of distinct `.m4s` bases (the `range` parameter
  //     stripped, signing params kept — see `withoutRangeParam`) → DM
  //     downloads both halves and merges them into one playable file;
  //   * otherwise a direct media file (`<video src>`, a real link, a
  //     direct CDN hit);
  //   * otherwise a manifest — honest but limited: DM can only fetch
  //     the playlist file itself today, so it is offered last;
  //   * loose `.ts` segments, duplicate range requests and
  //     extensionless URLs are never candidates — they are pieces or
  //     guesswork, not files.
  const isManifestUrl = /\.(m3u8|mpd)(\?|#|$)/i;
  const isDashSegment = /\.m4s(\?|#|$)/i;
  const isTsSegment = /\.ts(\?|#|$)/i;

  /**
   * Remove ONLY the `range` query parameter from a URL, keeping
   * everything else (auth/signing params like `upsign`, `deadline`,
   * `expires` are what makes the URL downloadable at all).
   *
   * The previous heuristic stripped the ENTIRE query — which destroyed
   * the signature and produced `400 Bad Request` from bilibili's PCDN
   * edge (mcdn) for a request that had no credentials left. The full
   * stream is the URL minus `range`: the player adds `range` per fetch;
   * the CDN serves the whole file when it is absent.
   */
  function withoutRangeParam(raw) {
    const qIdx = raw.indexOf("?");
    if (qIdx === -1) return raw;
    const base = raw.slice(0, qIdx);
    let query = raw.slice(qIdx + 1);
    let fragment = "";
    const fIdx = query.indexOf("#");
    if (fIdx !== -1) {
      fragment = query.slice(fIdx);
      query = query.slice(0, fIdx);
    }
    const kept = query
      .split("&")
      .filter((kv) => kv.split("=")[0].toLowerCase() !== "range");
    return base + (kept.length ? "?" + kept.join("&") : "") + fragment;
  }

  const files = [];
  const manifests = [];
  // Key = the URL's path form (no query). Value = `{ index, url, cid }`:
  // the candidate to offer for that stream (range param stripped,
  // signing params kept), the discovery index of its most recent
  // occurrence — network entries are scanned newest-first, so the
  // smallest index is the freshest request — and the stream's content
  // id (`cid`), which is how episodes/preloads on playlist pages are
  // told apart.
  const dashBases = new Map();

  ordered.forEach((url, index) => {
    if (isDashSegment.test(url)) {
      const key = url.split("#")[0].split("?")[0];
      const cid = cidOfMediaUrl(url);
      const cand = withoutRangeParam(url);
      const prev = dashBases.get(key);
      if (prev === undefined) {
        dashBases.set(key, { index, url: cand, cid });
        return;
      }
      const prevHasAuth = prev.url.includes("?");
      const candHasAuth = cand.includes("?");
      if (
        (candHasAuth && !prevHasAuth) ||
        (candHasAuth === prevHasAuth && index < prev.index)
      ) {
        // Prefer a candidate that still carries its signing params
        // (more likely to be accepted whole), newest among equals.
        dashBases.set(key, { index, url: cand, cid });
      }
    } else if (isTsSegment.test(url)) {
      // `.ts` never becomes a candidate: one piece of a longer stream.
    } else if (isManifestUrl.test(url) || hintKind.get(url) === "manifest") {
      manifests.push(url);
    } else if (mediaRank(url) < 2 || hintKind.get(url) === "file") {
      // Rank 0/1: a real video or audio file extension. The background
      // hint covers streams whose URL has no extension at all
      // (`/videoplayback?…`) but whose `Content-Type` is video/* or
      // audio/* — that is a whole file served with ranges, not a
      // fragment.
      files.push(url);
    }
    // Anything else (rank ≥ 2 non-manifest, extensionless URLs with no
    // typed hint) is dropped: it cannot be described as "the file"
    // confidently.
  });

  const candidates = [];
  // Lowest discovery index wins: sources 1–4 (the page DOM) are pushed
  // before source 5, and the network scan pushes newest requests first —
  // so ascending index order is "most authoritative / most recent"
  // first. **Only one episode at a time**: playlist/season pages
  // accumulate several cids in the buffer (preloaded next parts,
  // previously watched parts), and pairing a video from one episode
  // with audio from another merges two different videos into one file.
  // The newest base's cid is the stream the player is actually
  // fetching, so the pair is drawn from that cid alone.
  const bases = [...dashBases.values()].sort((a, b) => a.index - b.index);
  // P2P edge copies are demoted within the pair: a normal CDN base
  // downloads fine, an mcdn copy usually refuses whole-file requests.
  const rankBases = (list) =>
    list
      .slice()
      .sort(
        (a, b) =>
          Number(isP2pUrl(a.url)) - Number(isP2pUrl(b.url)) ||
          a.index - b.index,
      );
  // The episode the player is ACTUALLY streaming: the one whose
  // segments dominate the request log. Pure "newest wins" loses to
  // hover previews and promo clips — a 14-second clip fetched just now
  // would out-shout the 33-minute video the user has been watching
  // (which streamed hundreds of segment requests). Ties go to the
  // newest; when nothing carries a recognizable cid, fall back to the
  // newest base's cid.
  //
  // The vote is restricted to the bases we actually hold: scanning the
  // raw buffer (as this used to) could elect a cid whose streams never
  // made it into `dashBases`, which then filtered every real base out
  // and produced "no media" on a page that was playing fine.
  const currentCid =
    dominantCid(new Set(dashBases.keys())) ??
    (bases.length > 0 ? bases[0].cid : null);
  // When no base carries a recognizable cid (generic DASH URLs like
  // `…/seg.m4s` don't embed one), skip the episode grouping entirely —
  // there is nothing to group by.
  //
  // The grouping is allowed to *reorder* and *prefer*, never to empty
  // the set: an empty intersection falls back to every base, and bases
  // whose cid is unparseable are kept as candidates because they are
  // not evidence of a different episode.
  const matching = bases.filter(
    (b) => currentCid === null || (b.cid !== null && b.cid === currentCid),
  );
  let offerBases = matching.length > 0 ? matching : bases;
  if (offerBases.length < 2) {
    const unknown = bases.filter(
      (b) => b.cid === null && !offerBases.includes(b),
    );
    if (unknown.length > 0) offerBases = offerBases.concat(unknown);
  }
  offerBases = rankBases(offerBases);
  if (offerBases.length >= 2) {
    candidates.push({
      kind: "pair",
      urls: [offerBases[0].url, offerBases[1].url],
    });
  }

  if (candidates.length === 0) {
    if (files.length > 0) {
      candidates.push({ kind: "file", urls: [files[0]] });
      // A second file is only worth offering when it very likely
      // belongs to the first — same host, same directory (sites that
      // use explicit extensions for DASH keep both halves together).
      if (files[1] && sameDirAndHost(files[0], files[1])) {
        candidates.push({ kind: "file", urls: [files[1]] });
      }
    } else if (manifests.length > 0) {
      candidates.push({ kind: "manifest", urls: [manifests[0]] });
    } else if (offerBases.length === 1) {
      // A lone segment is a fragment, not a file — only surfaced when
      // the page gave us nothing better at all.
      candidates.push({ kind: "segment", urls: [offerBases[0].url] });
    }
  }

  const urls = candidates.flatMap((c) => c.urls);

  // Say where the URLs came from — and what was deliberately left out.
  // Without this, a grab that finds nothing (or declines fragments) is
  // indistinguishable from a broken button.
  const total = Object.values(sources).reduce((a, b) => a + b, 0);
  let note;
  if (urls.length === 0) {
    if (total === 0 && unusablePlayers > 0 && blobPlayers > 0) {
      // A player is there. Saying "no media" here is what made an
      // actively playing MSE page look broken.
      note =
        `Found ${players} media element(s) on this page, but their source ` +
        "is a blob: URL (MSE) and the underlying request is not observable " +
        "from here yet — press play, let it buffer for a few seconds, then " +
        "grab again.";
    } else if (total === 0 && unusablePlayers > 0) {
      note =
        `Found ${players} media element(s) on this page, but none of them ` +
        "resolved to a URL and no media request is observable from here — " +
        "press play, let it buffer, then grab again.";
    } else if (total === 0) {
      note =
        "No media found in this page's DOM or network activity. Media " +
        "loaded inside a cross-origin iframe or fetched by a service " +
        "worker can't be seen from here.";
    } else {
      note =
        `Found ${total} media request(s) but none of them is a direct ` +
        "file, stream pair, or manifest a download manager can use.";
    }
  } else {
    const parts = [];
    if (candidates[0].kind === "pair") {
      parts.push(
        "video + audio pair — DM downloads both halves and merges them " +
          "into one file (needs ffmpeg)",
      );
    } else if (candidates[0].kind === "file") {
      parts.push(
        candidates.length === 2
          ? "video + audio files (merged on completion)"
          : "media file",
      );
    } else if (candidates[0].kind === "manifest") {
      parts.push(
        "HLS/DASH manifest — DM downloads the playlist file itself; " +
          "expanding it into segments is not supported yet",
      );
    } else {
      parts.push("single DASH segment (no complete file found on the page)");
    }
    const dropped = ordered.length - urls.length;
    if (dropped > 0) {
      parts.push(`${dropped} other media request(s) not offered`);
    }
    if (urls.some((u) => isP2pUrl(u))) {
      parts.push(
        "warning: a P2P edge (mcdn/pcdn) copy could not be avoided — " +
          "if the download is refused, let the video play a bit and grab again",
      );
    }
    note = `${urls.length} URL(s) selected — ${parts.join("; ")}.`;
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
    // Page metadata for the merged file (title / date / uploader).
    meta: collectPageMeta(),
    // Pairing intent: when true, the app must treat `urls` as ONE
    // logical download (two halves of the same stream) no matter which
    // mirrors they come from — bilibili hands video and audio from
    // different `upos-sz-*` hosts, and a same-origin pairing check
    // silently un-paired such grabs.
    pair: candidates.length > 0 && candidates[0].kind === "pair",
    note,
    // What the scan actually saw, so the popup can distinguish "this
    // page has no media" from "it has media I could not name".
    diag: {
      frames: docs.length,
      players,
      blobPlayers,
      unusablePlayers,
      network: networkSeen,
      hints: Array.isArray(hints) ? hints.length : 0,
      sources,
    },
    // Raw findings, newest-discovery first. Consumed by the play-info
    // enhancer below (observed-path matching) and stripped from the
    // payload before the reply is sent.
    ordered,
  };
}

// ── The page's own play info (bilibili playurl) ─────────────────────
//
// The URL heuristics can only see what the resource buffer exposes —
// and it is finite. On a real bilibili watch page this is exactly how a
// grab ended up offering the AUDIO half alone: the video half's
// requests were no longer in the buffer. The page also calls its
// **playurl API**, whose JSON names every stream explicitly
// (`dash.video[]` / `dash.audio[]`, signed, on a normal CDN).
// Re-fetching that same request (same-origin; the player does it too)
// turns a half-observed grab into a complete pair, and replaces P2P
// edge URLs with normal-CDN mirrors when the API offers them.

function isPlayInfoUrl(name) {
  try {
    const u = new URL(name);
    return /(^|\.)bilibili\.com$/i.test(u.hostname) && /playurl/i.test(u.pathname);
  } catch (_e) {
    return false;
  }
}

/**
 * Read a page-defined global from the content script's isolated world.
 *
 * `window.__playinfo__` is set by the page's own script, so in Chrome's
 * isolated world it is simply absent — the earlier code (and the
 * harness, which loads both scripts into one context) assumed it was
 * visible, which made this source silently dead on every real page.
 * Firefox exposes page expando properties through `wrappedJSObject`;
 * Chrome has no equivalent without a MAIN-world injection, so there
 * the call returns null and the playurl replay below is the only path.
 */
function pageGlobal(name) {
  try {
    if (typeof window === "undefined" || !window) return null;
    const w = window.wrappedJSObject || window;
    return w[name] || null;
  } catch (_e) {
    return null;
  }
}

function hasPlayInfoCandidate() {
  // The page's embedded initial play state counts too: bilibili defines
  // `window.__playinfo__` (the raw playurl payload) on every watch page,
  // and unlike a resource-buffer entry it can never be dropped. Only
  // readable through `wrappedJSObject` (Firefox) — see `pageGlobal`.
  if (pageGlobal("__playinfo__")) return true;
  try {
    const names = resourceNames();
    for (const name of names) {
      if (name && isPlayInfoUrl(name)) return true;
    }
  } catch (_e) {
    // Fall through.
  }
  return false;
}

/**
 * All play-info request URLs in the resource buffer, newest first.
 * Playlist/season pages accumulate one per episode (plus preloads), so
 * there can be several — and the newest is not necessarily the one
 * describing the stream the user is watching.
 */
function playInfoUrls() {
  const urls = [];
  try {
    // `resourceNames` includes our own ring, so a playurl request that
    // fell out of the browser's buffer is still re-playable.
    for (const name of resourceNames()) {
      if (name && isPlayInfoUrl(name)) urls.push(name);
    }
  } catch (_e) {
    // Fall through.
  }
  return urls;
}

async function fetchJson(url) {
  try {
    const ctrl = typeof AbortController === "function" ? new AbortController() : null;
    const timer = setTimeout(() => {
      if (ctrl) ctrl.abort();
    }, 4000);
    const res = await fetch(url, {
      credentials: "include",
      signal: ctrl ? ctrl.signal : undefined,
    });
    clearTimeout(timer);
    if (!res || !res.ok) return null;
    return await res.json();
  } catch (_e) {
    // Network error, CORS refusal, timeout, non-JSON — any of these
    // just means "no enhancement"; the heuristic result stands.
    return null;
  }
}

function pathTail(u) {
  try {
    return new URL(u).pathname.split("/").pop() || "";
  } catch (_e) {
    return "";
  }
}

/** base + backup URLs of a play-info stream entry. */
function streamUrls(entry) {
  const urls = [entry.baseUrl];
  if (Array.isArray(entry.backupUrl)) urls.push(...entry.backupUrl);
  return urls.filter(Boolean);
}

/**
 * Re-read the page's play info and, when one of its responses names the
 * streams of the episode the player is actually streaming, replace the
 * heuristic candidates with that response's clean, signed URLs.
 *
 * Sources, in order:
 *   1. `window.__playinfo__` — the playurl payload bilibili embeds in
 *      every watch page. No network, and it can never fall out of the
 *      resource buffer. (The player updates it when the episode or
 *      quality changes; the cid check below still guards staleness.)
 *   2. Up to five playurl requests still in the resource buffer,
 *      newest first — playlist/season pages hold several (preloads,
 *      previously watched parts) and the newest need not match what's
 *      playing.
 *
 * The observed segment requests know which episode is playing: their
 * paths embed the cid (`{cid}-1-{codec}.m4s`), and the newest observed
 * m4s belongs to the stream being fetched right now. A response is used
 * only when its dash contains that cid AND both halves; anything else
 * is skipped and recorded as a diagnostic, which lands in the grab note
 * so a half-offer is never mysterious.
 */
async function enhanceWithPlayInfo(base, ordered) {
  const sources = [];
  const embedded = pageGlobal("__playinfo__");
  if (embedded) sources.push(embedded);
  for (const u of playInfoUrls().slice(0, 5)) sources.push(u);

  // The newest observed m4s names the playing episode's cid. (ordered is
  // newest-discovery-first.) The dominant cid (most requested) wins over
  // pure recency — hover previews / promo clips fetch a handful of
  // segments very recently, but the watched video out-requests them.
  const observedSet = new Set();
  for (const u of ordered) {
    if (!/\.m4s(\?|#|$)/i.test(u)) continue;
    const tail = pathTail(u);
    if (!tail) continue;
    observedSet.add(tail);
  }
  const currentCid = dominantCid();

  let diag =
    sources.length === 0 ? "no play-info request seen in this tab" : null;

  for (const src of sources) {
    let data = null;
    if (typeof src === "string") {
      const info = await fetchJson(src);
      if (info && typeof info === "object") data = info.data || info.result || info;
      if (!data) {
        diag = "play-info fetch failed";
        continue;
      }
    } else {
      // Embedded payloads appear as the raw playurl body (`{dash, …}`),
      // older shapes wrap it in `data`/`result` — accept all three.
      data = src && (src.data || src.result || src);
      if (!data) {
        diag = "embedded play info unusable";
        continue;
      }
    }

    const dash = data.dash;
    if (
      dash &&
      Array.isArray(dash.video) &&
      dash.video.length > 0
    ) {
      // Restrict to the playing episode: a playurl entry belongs to it
      // when any of its URLs' path starts with `{cid}-`.
      const inEpisode = (list) =>
        list.filter((v) =>
          streamUrls(v).some(
            (u) => currentCid === null || pathTail(u).startsWith(currentCid + "-"),
          ),
        );
      const episodeVideos = inEpisode(dash.video);
      const episodeAudios = Array.isArray(dash.audio)
        ? inEpisode(dash.audio)
        : [];
      // A response with only one half can't describe a full download —
      // skip it and try the next source. (Bilibili previews and some
      // special responses carry video without an audio track.)
      if (episodeVideos.length === 0 || episodeAudios.length === 0) {
        diag = `play info has no audio track for stream ${currentCid ?? "?"}`;
        continue;
      }

      const pick = (list) => {
        const matched = list.find((v) =>
          streamUrls(v).some((u) => observedSet.has(pathTail(u))),
        );
        const entry = matched || list[0];
        if (!entry) return null;
        // A P2P edge serves only byte ranges; prefer a normal CDN mirror
        // (baseUrl or any backupUrl) whenever one exists.
        const urls = streamUrls(entry);
        return urls.find((u) => !isP2pUrl(u)) || urls[0];
      };
      const video = pick(episodeVideos);
      const audio = pick(episodeAudios);
      if (video && audio) {
        const p2p = [video, audio].some((u) => isP2pUrl(u));
        return {
          ...base,
          urls: [video, audio],
          note:
            `2 URL(s) selected — video + audio pair from the page's play info ` +
            `(stream ${currentCid ?? "best"}, the page's own signed CDN URLs, ` +
            `not P2P edges) — DM downloads both halves and merges them into ` +
            `one file (needs ffmpeg).` +
            (p2p
              ? " warning: only a P2P edge copy was available; if the download is refused, let the video play a bit and grab again."
              : ""),
        };
      }
      diag = "play info entries unusable";
      continue;
    }

    // Legacy (non-DASH) play pages expose `durl`: direct file URLs. Only
    // meaningful when the page showed no segment traffic at all (a fresh
    // page) — otherwise the heuristic result describes the playing
    // stream better than an episode's durl would.
    if (
      observedSet.size === 0 &&
      Array.isArray(data.durl) &&
      data.durl.length > 0 &&
      data.durl[0] &&
      data.durl[0].url
    ) {
      return {
        ...base,
        urls: [data.durl[0].url],
        note: `1 URL(s) selected — direct media file from the page's play info.`,
      };
    }
    diag = "play info response had no dash/durl";
  }

  // No source produced a pair: keep the heuristic result, but say WHY —
  // a half-offer ("video only") must never be mysterious.
  if (diag) base.note += ` (play info: ${diag})`;
  return base;
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

// Resource observation starts regardless of the page's protocol: the
// content script is injected at `document_start` precisely so this runs
// before the page's own load burst fills the browser's ~250-entry
// buffer. `setResourceTimingBufferSize` only raises the cap — once full,
// the browser stops recording *new* entries — so the observer mirrors
// every entry into our ring and keeps the live buffer empty.
startResourceObserver();
try {
  if (
    typeof performance !== "undefined" &&
    typeof performance.setResourceTimingBufferSize === "function"
  ) {
    performance.setResourceTimingBufferSize(1024);
  }
} catch (_e) {
  // Non-fatal; the grab works with whatever the buffer holds.
}

// Respond to explicit "collect" messages from the popup / background,
// and accept state pushes from the background.
// We do **not** call `collectMedia()` ourselves and we do **not** start
// a MutationObserver — see the top-of-file comment for the rationale.
//
// **Only the top frame answers `collect`.** The message is delivered to
// every frame in the tab, and the browser keeps just the *first*
// response. A content script inside an embed iframe would otherwise
// answer with its own `location.href` as `pageUrl` — that is how an
// embed URL like `player.html` used to end up in the grab payload — and
// could also win the race with an empty result, masking the media the
// top frame had found. Same-origin iframe DOMs are still scanned (from
// the top frame, inside `collectMedia`), so nothing discoverable is
// lost by making the response deterministic.
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (!msg) return;
  if (msg.type === "collect") {
    // `typeof window === "undefined"` covers the test harness (and any
    // exotic context without a window object); in a real content script
    // `window` always exists. Reading `window.top` itself is allowed
    // even across origins, so the comparison cannot throw.
    const inTopFrame =
      typeof window === "undefined" || window.top === window;
    if (!inTopFrame) return;
    // `hints` carries the background's `webRequest` log — the only
    // source that sees media inside cross-origin iframes, MSE traffic
    // with extensionless URLs, and requests the resource buffer already
    // dropped. It is advisory: the curation below still decides.
    const base = collectMedia(msg.hints);
    // When the page exposed its play-info API (bilibili) and fetch is
    // available, upgrade the heuristic result asynchronously: the API
    // names BOTH streams explicitly, which the finite resource buffer
    // may not. Everything else answers synchronously.
    if (typeof fetch === "function" && hasPlayInfoCandidate()) {
      enhanceWithPlayInfo(base, base.ordered || [])
        .then((enhanced) => {
          delete enhanced.ordered;
          sendResponse(enhanced);
        })
        .catch(() => {
          delete base.ordered;
          sendResponse(base);
        });
      return true; // keep the message channel open for the async reply
    }
    delete base.ordered;
    sendResponse(base);
    return;
  }
  if (msg.type === "state") {
    applyState(msg.state);
  }
});
