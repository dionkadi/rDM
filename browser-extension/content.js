// Content script: scan the page for media/download URLs **on demand**
// only. We deliberately do NOT auto-scrape on every page load or every DOM
// mutation — that would flood the user's DM queue with media URLs from
// every page they visit (and from any re-render of those pages), which
// is the opposite of "you wanted a download manager, not a passive
// capture-everything tap on the browser". The popup's "Grab page media"
// button explicitly asks the content script to scan via a `collect`
// message, and only that path is wired up.
const MEDIA_RE =
  /\.(m3u8|mp4|webm|mkv|mov|flv|avi|ts|m4v|ogg|mp3|m4a|wav|flac|zip|7z|rar|pdf|iso|exe)(\?|#|$)/i;

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
    .querySelectorAll('meta[property="og:video"], meta[name="twitter:player:stream"]')
    .forEach((el) => {
      const c = el.getAttribute("content");
      if (c && /^https?:/i.test(c)) urls.add(c);
    });
  return { type: "media", pageUrl: location.href, urls: Array.from(urls) };
}

// Respond to explicit "collect" messages from the popup / background.
// We do **not** call `collectMedia()` ourselves and we do **not** start
// a MutationObserver — the previous behaviour was: every page load, the
// content script auto-sent every media URL on the page; every DOM
// change, it re-scanned. For pages with embedded videos, ads, or HLS
// players that swap sources, that meant the user came back to DM with
// dozens of queued downloads they never asked for. Now the only path
// that produces a `sendMessage` is the explicit "Grab page media" click
// in the popup.
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg && msg.type === "collect") {
    sendResponse(collectMedia());
  }
});
