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
// and update the same `state` object that the popup polls.
//
// Connection model (per transport):
//   * WebSocket: persistent, with exponential-backoff
//     reconnect on close. Messages sent while disconnected are
//     queued (capped at 64 entries, and dropped after
//     `WS_QUEUE_TTL_MS` so a click can't come back to life as a
//     surprise dialog an hour later).
//   * Native messaging: opened on demand; the host closes the
//     port after a short idle, so we reconnect per send. This
//     matches the original pre-WebSocket behaviour.
//
// ─── The rule this file exists to enforce ───────────────────────
//
// **Nothing is ever taken away from the browser unless DM actually
// received it.** Every interception path funnels through `send()`,
// which returns `true` only when the payload reached a live
// transport, and:
//
//   * `chrome.downloads.onCreated` forwards first and only cancels
//     the browser's copy when the forward succeeded;
//   * a click that the background can't deliver is handed back to
//     the browser via `chrome.downloads.download()`;
//   * the content script is told whether DM is reachable and skips
//     interception entirely when it isn't.
//
// Interception gates on liveness. Delivery failures degrade to
// normal browser behaviour instead of silently eating a download.

const HOST_NAME = "com.app.dm.native";
const WS_URL = "ws://127.0.0.1:9157/";
const WS_RECONNECT_BACKOFF_MS = [200, 500, 1000, 2000, 5000];
const WS_QUEUE_MAX = 64;
/// How long a payload may sit in the send queue before we give up on
/// it. Bounds the "the host came back and a burst of stale clicks
/// suddenly opened dialogs" case.
const WS_QUEUE_TTL_MS = 30_000;
const STORAGE_KEY_INSTALL_TIME = "dmInstallTime";
const STORAGE_KEY_SETTINGS = "dmSettings";

/// User-facing behaviour switches, mirrored to the popup and to every
/// content script. Defaults reproduce the pre-settings behaviour
/// (both on) so nothing changes for an existing user until they
/// choose to change it.
const DEFAULT_SETTINGS = {
  /// Handle clicks on media-looking links.
  interceptOnClick: true,
  /// Take over downloads the browser starts on its own (right-click
  /// "Save link as", Ctrl-click, another extension's download, …).
  /// This is the switch that used to not exist at all.
  takeoverBrowserDownloads: true,
};

// Connection state — visible to the popup via `chrome.runtime.sendMessage`.
const state = {
  connected: false,
  lastError: null,
  lastSentAt: 0,
  lastSentCount: 0,
  // `transportKind` is `"websocket"` or `"native"`. The popup
  // surfaces this so the user can see which path is in use
  // (and the install docs can link to the right section).
  transportKind: null,
  /// Epoch ms of the first install, or **0 while unknown**.
  ///
  /// 0 is a real state, not a placeholder: it means "we haven't read
  /// storage yet", and `shouldForwardDownload` refuses to forward
  /// anything until it's resolved. That matters because a service
  /// worker is routinely woken *by* the very download event we're
  /// being asked about, so `Date.now()` at boot is always a little
  /// later than that download's `startTime` — using the boot time as
  /// a stand-in would silently drop legitimate downloads.
  installTimeMs: 0,
  settings: { ...DEFAULT_SETTINGS },
};

function setBadge(badge) {
  const text = badge === "ok" ? "" : "!";
  const color = badge === "ok" ? "#74e0a4" : "#ff7a8f";
  try {
    chrome.action.setBadgeText({ text });
    chrome.action.setBadgeBackgroundColor({ color });
  } catch (e) {
    /* not available in some test harnesses */
  }
}

/// The slice of state the popup and the content scripts get to see.
function publicState() {
  return {
    connected: state.connected,
    lastError: state.lastError,
    lastSentAt: state.lastSentAt,
    lastSentCount: state.lastSentCount,
    transportKind: state.transportKind,
    settings: { ...state.settings },
  };
}

/// Push the current state to every tab's content script. Content
/// scripts cache it so the click handler can decide synchronously
/// (an async round-trip can't inform a `preventDefault()` that has
/// to happen *now*). Only called on real transitions, so this isn't
/// a per-keystroke broadcast.
function broadcastState() {
  try {
    chrome.tabs.query({}, (found) => {
      // Read `lastError` in the same tick as the callback.
      void chrome.runtime.lastError;
      const payload = { type: "state", state: publicState() };
      for (const tab of found || []) {
        if (!tab || tab.id == null) continue;
        try {
          chrome.tabs.sendMessage(tab.id, payload, () => {
            // Tabs without a content script (chrome:// pages, the
            // about: pages we deliberately skip, tabs loaded before
            // the extension) reject. Expected and harmless.
            void chrome.runtime.lastError;
          });
        } catch (_e) {
          void _e;
        }
      }
    });
  } catch (_e) {
    void _e;
  }
}

// One funnel for "the transport is not reachable" so the popup,
// the badge, and the console stay in sync. We deliberately do
// NOT include the full stack — `lastError` strings are
// already user-readable.
function setState(connected, message) {
  const next = !!connected;
  const reason = message || null;
  const changed = next !== state.connected || reason !== state.lastError;
  state.connected = next;
  state.lastError = reason;
  setBadge(next ? "ok" : "err");
  if (!next && reason) {
    console.warn("DM transport:", reason);
  }
  if (changed) broadcastState();
}

// ─── WebSocket transport (Chromium browsers) ────────────────────
//
// Wraps the standard browser `WebSocket` with reconnect, queueing,
// and the state integration. We don't pull in any libraries:
// the WebSocket API is built in to MV3 service workers, and a
// few hundred lines of state machine is enough.
function makeWebSocketTransport() {
  state.transportKind = "websocket";
  let ws = null;
  /// `[{ payload, at }]` — `at` is used to expire stale entries.
  let queue = [];
  let backoffIdx = 0;
  let closing = false;
  let onConnected = null; // hook for `forceReconnect` / `probeHost`

  function enqueue(payload) {
    const now = Date.now();
    // Expire anything that has been waiting too long. A click that
    // couldn't be delivered while DM was down should not reappear as
    // a modal ten minutes later.
    while (queue.length && now - queue[0].at > WS_QUEUE_TTL_MS) queue.shift();
    if (queue.length >= WS_QUEUE_MAX) queue.shift();
    queue.push({ payload, at: now });
  }

  function connect() {
    if (ws || closing) return;
    // Hold this attempt's socket locally and have every handler below
    // check `ws === socket` first.
    //
    // `forceReconnect` (the popup's "Test host connection") closes the
    // current socket and opens a replacement in the same tick; the
    // closed socket's `close` event then arrives *later*. Without the
    // identity check that stale event nulls out the reference to the
    // live socket, flips the state to "disconnected" while a perfectly
    // good socket is open — so the popup says DM isn't running — and
    // schedules yet another connect, leaking a connection every time.
    let socket;
    try {
      socket = new WebSocket(WS_URL);
    } catch (e) {
      setState(false, "WebSocket ctor failed: " + (e && e.message ? e.message : e));
      scheduleReconnect();
      return;
    }
    ws = socket;
    socket.addEventListener("open", () => {
      if (ws !== socket) return; // a superseded attempt
      backoffIdx = 0;
      setState(true, null);
      // Drain anything queued while we were disconnected, skipping
      // entries that have gone stale in the meantime.
      const pending = queue;
      queue = [];
      const now = Date.now();
      for (let i = 0; i < pending.length; i++) {
        const entry = pending[i];
        if (now - entry.at > WS_QUEUE_TTL_MS) continue;
        try {
          socket.send(JSON.stringify(entry.payload));
          state.lastSentAt = Date.now();
          state.lastSentCount++;
        } catch (e) {
          // Socket died mid-drain: put the rest back, keeping their
          // original timestamps so they still expire.
          queue = pending.slice(i).concat(queue);
          break;
        }
      }
      if (typeof onConnected === "function") {
        const cb = onConnected;
        onConnected = null;
        cb();
      }
    });
    socket.addEventListener("message", () => {
      // The Tauri side acks every payload with `{"ok":true}`.
      // We don't need the ack for correctness (the next send
      // would fail loudly if the socket were broken), but we
      // keep the listener wired so Chrome's WebSocket impl
      // doesn't fill an internal buffer.
    });
    socket.addEventListener("close", (ev) => {
      if (ws !== socket) return; // a superseded attempt: `ws` is the live one
      ws = null;
      setState(false, `disconnected (code ${ev.code})`);
      if (!closing) scheduleReconnect();
    });
    socket.addEventListener("error", () => {
      // The `error` event fires just before `close`. Chrome
      // does not populate a useful message here, so we let
      // `close` set the user-facing error string.
    });
  }

  function scheduleReconnect() {
    if (closing) return;
    const wait =
      WS_RECONNECT_BACKOFF_MS[Math.min(backoffIdx, WS_RECONNECT_BACKOFF_MS.length - 1)];
    backoffIdx++;
    setTimeout(connect, wait);
  }

  return {
    connect,
    send(payload) {
      if (ws && ws.readyState === WebSocket.OPEN) {
        try {
          ws.send(JSON.stringify(payload));
          state.lastSentAt = Date.now();
          state.lastSentCount++;
          return true;
        } catch (e) {
          // Fall through to the queue path below.
        }
      }
      // No live socket (or the send threw): queue and ensure a
      // connection attempt is in flight. The queue is capped so a
      // runaway tab can't OOM the SW — we drop oldest first, which
      // is the right call (the user clicked the most recent link).
      enqueue(payload);
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
      // The `close` handler now ignores events from superseded
      // sockets, so the state has to be set here rather than there.
      setState(false, "disconnected");
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
// Firefox MV3 cannot reliably open a `ws://127.0.0.1` connection
// (Firefox upgrades insecure ws:// requests to wss:// and fails),
// so Firefox users still install the `dm-native-host` binary and
// register the host manifest.
//
// **This path needs the `"nativeMessaging"` permission.** Without
// it `chrome.runtime.connectNative` is not a function at all and
// this transport can never connect — which is exactly what used to
// happen, because the permission was dropped from the manifest when
// the WebSocket transport landed. `manifest.json` declares it now;
// `scripts/package.sh` strips it again for the Chrome-shaped zip,
// which never uses this transport.
function makeNativeTransport() {
  state.transportKind = "native";
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
    connect,
    send(payload) {
      const p = connect();
      if (!p) return false;
      try {
        p.postMessage(payload);
        state.lastSentAt = Date.now();
        state.lastSentCount++;
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
      // Firefox's connectNative resolves/disconnects asynchronously;
      // give it a tick before reporting back so `probeHost` sees the
      // real outcome instead of the pre-attempt state.
      setTimeout(() => {
        if (typeof onDone === "function") onDone();
      }, 150);
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
  try {
    transport.connect();
  } catch (e) {
    setState(false, String((e && e.message) || e));
  }
}

/**
 * Force a fresh connection attempt and report what actually
 * happened. Used by the popup's "Test host" button.
 *
 * Resolves exactly once: `settled` guards the race between the
 * transport's own callback and our timeout, which used to be able
 * to resolve the promise twice and render a stale result.
 */
function probeHost() {
  return new Promise((resolve) => {
    let settled = false;
    let timer = null;
    const finish = () => {
      if (settled) return;
      settled = true;
      if (timer) clearTimeout(timer);
      resolve({
        ok: state.connected,
        lastError: state.lastError,
        transportKind: state.transportKind,
        hostName: isFirefox ? HOST_NAME : null,
        url: isFirefox ? null : WS_URL,
      });
    };
    // Safety net: the popup's "Test" button must never hang. WebSocket
    // events fire async, and a dead host only ever reports via the
    // `close` event.
    timer = setTimeout(finish, isFirefox ? 400 : 900);
    try {
      transport.forceReconnect(finish);
    } catch (e) {
      setState(false, String((e && e.message) || e));
      finish();
    }
  });
}

// ─── Install time (the "don't replay history" filter) ───────────

// Parse Chrome's `DownloadItem.startTime` (ISO 8601 string) → epoch ms.
// Returns 0 for unparseable / missing timestamps so callers can treat
// that as "drop, not enough info".
function startTimeToMs(s) {
  if (!s) return 0;
  const t = Date.parse(s);
  return Number.isFinite(t) ? t : 0;
}

let installTimePromise = null;

/**
 * Resolve `state.installTimeMs` from storage, at most once per
 * service-worker lifetime. Callers that need the real value `await`
 * this; nothing else should read `installTimeMs` before it resolves.
 */
function loadInstallTime() {
  if (installTimePromise) return installTimePromise;
  installTimePromise = (async () => {
    try {
      const got = await chrome.storage.local.get(STORAGE_KEY_INSTALL_TIME);
      const v = got && got[STORAGE_KEY_INSTALL_TIME];
      state.installTimeMs = typeof v === "number" && v > 0 ? v : Date.now();
    } catch (e) {
      // Storage failed — fall back to "now". Worse than the persisted
      // case (a download that began a moment before this SW booted
      // gets dropped) but strictly better than denying all downloads.
      console.warn("DM Grabber: loadInstallTime failed:", e);
      state.installTimeMs = Date.now();
    }
    return state.installTimeMs;
  })();
  return installTimePromise;
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
      state.installTimeMs = now;
    } else {
      state.installTimeMs = v;
    }
  } catch (e) {
    console.warn("DM Grabber: maybePersistInstallTime failed:", e);
    state.installTimeMs = Date.now();
  }
}

// ─── Settings ───────────────────────────────────────────────────

function normaliseSettings(raw) {
  const out = { ...DEFAULT_SETTINGS };
  if (raw && typeof raw === "object") {
    if (typeof raw.interceptOnClick === "boolean") {
      out.interceptOnClick = raw.interceptOnClick;
    }
    if (typeof raw.takeoverBrowserDownloads === "boolean") {
      out.takeoverBrowserDownloads = raw.takeoverBrowserDownloads;
    }
  }
  return out;
}

let settingsPromise = null;

function loadSettings() {
  if (settingsPromise) return settingsPromise;
  settingsPromise = (async () => {
    try {
      const got = await chrome.storage.local.get(STORAGE_KEY_SETTINGS);
      state.settings = normaliseSettings(got && got[STORAGE_KEY_SETTINGS]);
    } catch (e) {
      console.warn("DM Grabber: loadSettings failed:", e);
    }
    return state.settings;
  })();
  return settingsPromise;
}

async function saveSettings(patch) {
  state.settings = normaliseSettings({ ...state.settings, ...patch });
  // Tell the content scripts immediately — the click handler reads
  // its cached copy synchronously.
  broadcastState();
  try {
    await chrome.storage.local.set({ [STORAGE_KEY_SETTINGS]: state.settings });
  } catch (e) {
    console.warn("DM Grabber: saveSettings failed:", e);
  }
  return state.settings;
}

// ─── Download interception ──────────────────────────────────────

/// URLs we have just handed back to the browser ourselves. Without
/// this, our own `chrome.downloads.download()` fallback would
/// re-enter `onCreated` and we'd act on it a second time.
const IGNORED_URL_TTL_MS = 10_000;
const ignoredUrls = new Map();

function markIgnored(url) {
  ignoredUrls.set(url, Date.now());
}

function consumeIgnored(url) {
  const at = ignoredUrls.get(url);
  if (at == null) return false;
  ignoredUrls.delete(url);
  // A stale entry means the same URL came back for a different
  // reason (the user really did start this download); treat it as
  // a genuine new download.
  return Date.now() - at <= IGNORED_URL_TTL_MS;
}

/**
 * Reject anything that isn't plausibly a filename — a slash, a quote,
 * a control character, or a paragraph of anchor text. `downloads.download`
 * throws on an invalid filename, and throwing here would lose the
 * download we were trying to rescue.
 */
function safeFilename(name) {
  if (!name || typeof name !== "string") return null;
  const trimmed = name.trim();
  if (!trimmed || trimmed.length > 120) return null;
  if (/[\\/:*?"<>|\u0000-\u001f]/.test(trimmed)) return null;
  if (trimmed === "." || trimmed === "..") return null;
  return trimmed;
}

/**
 * Give the download back to the browser.
 *
 * This is the "never lose a download" half of the contract: the
 * content script has already called `preventDefault()`, so if we
 * can't hand the URL to DM the only acceptable outcome is to let
 * the browser do exactly what it would have done. The resulting
 * `onCreated` event is suppressed via `ignoredUrls`.
 */
function handBackToBrowser(url, filename) {
  if (!url) return false;
  const opts = { url, saveAs: false };
  const clean = safeFilename(filename);
  if (clean) opts.filename = clean;
  markIgnored(url);
  try {
    chrome.downloads.download(opts, (id) => {
      const err = chrome.runtime.lastError;
      if (err) {
        console.warn("DM Grabber: browser fallback failed:", url, "-", err.message);
        return;
      }
      console.info(
        "DM Grabber: DM unreachable — handed the download back to the browser:",
        url,
        "(id",
        id,
        ")",
      );
    });
    return true;
  } catch (e) {
    console.warn("DM Grabber: browser fallback threw:", e);
    return false;
  }
}

/**
 * Cancel the browser's own copy of `item` and remove the entry from
 * the downloads list, so the user doesn't see a phantom "interrupted"
 * row in `chrome://downloads` next to the successful DM transfer.
 *
 * Only called once the URL has actually reached DM. The cancel is
 * best-effort: if `cancel` rejects (the entry is already gone, or
 * Chrome's UI is in a state that disallows cancel) we log and move on.
 */
function takeOverChromeDownload(item) {
  const id = item && item.id;
  if (id == null) return;
  try {
    chrome.downloads.cancel(id, () => {
      // The callback fires whether or not the cancel succeeded.
      // Reading `lastError` keeps the browser from logging
      // "Unchecked runtime.lastError" when the entry was already gone.
      void chrome.runtime.lastError;
      discardPartialThenErase(id);
    });
  } catch (e) {
    // The cancel API can throw if the download is in a state that
    // disallows cancellation (e.g. "complete"). The DM copy is
    // already in flight; nothing else to do.
    console.warn("DM Grabber: chrome.downloads.cancel failed:", e);
  }
}

/**
 * Clean up after a cancelled download.
 *
 * Chrome was mid-write when we cancelled, so what's on disk is a
 * truncated fragment rather than the file the user asked for. We
 * remove it so it doesn't linger as a mystery orphan — but **only
 * when it isn't complete**: a finished file is real data and deleting
 * it would be destructive. Note that `downloads.erase()` on its own
 * only clears history; deleting bytes requires `downloads.removeFile()`.
 */
function discardPartialThenErase(id) {
  function finish() {
    eraseEntry(id);
  }
  try {
    chrome.downloads.search({ id }, (items) => {
      void chrome.runtime.lastError;
      const found = items && items[0];
      if (!found || found.state !== "complete") {
        try {
          chrome.downloads.removeFile(id, () => {
            void chrome.runtime.lastError;
            finish();
          });
          return;
        } catch (_e) {
          void _e;
        }
      }
      finish();
    });
  } catch (_e) {
    // Search unavailable or threw: erase the history row only. We
    // never delete a file we couldn't confirm is incomplete.
    void _e;
    finish();
  }
}

function eraseEntry(id) {
  try {
    chrome.downloads.erase({ id }, () => {
      void chrome.runtime.lastError;
    });
  } catch (_e) {
    void _e;
  }
}

// Decide whether a `DownloadItem` should be forwarded to DM. Drops:
//   - items whose `startTime` is missing or unparseable (defensive);
//   - items that started before the extension was installed
//     (the documented re-fire on install that would otherwise flood
//     DM with the user's entire download history);
//   - everything, while the install time is still unknown.
function shouldForwardDownload(item) {
  if (!item || !item.url) return false;
  if (state.installTimeMs === 0) return false;
  const t = startTimeToMs(item.startTime);
  if (t === 0) {
    // No timestamp → can't tell if it's historical. Be conservative
    // and drop it. The user can re-download manually if needed.
    return false;
  }
  return t >= state.installTimeMs;
}

/**
 * Forward a real browser download ("Save link as", Ctrl-click,
 * another extension's download, …) to DM — but only those that
 * started **after** the extension was installed, and only when DM
 * actually accepts the URL.
 */
async function handleCreatedDownload(item) {
  if (!item || !item.url) return;
  // Our own fallback download coming back around. Must be checked
  // before anything else, or we'd ping-pong.
  if (consumeIgnored(item.url)) return;
  if (!state.settings.takeoverBrowserDownloads) return;
  // A service worker is often woken *by* this event, so the boot
  // sequence may not have read storage yet. Resolve before judging.
  if (state.installTimeMs === 0) await loadInstallTime();
  if (!shouldForwardDownload(item)) return;
  // Forward first, cancel second — and only cancel if the forward
  // actually succeeded. The two used to be independent, which meant
  // a dead host still destroyed the download the user had started.
  const delivered = send({
    type: "download",
    url: item.url,
    filename: item.filename,
  });
  if (!delivered) {
    console.warn(
      "DM Grabber: DM unreachable — leaving the browser's download in place:",
      item.url,
    );
    return;
  }
  if (item.id != null) {
    takeOverChromeDownload(item);
  }
}

// Registered **synchronously at the top level**, which is what MV3
// requires: a listener added later (after an `await`) misses the
// event that woke the service worker, and the browser may then stop
// waking it for that event type altogether. The install-time filter
// that used to make us defer this is applied inside the handler.
chrome.downloads.onCreated.addListener((item) => {
  void handleCreatedDownload(item);
});

// ─── Messaging ──────────────────────────────────────────────────

/**
 * The popup's "Grab page media" button. Answers with what actually
 * happened — the popup used to hardcode success and print
 * "✓ Native host connected" no matter what.
 */
function handleGrab(tabId, sendResponse) {
  if (tabId == null) {
    sendResponse({ ok: false, error: "no active tab", delivered: false });
    return;
  }
  try {
    chrome.tabs.sendMessage(tabId, { type: "collect" }, (res) => {
      const err = chrome.runtime.lastError;
      if (err) {
        sendResponse({ ok: false, error: err.message, delivered: false });
        return;
      }
      const urls = (res && res.urls) || [];
      if (!urls.length) {
        sendResponse({
          ok: true,
          urls: 0,
          delivered: false,
          error: "no media found on this page",
        });
        return;
      }
      const delivered = send({
        type: "capture",
        url: (res && res.pageUrl) || "",
        urls,
        referer: (res && res.referer) || "",
        userAgent: (res && res.userAgent) || "",
      });
      sendResponse({
        ok: true,
        urls: urls.length,
        delivered,
        error: delivered ? null : state.lastError || "DM host not reachable",
      });
    });
  } catch (e) {
    sendResponse({
      ok: false,
      error: String((e && e.message) || e),
      delivered: false,
    });
  }
}

// Messages from content scripts / popup.
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (!msg) return;
  switch (msg.type) {
    case "get-status":
      sendResponse({ ok: true, state: publicState() });
      return;
    // The popup's "Test host" button. Forces a fresh connect
    // attempt and reports the actual result — including the precise
    // `chrome.runtime.lastError.message` from Firefox/Chrome.
    case "probe":
      probeHost().then((res) => sendResponse({ ok: true, probe: res }));
      return true; // tell the browser we'll call sendResponse async
    case "set-settings":
      saveSettings(msg.settings || {})
        .then((settings) =>
          sendResponse({ ok: true, settings, state: publicState() }),
        )
        .catch((e) =>
          sendResponse({ ok: false, error: String((e && e.message) || e) }),
        );
      return true; // async
    case "grab":
      handleGrab(msg.tabId, sendResponse);
      return true; // async
    case "download-click": {
      // **Primary takeover path.** The content script has already
      // synchronously suppressed the browser's default "start a
      // download" behaviour, and only did so because it believed DM
      // was reachable. If that belief was stale, hand the download
      // straight back to the browser rather than dropping it.
      if (!msg.url) {
        sendResponse({ ok: false, error: "missing url", delivered: false });
        return;
      }
      const delivered = send({
        type: "download",
        url: msg.url,
        filename: msg.filename || "",
        // These two used to be dropped here even though the content
        // script has always sent them: the Tauri side reads them to
        // pre-fill the per-download Referer / User-Agent fields.
        referer: msg.referer || "",
        userAgent: msg.userAgent || "",
      });
      if (delivered) {
        sendResponse({ ok: true, delivered: true });
        return;
      }
      const handedBack = handBackToBrowser(msg.url, msg.filename);
      sendResponse({ ok: true, delivered: false, handedBack });
      return;
    }
    // We **deliberately do not** accept a `type === "media"` message
    // — the legacy auto-scrape path that used to spam DM on every
    // page load / DOM mutation has been removed in both content.js
    // and here, so any third-party content script (or a stale cached
    // copy of our own) that still sends a `media` message is dropped
    // on the floor. This is a hard boundary: a download manager must
    // not be a passive capture-everything tap on the browser.
    default:
      return;
  }
});

// ─── Lifecycle ──────────────────────────────────────────────────

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
  await loadSettings();
  broadcastState();
  connect();
});

chrome.runtime.onStartup.addListener(async () => {
  await loadInstallTime();
  await loadSettings();
  broadcastState();
  connect();
});

// Keep in sync with a popup open in another window (or with our own
// writes, which is harmless — the values are already equal).
chrome.storage.onChanged.addListener((changes, area) => {
  if (area !== "local" || !changes[STORAGE_KEY_SETTINGS]) return;
  state.settings = normaliseSettings(changes[STORAGE_KEY_SETTINGS].newValue);
  broadcastState();
});

// Boot. Both listeners above are already registered by the time we
// get here, so nothing is missed; these only warm the caches and
// open the transport.
void loadInstallTime();
loadSettings().then(() => connect());
