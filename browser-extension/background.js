// MV3 service worker for the DM Download Grabber extension.
//
// Two transports, picked at module load:
//   * **WebSocket** to `ws://127.0.0.1:9157/` — used by Chromium
//     browsers (Chrome, Edge, Brave, Arc, Vivaldi). This is the
//     "load unpacked and it just works" path: no host manifest,
//     no native binary, no per-OS install. The Tauri app already
//     binds that port for the legacy native host, so the
//     extension piggybacks on the same socket.
//   * **Native messaging** to `com.app.dm.native` — used by
//     Firefox MV3. Firefox upgrades insecure `ws://` requests to
//     `wss://` and silently fails, so a direct WebSocket to
//     localhost is not viable there. The native-messaging host
//     (`crates/native-host`) bridges stdio JSON to the same
//     `127.0.0.1:9157` socket.
//
// Both transports share the same on-the-wire payload
// (`{"url":"…","type":"…","referer":"…","userAgent":"…"}`)
// and update the same `hostState` object that the popup polls.
//
// Connection model (per transport):
//   * WebSocket: persistent, with exponential-backoff
//     reconnect on close. Messages sent while disconnected are
//     queued (capped at 64 entries to bound memory).
//   * Native messaging: opened on demand; the host closes the
//     port after a short idle, so we reconnect per send. This
//     matches the original pre-WebSocket behaviour.

const HOST_NAME = "com.app.dm.native";
const WS_URL = "ws://127.0.0.1:9157/";
const WS_RECONNECT_BACKOFF_MS = [200, 500, 1000, 2000, 5000];
const WS_QUEUE_MAX = 64;
const STORAGE_KEY_INSTALL_TIME = "dmInstallTime";

// Connection state — visible to the popup via `chrome.runtime.sendMessage`.
const hostState = {
  connected: false,
  lastError: null,
  lastSentAt: 0,
  lastSentCount: 0,
  // `transportKind` is `"websocket"` or `"native"`. The popup
  // surfaces this so the user can see which path is in use
  // (and the install docs can link to the right section).
  transportKind: null,
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

// One funnel for "the transport is not reachable" so the popup,
// the badge, and the console stay in sync. We deliberately do
// NOT include the full stack — `lastError` strings are
// already user-readable.
function setState(connected, message) {
  hostState.connected = !!connected;
  hostState.lastError = message || null;
  setBadge(connected ? "ok" : "err");
  if (!connected && message) {
    console.warn("DM transport:", message);
  }
}

// ─── WebSocket transport (Chromium browsers) ────────────────────
//
// Wraps the standard browser `WebSocket` with reconnect, queueing,
// and the hostState integration. We don't pull in any libraries:
// the WebSocket API is built in to MV3 service workers, and a
// few hundred lines of state machine is enough.
function makeWebSocketTransport() {
  hostState.transportKind = "websocket";
  let ws = null;
  let queue = [];
  let backoffIdx = 0;
  let closing = false;
  let onConnected = null; // test hook for `forceReconnect`

  function connect() {
    if (ws || closing) return;
    try {
      ws = new WebSocket(WS_URL);
    } catch (e) {
      setState(false, "WebSocket ctor failed: " + (e && e.message ? e.message : e));
      scheduleReconnect();
      return;
    }
    ws.addEventListener("open", () => {
      backoffIdx = 0;
      setState(true, null);
      // Drain anything queued while we were disconnected.
      const pending = queue;
      queue = [];
      for (const payload of pending) {
        try {
          ws.send(JSON.stringify(payload));
          hostState.lastSentAt = Date.now();
          hostState.lastSentCount++;
        } catch (e) {
          // Re-queue if the socket died mid-drain.
          queue.push(payload);
          break;
        }
      }
      if (typeof onConnected === "function") {
        const cb = onConnected;
        onConnected = null;
        cb();
      }
    });
    ws.addEventListener("message", () => {
      // The Tauri side acks every payload with `{"ok":true}`.
      // We don't need the ack for correctness (the next send
      // would fail loudly if the socket were broken), but we
      // keep the listener wired so Chrome's WebSocket impl
      // doesn't fill an internal buffer.
    });
    ws.addEventListener("close", (ev) => {
      ws = null;
      setState(false, `disconnected (code ${ev.code})`);
      if (!closing) scheduleReconnect();
    });
    ws.addEventListener("error", () => {
      // The `error` event fires just before `close`. Chrome
      // does not populate a useful message here, so we let
      // `close` set the user-facing error string.
    });
  }

  function scheduleReconnect() {
    if (closing) return;
    const wait = WS_RECONNECT_BACKOFF_MS[Math.min(backoffIdx, WS_RECONNECT_BACKOFF_MS.length - 1)];
    backoffIdx++;
    setTimeout(connect, wait);
  }

  return {
    send(payload) {
      if (ws && ws.readyState === WebSocket.OPEN) {
        try {
          ws.send(JSON.stringify(payload));
          hostState.lastSentAt = Date.now();
          hostState.lastSentCount++;
          return true;
        } catch (e) {
          // Fall through to the queue path below.
        }
      }
      // No live socket (or the send threw): queue and ensure a
      // connection attempt is in flight. Cap the queue so a
      // runaway tab can't OOM the SW — we drop oldest first,
      // which is the right call (the user clicked the most
      // recent link).
      if (queue.length >= WS_QUEUE_MAX) queue.shift();
      queue.push(payload);
      if (!ws) connect();
      return false;
    },
    disconnect() {
      closing = true;
      if (ws) {
        try {
          ws.close();
        } catch (e) {
          void e;
        }
        ws = null;
      }
      queue = [];
    },
    forceReconnect(onDone) {
      onConnected = onDone || null;
      if (ws) {
        try {
          ws.close();
        } catch (e) {
          void e;
        }
        ws = null;
      }
      closing = false;
      // Reset backoff so the popup's "Test" button gives the
      // server a fast first attempt.
      backoffIdx = 0;
      connect();
    },
  };
}

// ─── Native-messaging transport (Firefox MV3) ──────────────────
//
// Kept as a faithful copy of the original Chrome / Firefox
// connectNative path. Firefox MV3 cannot reliably open a
// `ws://127.0.0.1` connection (Firefox upgrades insecure
// ws:// requests to wss:// and fails), so Firefox users still
// install the `dm-native-host` binary and register the host
// manifest.
function makeNativeTransport() {
  hostState.transportKind = "native";
  let port = null;

  function connect() {
    if (port) return port;
    let newPort = null;
    try {
      newPort = chrome.runtime.connectNative(HOST_NAME);
    } catch (e) {
      setState(false, String(e && e.message ? e.message : e));
      return null;
    }
    port = newPort;
    // **CRITICAL (Firefox MV3):** `connectNative` is async —
    // when the host lookup fails, Firefox reports the error
    // via `chrome.runtime.lastError` *and* via
    // `port.onDisconnect` on the next event-loop tick. If we
    // don't read `lastError` in the same tick (before any
    // other chrome.* call), it gets cleared and we lose the
    // real error message. So we optimistically mark the
    // connection as up, then check `lastError` immediately
    // and roll back if it's set.
    if (chrome.runtime.lastError) {
      setState(false, chrome.runtime.lastError.message);
      try {
        port.disconnect();
      } catch (e) {
        void e;
      }
      port = null;
      return null;
    }
    setState(true, null);
    port.onMessage.addListener(() => {
      // The host may ack; nothing to act on.
    });
    port.onDisconnect.addListener(() => {
      const err = chrome.runtime.lastError
        ? chrome.runtime.lastError.message
        : "host disconnected";
      setState(false, err);
      port = null;
    });
    return port;
  }

  return {
    send(payload) {
      const p = connect();
      if (!p) return false;
      try {
        p.postMessage(payload);
        hostState.lastSentAt = Date.now();
        hostState.lastSentCount++;
        return true;
      } catch (e) {
        setState(false, String(e && e.message ? e.message : e));
        try {
          p.disconnect();
        } catch (innerErr) {
          void innerErr;
        }
        port = null;
        return false;
      }
    },
    disconnect() {
      if (port) {
        try {
          port.disconnect();
        } catch (e) {
          void e;
        }
        port = null;
      }
    },
    forceReconnect(onDone) {
      if (port) {
        try {
          port.disconnect();
        } catch (e) {
          void e;
        }
        port = null;
      }
      setState(false, null);
      connect();
      // Drain a tick so Firefox's async disconnect can run.
      setTimeout(() => {
        if (typeof onDone === "function") onDone();
      }, 50);
    },
  };
}

// Pick the transport at module load. We use WebSocket for
// Chromium-based browsers and the native-messaging host for
// Firefox. The detection is intentionally conservative: only
// pick native if `browser.runtime.getBrowserInfo` exists,
// which is Firefox-specific.
const isFirefox =
  typeof browser !== "undefined" &&
  typeof browser.runtime !== "undefined" &&
  typeof browser.runtime.getBrowserInfo === "function";
const transport = isFirefox ? makeNativeTransport() : makeWebSocketTransport();

// Thin wrappers around the transport. The rest of this file
// uses `send()` and `connect()`; the underlying transport is
// an implementation detail.
function send(payload) {
  return transport.send(payload);
}
function connect() {
  if (typeof transport.connect === "function") {
    transport.connect();
  } else {
    // WebSocket transport auto-connects on first send;
    // for the boot sequence we just call forceReconnect to
    // make sure the first connection attempt is in flight.
    transport.forceReconnect();
  }
}

/**
 * Force a fresh connection attempt. Used by the popup's
 * "Test host" button. Resolves with a `{ok, lastError,
 * transportKind}` summary that the popup can render.
 */
async function probeHost() {
  return new Promise((resolve) => {
    transport.forceReconnect(() => {
      resolve({
        ok: hostState.connected,
        lastError: hostState.lastError,
        transportKind: hostState.transportKind,
        hostName: isFirefox ? HOST_NAME : null,
        url: isFirefox ? null : WS_URL,
      });
    });
    // Safety net: if neither transport calls us back within
    // 1.5 s, return the current state anyway. WebSocket
    // events fire async, and the "Test" button should never
    // hang the popup.
    setTimeout(() => {
      resolve({
        ok: hostState.connected,
        lastError: hostState.lastError,
        transportKind: hostState.transportKind,
        hostName: isFirefox ? HOST_NAME : null,
        url: isFirefox ? null : WS_URL,
      });
    }, 1500);
  });
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
    // Forward the URL to DM. We deliberately **also** cancel Chrome's
    // own download: the goal of the extension is to take over the
    // download, not to capture *and* let Chrome also download. If
    // both proceed the upstream (especially a single-connection
    // CDN mirror) will sometimes truncate one of them, leaving DM
    // with a partial body that the engine then wrongly marks
    // Completed. Cancelling here means Chrome drops its connection,
    // DM is the only one talking to the mirror, and the body stream
    // is the real file.
    send({ type: "download", url: item.url, filename: item.filename });
    // `item.id` is Chrome's internal numeric id for the download.
    // We pass it through the native host so the *Rust* side (or
    // the eventual `chrome.downloads.cancel` call below) can target
    // the right entry. The cancel itself runs in this service
    // worker — we have the `chrome.downloads` permission here.
    //
    // We do the cancel + erase **after** the `send` so the
    // cancellation can't race ahead of the URL reaching the host.
    // The two operations are independent (the host doesn't need to
    // ack the URL for us to cancel Chrome), so we don't await.
    if (item.id != null) {
      takeOverChromeDownload(item);
    }
  });
  onDownloadsCreatedRegistered = true;
}

/**
 * Cancel Chrome's own download of `item` and remove the entry from
 * Chrome's download list (so the user doesn't see a phantom "interrupted"
 * item in `chrome://downloads` next to the successful DM transfer).
 *
 * The cancel is best-effort: if `chrome.downloads.cancel` rejects
 * (the entry is already gone, or Chrome's UI is in a state that
 * disallows cancel), we log and move on. The same is true of
 * `chrome.downloads.erase` — best-effort cleanup.
 *
 * We use `removeFromDisk: false` because the file Chrome was writing
 * is going to be a partial / truncated file (the race we just
 * described). We don't want DM's download to fail because Chrome
 * left a stale partial on disk; and we don't want the user to see
 * "this file is 879 B" in their Downloads folder.
 */
function takeOverChromeDownload(item) {
  const id = item.id;
  if (id == null) return;
  try {
    chrome.downloads.cancel(id, () => {
      // Whether the cancel succeeded or not (the callback fires
      // for both), we want to erase the entry. `removeFromDisk:
      // false` because the partial file Chrome wrote is not the
      // real file and we don't want to keep it around.
      try {
        chrome.downloads.erase({ id, removeFromDisk: false }, () => {
          // Last error is expected if the entry was already
          // gone (e.g. user erased it manually, or the cancel
          // cleaned it up). Silent.
          if (chrome.runtime.lastError) {
            // Reference to suppress unused-var lint; the message
            // is intentionally not surfaced to the user.
            void chrome.runtime.lastError.message;
          }
        });
      } catch (e) {
        // Best-effort cleanup. Any failure here is non-fatal.
        void e;
      }
    });
  } catch (e) {
    // The cancel API can throw if the download is in a state that
    // disallows cancellation (e.g. "complete"). The DM copy is
    // already in flight; nothing else to do.
    console.warn("DM Grabber: chrome.downloads.cancel failed:", e);
  }
}

// Messages from content scripts / popup.
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (!msg) return;
  if (msg.type === "get-status") {
    sendResponse({ ok: true, state: hostState });
    return;
  }
  // The popup's "Test host" button. Forces a fresh connectNative
  // attempt and reports the actual result — including the precise
  // `chrome.runtime.lastError.message` from Firefox/Chrome. Returns
  // a Promise (via `sendResponse` after the async `probeHost()`
  // resolves) so the popup can render the error verbatim.
  if (msg.type === "probe") {
    probeHost().then((res) => sendResponse({ ok: true, probe: res }));
    return true; // tell the browser we'll call sendResponse async
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
  } else if (msg.type === "download-click") {
    // **Primary takeover path.** The content script has already
    // synchronously suppressed the browser's default "start a
    // download" behaviour in its capture-phase click listener.
    // Forward the URL to DM via the native host. The browser will
    // not create a download entry because we never let the click
    // reach the browser's own anchor-click handler.
    if (msg.url) {
      send({
        type: "download",
        url: msg.url,
        filename: msg.filename || "",
      });
    }
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
    // Storage failed — fall back to "now" so we don't drop
    // brand-new downloads. Logged for diagnostic visibility.
    console.warn("DM Grabber: loadInstallTime failed:", e);
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
    console.warn("DM Grabber: maybePersistInstallTime failed:", e);
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
loadInstallTime().then(registerDownloadsListener).then(connect);
