// MV3 service worker for the DM Download Grabber extension.
// Connects to the native-messaging host (`com.app.dm.native`) and forwards
// detected media/download URLs to the running DM app.
//
// Connection model:
//   * `connect()` is called on every send (the host terminates the connection
//     after a short idle, so we reconnect on demand).
//   * Failures (host not running, manifest missing, etc.) surface to the
//     user via `chrome.action.setBadgeText` so a broken install is obvious.

const HOST_NAME = "com.app.dm.native";
const STORAGE_KEY_INSTALL_TIME = "dmInstallTime";

// Connection state — visible to the popup via `chrome.runtime.sendMessage`.
let hostState = {
  connected: false,
  lastError: null,
  lastSentAt: 0,
  lastSentCount: 0,
  // Live `chrome.downloads.onCreated` filter: epoch ms. Loaded from
  // `chrome.storage.local` on boot so the filter survives service-worker
  // restarts and extension reloads. Updated once, on the very first
  // `onInstalled` event.
  //
  // The **synchronous** `Date.now()` default is important: it means
  // even if `onCreated` fires before `loadInstallTime()` resolves
  // (e.g. during the very first service-worker boot, before storage
  // is read), the filter is still armed with a sane cutoff. The
  // listener itself is only registered *after* `loadInstallTime()`
  // has resolved, so the race window is closed in practice — but the
  // default here is the safety net.
  installTimeMs: Date.now(),
};

// `onDownloadsCreatedRegistered` flips to true once the boot sequence
// has loaded the persisted install time and wired up the
// `chrome.downloads.onCreated` listener. Until then, any download
// events are intentionally ignored — see the boot sequence at the
// bottom of this file. (We don't want to forward anything that
// fired during the "we don't know when the user installed us" race
// window.)
let onDownloadsCreatedRegistered = false;

function setBadge(state) {
  const text = state === "ok" ? "" : "!";
  const color = state === "ok" ? "#74e0a4" : "#ff7a8f";
  try {
    chrome.action.setBadgeText({ text });
    chrome.action.setBadgeBackgroundColor({ color });
  } catch (e) {
    /* not available in some test harnesses */
  }
}

let port = null;

function connect() {
  if (port) return port;
  try {
    port = chrome.runtime.connectNative(HOST_NAME);
    hostState.connected = true;
    hostState.lastError = null;
    setBadge("ok");
    port.onMessage.addListener(() => {
      // The host may ack; nothing to act on.
    });
    port.onDisconnect.addListener(() => {
      const err = chrome.runtime.lastError
        ? chrome.runtime.lastError.message
        : "host disconnected";
      hostState.connected = false;
      hostState.lastError = err;
      port = null;
      setBadge("err");
      console.warn("DM native host disconnected:", err);
    });
  } catch (e) {
    hostState.connected = false;
    hostState.lastError = String(e);
    setBadge("err");
    console.warn("DM native host connect failed:", e);
  }
  return port;
}

function send(payload) {
  const p = connect();
  if (!p) return false;
  try {
    p.postMessage(payload);
    hostState.lastSentAt = Date.now();
    hostState.lastSentCount++;
    return true;
  } catch (e) {
    hostState.connected = false;
    hostState.lastError = String(e);
    setBadge("err");
    try { p.disconnect(); } catch (_) {}
    port = null;
    return false;
  }
}

// Parse Chrome's `DownloadItem.startTime` (ISO 8601 string) → epoch ms.
// Returns 0 for unparseable / missing timestamps so callers can treat
// that as "drop, not enough info".
function startTimeToMs(s) {
  if (!s) return 0;
  const t = Date.parse(s);
  return Number.isFinite(t) ? t : 0;
}

// Decide whether a `DownloadItem` should be forwarded to DM. Drops:
//   - items whose `startTime` is missing or unparseable (defensive);
//   - items that started before the extension was installed
//     (the documented re-fire on install that would otherwise flood
//     DM with the user's entire Chrome download history).
function shouldForwardDownload(item) {
  if (!item || !item.url) return false;
  const t = startTimeToMs(item.startTime);
  if (t === 0) {
    // No timestamp → can't tell if it's historical. Be conservative
    // and drop it. The user can re-download manually if needed.
    return false;
  }
  return t >= hostState.installTimeMs;
}

// Forward real browser downloads ("Save link as", etc.) to DM — but
// only those that started **after** the extension was installed.
//
// The listener is registered **only after** `loadInstallTime()`
// resolves, so we never accidentally forward an `onCreated` event
// while `hostState.installTimeMs` is still in its synchronous
// `Date.now()` bootstrap default (which would correctly filter
// events that fire after we initialise but is *intended* to be
// overwritten by the persisted install time). See the boot
// sequence at the bottom of this file.
function registerDownloadsListener() {
  if (onDownloadsCreatedRegistered) return;
  chrome.downloads.onCreated.addListener((item) => {
    if (!shouldForwardDownload(item)) return;
    send({ type: "download", url: item.url, filename: item.filename });
  });
  onDownloadsCreatedRegistered = true;
}

// Messages from content scripts / popup.
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (!msg) return;
  if (msg.type === "get-status") {
    sendResponse({ ok: true, state: hostState });
    return;
  }
  // The supported capture path is `type === "grab"` (popup button
  // → background → content-script `collect` → explicit response).
  // We **deliberately do not** accept a `type === "media"` message
  // — the legacy auto-scrape path that used to spam DM on every
  // page load / DOM mutation has been removed in both content.js
  // and here, so any third-party content script (or a stale cached
  // copy of our own) that still sends a `media` message is dropped
  // on the floor. This is a hard boundary: a download manager must
  // not be a passive capture-everything tap on the browser.
  if (msg.type === "grab") {
    chrome.tabs.sendMessage(msg.tabId, { type: "collect" }, (res) => {
      if (res && res.urls && res.urls.length) {
        send({ type: "capture", url: res.pageUrl, urls: res.urls });
      }
    });
  } else if (msg.type === "open") {
    send({ type: "open" });
  }
  if (sendResponse) sendResponse({ ok: true });
});

/**
 * Read the persisted install time (set on the very first `onInstalled`
 * event) and cache it in `hostState.installTimeMs`. If storage is
 * empty — which only happens on a brand-new install before
 * `onInstalled` has fired — we use `Date.now()` as a safe default;
 * any download that fires before storage is written is a Chrome-side
 * re-fire and should be dropped.
 */
async function loadInstallTime() {
  try {
    const got = await chrome.storage.local.get(STORAGE_KEY_INSTALL_TIME);
    const v = got && got[STORAGE_KEY_INSTALL_TIME];
    if (typeof v === "number" && v > 0) {
      hostState.installTimeMs = v;
    } else {
      hostState.installTimeMs = Date.now();
    }
  } catch (e) {
    hostState.installTimeMs = Date.now();
  }
}

/**
 * Persist the install time. Only called on the **first** `onInstalled`
 * event (i.e. when no install time is already stored). On extension
 * updates we leave the existing value alone — we want to keep
 * filtering pre-update downloads.
 */
async function maybePersistInstallTime() {
  try {
    const got = await chrome.storage.local.get(STORAGE_KEY_INSTALL_TIME);
    const v = got && got[STORAGE_KEY_INSTALL_TIME];
    if (typeof v !== "number" || v <= 0) {
      const now = Date.now();
      await chrome.storage.local.set({ [STORAGE_KEY_INSTALL_TIME]: now });
      hostState.installTimeMs = now;
    } else {
      hostState.installTimeMs = v;
    }
  } catch (e) {
    // Storage failed — fall back to "now" so we don't drop
    // brand-new downloads. (Worse than the persisted case, but
    // strictly better than denying all downloads.)
    hostState.installTimeMs = Date.now();
  }
}

chrome.runtime.onInstalled.addListener(async (details) => {
  // Only persist on the **first** install, not on update.
  // `details.reason` is `"install"` for a fresh install, `"update"`
  // for an upgrade, and `"chrome_update"` for a Chrome-level browser
  // upgrade. We only write on `"install"`.
  if (details && details.reason === "install") {
    await maybePersistInstallTime();
  } else {
    await loadInstallTime();
  }
  // Now that we know the install time, register the
  // `chrome.downloads.onCreated` listener. Any download events that
  // fired before this point (during the very first service-worker
  // boot, before storage was read) are intentionally dropped — they
  // are by definition pre-install and would have been filtered
  // anyway. Skipping them is the race-free correct behaviour.
  registerDownloadsListener();
  connect();
});

chrome.runtime.onStartup.addListener(async () => {
  await loadInstallTime();
  registerDownloadsListener();
  connect();
});

// Boot: load the persisted install time, then register the
// `onCreated` listener, then connect to the host. The order is
// important: any `onCreated` events that fire before
// `loadInstallTime()` resolves are **not** forwarded. That means
// in-progress / interrupted downloads that Chrome re-fires during
// extension load are dropped (correct — they predate the
// install) rather than being forwarded under a default install
// time of 0 (which would let everything through).
loadInstallTime()
  .then(registerDownloadsListener)
  .then(connect);
