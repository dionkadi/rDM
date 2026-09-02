// Content script: two responsibilities, both opt-in.
//
// 1. **Click interception** — when the user left-clicks a link to a
//    downloadable URL (e.g. `<a href="archive.zip">`, `<a download>`,
//    or anything matching the `MEDIA_RE` heuristic), we suppress
//    Firefox's default "start a download" behaviour, then forward
//    the URL to the background so it can be pushed to DM. The result:
//    Firefox never creates a "canceled" download row in the downloads
//    library — the user only sees the DM entry.
//
//    Modifier keys (Ctrl, Cmd, Shift, Alt, Meta), middle-click, and
//    right-click are NOT intercepted — those should keep the browser's
//    normal "open in new tab" / "save as" behaviour. "Save link as"
//    from the right-click context menu will still hit the
//    background's `onCreated` safety net, which cancels + erases
//    as best it can.
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
 *   - the resolved target is not an `<a>`;
 *   - the URL is not http(s) or doesn't look downloadable.
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
  if (!anchor) return false;
  const url = anchor.href;
  if (!/^https?:/i.test(url)) return false;
  if (anchor.hasAttribute("download")) return true;
  if (MEDIA_RE.test(url)) return true;
  return false;
}

function onClickCapture(e) {
  const anchor = findAnchor(e);
  if (!shouldIntercept(e, anchor)) return;
  // **CRITICAL** — synchronously suppress the browser's default
  // "start a download" behaviour, *before* the click event
  // propagates to the page and *before* the browser's own
  // download-anchor handler runs. `capture: true` on the
  // listener (see below) plus the synchronous preventDefault is
  // what makes Firefox never create the download entry. If we
  // were a bubble-phase listener, or async, Firefox would win
  // the race and we'd be back to cancel+erase.
  e.preventDefault();
  e.stopPropagation();
  e.stopImmediatePropagation();
  // Now forward the URL to the background. This is async but
  // that's fine — the click has already been suppressed and the
  // browser will not start a download regardless of when (or
  // whether) the message arrives. The background's onMessage
  // handler turns this into a native-host send to DM.
  try {
    chrome.runtime.sendMessage({
      type: "download-click",
      url: anchor.href,
      filename: anchor.getAttribute("download") || anchor.textContent || "",
    });
  } catch (err) {
    // If the runtime is gone (e.g. the extension was reloaded
    // mid-click), the URL is lost — but Firefox's default
    // behaviour is also suppressed, so the user can just click
    // again. Better than silently letting Firefox create a
    // download we'd then have to cancel.
    console.warn("DM Grabber: sendMessage(download-click) failed:", err);
  }
}

function collectMedia() {
  const urls = new Set();
  // Same-origin media.
  document
    .querySelectorAll("video, audio, source, picture source, track")
    .forEach((el) => {
      const s = el.getAttribute("src");
      if (s && /^https?:/i.test(s)) urls.add(s);
    });
  // Anchor tags that look like downloadable media.
  document.querySelectorAll("a[href]").forEach((el) => {
    if (MEDIA_RE.test(el.href)) urls.add(el.href);
  });
  // Common HLS / DASH manifest references that don't end in a media
  // extension.
  document
    .querySelectorAll('a[href*=".m3u8"], a[href*="manifest"]')
    .forEach((el) => {
      if (el.href) urls.add(el.href);
    });
  // Open Graph and Twitter card video tags.
  document
    .querySelectorAll(
      'meta[property="og:video"], meta[name="twitter:player:stream"]',
    )
    .forEach((el) => {
      const c = el.getAttribute("content");
      if (c && /^https?:/i.test(c)) urls.add(c);
    });
  return { type: "media", pageUrl: location.href, urls: Array.from(urls) };
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
const _loc = (typeof location !== "undefined" ? location : null);
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
}

// Respond to explicit "collect" messages from the popup / background.
// We do **not** call `collectMedia()` ourselves and we do **not** start
// a MutationObserver — see the top-of-file comment for the rationale.
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg && msg.type === "collect") {
    sendResponse(collectMedia());
  }
});
