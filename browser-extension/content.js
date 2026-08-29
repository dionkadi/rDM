// Content script: scan the page for media/download URLs and report them to the
// background service worker, which forwards them to the DM native host.

const MEDIA_RE =
  /\.(m3u8|mp4|webm|mkv|mov|flv|avi|ts|m4v|ogg|mp3|m4a)(\?|#|$)/i;

function collectMedia() {
  const urls = new Set();
  document.querySelectorAll("video, source, audio").forEach((el) => {
    if (el.src) urls.add(el.src);
  });
  document.querySelectorAll("a").forEach((el) => {
    if (el.href && MEDIA_RE.test(el.href)) urls.add(el.href);
  });
  // Common HLS/DASH manifest links that don't end in a media extension.
  document
    .querySelectorAll('a[href*=".m3u8"], a[href*="manifest"]')
    .forEach((el) => {
      if (el.href) urls.add(el.href);
    });
  return { type: "media", pageUrl: location.href, urls: Array.from(urls) };
}

const payload = collectMedia();
if (payload.urls.length) chrome.runtime.sendMessage(payload);

chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg && msg.type === "collect") {
    sendResponse(collectMedia());
  }
});
