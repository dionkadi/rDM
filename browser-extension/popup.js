// Popup UI: grab the current tab's media, show the real host status,
// and expose the two behaviour switches.
//
// Two things this file used to get wrong and no longer does:
//   * The "Grab page media" button reported success unconditionally.
//     It hardcoded `{connected: true, lastSentCount: 1}` in the
//     message callback and never looked at `chrome.runtime.lastError`
//     or the background's answer, so the one screen that exists to
//     tell you DM is unreachable told you the opposite.
//   * The status line said "port 9157" even on the Firefox
//     native-messaging transport, where there is no port.
//
// The status pill shows the *headline*; when the host is down we also
// surface a more visible error block with the actual
// `chrome.runtime.lastError.message` Firefox/Chrome returned. On
// Firefox that message is usually the generic "No such native
// application com.app.dm.native" — which means the host manifest
// could not be located, the extension's ID isn't in
// `allowed_extensions`, or the binary path is wrong. The "Test host
// connection" button forces a fresh connection attempt and reports
// the result verbatim, so the user can see what's actually wrong
// without opening the browser console.
const status = document.getElementById("status");
const errDetail = document.getElementById("err-detail");
const openBtn = document.getElementById("open");
const grabBtn = document.getElementById("grab");
const optClick = document.getElementById("opt-click");
const optTakeover = document.getElementById("opt-takeover");

/** Clear the children of `el` without using `innerHTML = ""`. */
function clearChildren(el) {
  while (el.firstChild) el.removeChild(el.firstChild);
}

/** Append a text node. */
function appendText(parent, text) {
  parent.appendChild(document.createTextNode(text));
}

/**
 * Render an HTML-ish string built from a structured array of
 * `{tag, attrs, text}` nodes into `parent` using safe DOM
 * construction. We use this so the hint strings can mention
 * `<code>...</code>` and `<b>...</b>` for readability without
 * resorting to `innerHTML` (which the project's linter flags as
 * XSS-prone). All `text` values are escaped.
 */
function renderMarkup(parent, parts) {
  for (const part of parts) {
    const el = document.createElement(part.tag);
    if (part.attrs) {
      for (const [k, v] of Object.entries(part.attrs)) {
        el.setAttribute(k, v);
      }
    }
    if (Array.isArray(part.children)) {
      renderMarkup(el, part.children);
    } else if (typeof part.text === "string") {
      appendText(el, part.text);
    }
    parent.appendChild(el);
  }
}

/** Human-readable name for the transport that is actually in use. */
function transportLabel(kind) {
  if (kind === "native") {
    return "native messaging (com.app.dm.native)";
  }
  if (kind === "websocket") {
    return "WebSocket 127.0.0.1:9157";
  }
  return "transport unknown";
}

/**
 * Which layer is actually broken?
 *
 * "Not connected" used to always render as "DM host not running", which
 * is a lie in two of the three cases below — and the two lies send the
 * user to the wrong place. The most common one by far is `background`:
 * loading the repo folder in Chrome gives an MV3 extension with no
 * service worker at all (Chrome ignores `background.scripts`), so the
 * popup cannot even ask the background for state, let alone reach DM.
 */
function failureKind(state) {
  const msg = (state.lastError || "").toLowerCase();
  if (
    msg.includes("receiving end does not exist") ||
    msg.includes("message port closed") ||
    msg.includes("no response from background") ||
    msg.includes("background not reachable")
  ) {
    return "background";
  }
  if (msg.includes("websocket ctor failed")) return "blocked";
  return "host";
}

function headline(connected, kind) {
  if (connected) return "✓ DM host connected";
  if (kind === "background") return "✗ Extension background isn't running.";
  if (kind === "blocked") return "✗ The browser blocked the connection to DM.";
  return "✗ DM host not running.";
}

/**
 * The one hint worth more than all the others: DM is up, and the
 * extension still can't reach it. The browser only ever reports
 * `close code 1006`, which is identical whether nothing is listening or
 * whether a pre-0.4.2 DM build is listening (its listener only spoke
 * line-delimited JSON and swallows the upgrade request). The bundled
 * probe tells them apart in two seconds.
 */
const HOST_DOWN_HINT =
  "The browser only reports `close code 1006` for every way this can " +
  "fail, so the app being open doesn't rule it out. Run this to find " +
  "out which it is: node browser-extension/scripts/check-host.mjs";

// Map a `chrome.runtime.lastError.message` to a one-line hint
// pointing at the most likely fix. The messages below are exactly
// what Chrome and Firefox emit; see NativeMessaging.sys.mjs in
// Firefox (the `_throwGenericError` helper) and the Chrome
// "Native Messaging" docs.
function hintForError(msg, kind) {
  const m = (msg || "").toLowerCase();
  if (kind === "background") {
    return [
      { tag: "b", text: "The extension's service worker never started, so " },
      { tag: "b", text: "nothing" },
      {
        tag: "b",
        text: " can reach DM — the host may well be running fine. This is " +
          "what happens when the extension is loaded from the source folder " +
          "in Chrome: Chrome ignores ",
      },
      { tag: "code", text: "background.scripts" },
      { tag: "b", text: " under Manifest V3, so there is no background at all. Install the released " },
      { tag: "code", text: "dm-grabber-<version>.zip" },
      { tag: "b", text: " (not the repo folder), then Reload on chrome://extensions." },
    ];
  }
  if (kind === "blocked") {
    return [
      { tag: "b", text: "The browser refused to open the WebSocket. Check that the extension " },
      { tag: "b", text: "isn't built with a restrictive " },
      { tag: "code", text: "content_security_policy" },
      { tag: "b", text: " (connect-src) and reload it." },
    ];
  }
  if (!msg) return null;
  // The P0 regression this popup can now actually diagnose: without
  // the `nativeMessaging` permission, `runtime.connectNative` is not
  // even a function in Firefox.
  if (m.includes("is not a function") || m.includes("nativemessaging")) {
    return [
      { tag: "b", text: "The extension is missing the " },
      { tag: "code", text: "nativeMessaging" },
      {
        tag: "b",
        text: " permission — reload it from a build that declares it.",
      },
    ];
  }
  // Connected-but-not-really: the socket failed. `1006` is the only
  // thing the browser will ever say, and it can't distinguish "not
  // running" from "running but never answered the upgrade".
  if (kind === "host" && (m.includes("1006") || m.includes("disconnected"))) {
    return [
      { tag: "b", text: HOST_DOWN_HINT },
    ];
  }
  if (m.includes("no such native application")) {
    return [
      "Firefox can't find a valid host manifest for this extension. " +
        "Make sure ",
      {
        tag: "code",
        text: "~/.mozilla/native-messaging-hosts/com.app.dm.native.json",
      },
      " exists and lists ",
      { tag: "code", text: "dm-grabber@dm-project" },
      " in ",
      { tag: "code", text: "allowed_extensions" },
      " (use the bundled ",
      { tag: "code", text: "com.app.dm.native.firefox.json" },
      " template).",
    ];
  }
  if (m.includes("not found") || m.includes("not found.")) {
    return [
      "The host manifest points to a binary the browser can't read. " +
        "Verify the ",
      { tag: "code", text: "path" },
      " in the host manifest is absolute and the file is executable.",
    ];
  }
  if (m.includes("access") || m.includes("denied") || m.includes("allowed")) {
    return [
      "The extension isn't allowed to use this host. Check the host manifest's ",
      { tag: "code", text: "allowed_origins" },
      " (Chrome) or ",
      { tag: "code", text: "allowed_extensions" },
      " (Firefox) matches your extension's ID exactly.",
    ];
  }
  if (m.includes("not specified") || m.includes("manifest")) {
    return [
      "The host manifest is malformed or missing required fields. " +
        "Re-copy it from the bundle: ",
      { tag: "code", text: "browser-extension/com.app.dm.native.firefox.json" },
      ".",
    ];
  }
  return null;
}

// ─── Transient notes ────────────────────────────────────────────
//
// A note is the result of something the user just did ("nothing was
// sent", "found 3 URLs"). It outlives a couple of poll ticks so the
// 1.5 s refresh doesn't wipe it, and then goes away on its own.
const NOTE_TTL_MS = 6000;
let note = null;

function setNote(cls, text) {
  note = { cls, text, at: Date.now() };
}

function activeNote() {
  if (note && Date.now() - note.at < NOTE_TTL_MS) return note;
  note = null;
  return null;
}

function syncOptions(settings) {
  if (!settings) return;
  if (optClick) optClick.checked = !!settings.interceptOnClick;
  if (optTakeover) optTakeover.checked = !!settings.takeoverBrowserDownloads;
}

let lastState = null;

function setStatus(state, pendingNote) {
  errDetail.className = "err-detail";
  clearChildren(errDetail);
  clearChildren(status);

  if (!state) {
    status.className = "";
    appendText(status, "Checking…");
    return;
  }

  const connected = !!state.connected;
  const kind = connected ? null : failureKind(state);
  status.className = pendingNote ? pendingNote.cls : connected ? "ok" : "err";
  appendText(
    status,
    connected
      ? `✓ DM host connected — ${transportLabel(state.transportKind)}.\n`
      : `${headline(false, kind)}\n`,
  );

  if (connected) {
    const sent = state.lastSentCount || 0;
    const ago =
      state.lastSentAt > 0
        ? `${Math.round((Date.now() - state.lastSentAt) / 1000)}s ago`
        : "never";
    appendText(
      status,
      `Sent ${sent} ${sent === 1 ? "request" : "requests"} (last: ${ago}).\n`,
    );
  }

  if (pendingNote) {
    appendText(status, pendingNote.text);
  }

  if (connected) {
    // The "Open DM app" button can't actually launch a Tauri app
    // from a browser (no `chrome.app` API in MV3; the Tauri
    // `tauri://` scheme is unknown to Chrome), so it's really a
    // one-click re-check. Label it as such rather than implying it
    // launches anything.
    if (openBtn) {
      openBtn.title =
        "Ask the background to re-read the connection state. " +
        "This browser extension can't launch the Tauri app for you.";
    }
    return;
  }

  // Not connected: show the actual `chrome.runtime.lastError.message`
  // so the user can diagnose without opening the browser console.
  const detail = (state.lastError || "").toString();
  if (detail) {
    renderMarkup(errDetail, [
      { tag: "b", text: "Browser said: " },
      { tag: "code", text: detail },
    ]);
    const hint = hintForError(detail, kind);
    if (hint) {
      const hintEl = document.createElement("span");
      hintEl.className = "hint";
      renderMarkup(hintEl, hint);
      errDetail.appendChild(hintEl);
    }
    errDetail.className = "err-detail shown";
  } else {
    const hintEl = document.createElement("span");
    hintEl.className = "hint";
    appendText(hintEl, "The host process never reported a status. Click ");
    const b = document.createElement("b");
    appendText(b, "Test host connection");
    hintEl.appendChild(b);
    appendText(hintEl, " below to retry and see the actual error.");
    errDetail.appendChild(hintEl);
    errDetail.className = "err-detail shown";
  }
  if (openBtn) {
    openBtn.title =
      "The DM app doesn't seem to be running. Launch it on your " +
      "desktop, then click here to re-check the connection.";
  }
}

function render() {
  setStatus(lastState, activeNote());
}

// ─── Talking to the background ──────────────────────────────────

/** One-shot message with `lastError` folded into the result. */
function sendMessage(msg) {
  return new Promise((resolve) => {
    try {
      chrome.runtime.sendMessage(msg, (res) => {
        const err = chrome.runtime.lastError;
        resolve(err ? { ok: false, error: err.message } : res);
      });
    } catch (e) {
      resolve({ ok: false, error: (e && e.message) || String(e) });
    }
  });
}

// Probe the background service worker for current state.
//
// MV3 service workers are *terminated* after ~30 s of inactivity and
// re-launched on the next event. The very first `sendMessage` to a
// freshly-launched (or still-loading) SW can race the SW registering
// its `chrome.runtime.onMessage` listener — the browser then throws
// "Could not establish connection. Receiving end does not exist."
// (or, on some Chromium versions, silently resolves to `undefined`).
//
// That message is **not** a real failure; it's a cold-start blip. We
// retry with exponential backoff so the popup doesn't flash a scary
// red error for the first ~1 s after the user clicks the toolbar
// icon. The total cold-start budget is ~680 ms — well under the
// 1.5 s polling interval, so a successful reply still wins the race
// against the next `setInterval` tick. After the budget is
// exhausted we surface the actual `lastError` so the user can
// diagnose a real failure.
//
// We also use the callback form of `sendMessage` so we always see
// `chrome.runtime.lastError` in the same tick as the call — a
// `await` over the promise form can swallow `lastError` and we'd
// then have to fall back to the "no response" branch, which is
// less informative.
const COLD_START_BACKOFF_MS = [80, 200, 400];
const MAX_COLD_START_RETRIES = COLD_START_BACKOFF_MS.length;

let probeInFlight = false;

function isColdStartError(msg) {
  // The exact Chromium text. Brave inherits this verbatim.
  // Firefox's equivalent (when the SW is a non-persistent event
  // page that's not currently loaded) is the same string, so this
  // check is cross-browser-safe.
  //
  // `"no response from background"` is our own sentinel for the
  // silent-no-response case: some Chromium versions accept the
  // message and resolve the callback to `undefined` with no
  // `lastError` set, when the SW is still in its boot sequence.
  // That's the same root cause (cold start) and deserves the same
  // gentle retry.
  if (!msg) return false;
  const m = msg.toLowerCase();
  return (
    m.includes("receiving end does not exist") ||
    m.includes("message port closed before a response") ||
    m.includes("no such native application") ||
    m === "no response from background"
  );
}

/**
 * Send a single `get-status` message to the SW. Resolves to
 * `{ ok: true, state }` on success, or `{ ok: false, error }` on
 * any failure (cold-start, no-response, or real error). Never
 * throws.
 */
function probeOnce() {
  return new Promise((resolve) => {
    try {
      chrome.runtime.sendMessage({ type: "get-status" }, (res) => {
        // CRITICAL: read `lastError` in the same tick as the
        // callback, before any other `chrome.*` call. Chromium and
        // Firefox both clear it on the next runtime API call.
        const lastErr = chrome.runtime.lastError;
        if (lastErr) {
          resolve({ ok: false, error: lastErr.message || String(lastErr) });
          return;
        }
        if (res && res.state) {
          resolve({ ok: true, state: res.state });
          return;
        }
        // No error, no `state` — the SW accepted the message but
        // didn't reply. This happens on a fresh cold start when the
        // SW was still in the middle of its boot sequence when the
        // message arrived. Treat as a cold-start transient.
        resolve({ ok: false, error: "no response from background" });
      });
    } catch (e) {
      // `sendMessage` is documented as callback-only and shouldn't
      // throw, but Firefox in some versions throws synchronously
      // when the SW is in a bad state. Catch defensively.
      resolve({ ok: false, error: (e && e.message) || String(e) });
    }
  });
}

async function refresh() {
  if (probeInFlight) return; // never overlap probes
  probeInFlight = true;
  try {
    // First attempt, then retry cold-start failures only. A
    // non-cold-start error (e.g. host manifest missing) is
    // surfaced immediately — there's no benefit to retrying it.
    let r = await probeOnce();
    let attempts = 0;
    while (
      !r.ok &&
      isColdStartError(r.error) &&
      attempts < MAX_COLD_START_RETRIES
    ) {
      const wait = COLD_START_BACKOFF_MS[attempts];
      attempts++;
      await new Promise((res2) => setTimeout(res2, wait));
      r = await probeOnce();
    }
    if (r.ok && r.state) {
      lastState = r.state;
      syncOptions(r.state.settings);
      render();
      return;
    }
    // We never got a state from the SW. Show the actual error
    // from the last attempt — it's the most informative one, and
    // for cold-start exhaustion it'll be the genuine
    // "Receiving end does not exist" the user can paste into a
    // bug report.
    lastState = {
      connected: false,
      lastError: r.error || "background not reachable",
    };
    render();
  } finally {
    probeInFlight = false;
  }
}

refresh();
setInterval(refresh, 1500);

// ─── Actions ────────────────────────────────────────────────────

if (grabBtn) {
  grabBtn.addEventListener("click", async () => {
    grabBtn.disabled = true;
    const label = grabBtn.textContent;
    grabBtn.textContent = "Grabbing…";
    try {
      const [tab] = await chrome.tabs.query({
        active: true,
        currentWindow: true,
      });
      if (!tab || tab.id == null) {
        setNote("err", "No active tab to grab from.");
        render();
        return;
      }
      const res = await sendMessage({ type: "grab", tabId: tab.id });
      if (!res || !res.ok) {
        setNote(
          "err",
          `Grab failed: ${(res && res.error) || "no response from the background"}.`,
        );
      } else if (!res.urls) {
        setNote("warn", "No media found on this page.");
      } else if (!res.delivered) {
        setNote(
          "err",
          `Found ${res.urls} media URL(s) but nothing was sent — ${
            res.error || "DM is unreachable"
          }.`,
        );
      } else {
        setNote("ok", `Sent ${res.urls} media URL(s) to DM.`);
      }
      // Re-read the real state and render it together with the note.
      await refresh();
      render();
    } catch (e) {
      setNote("err", "Grab failed: " + ((e && e.message) || e));
      render();
    } finally {
      grabBtn.disabled = false;
      grabBtn.textContent = label;
    }
  });
}

if (openBtn) {
  openBtn.addEventListener("click", () => {
    // The previous version of this handler called `alert(...)` when
    // the host was down. `alert()` in an MV3 popup is a synchronous,
    // unclosable native dialog that takes focus away from the popup
    // and (in some Chromium versions) cannot be dismissed without
    // killing the popup. That's the worst possible UX for a status
    // indicator.
    //
    // Instead we re-issue the status probe. If the user has just
    // launched the DM app on their desktop, this is exactly what
    // they want — a single click to confirm the host is now
    // reachable.
    void refresh();
  });
}

const testBtn = document.getElementById("test");
if (testBtn) {
  testBtn.addEventListener("click", async () => {
    testBtn.disabled = true;
    const label = testBtn.textContent;
    testBtn.textContent = "Testing…";
    try {
      const res = await sendMessage({ type: "probe" });
      if (res && res.probe) {
        const ok = !!res.probe.ok;
        lastState = {
          connected: ok,
          lastError: ok ? null : res.probe.lastError || "host did not respond",
          transportKind: res.probe.transportKind,
          lastSentAt: lastState ? lastState.lastSentAt : 0,
          lastSentCount: lastState ? lastState.lastSentCount : 0,
          settings: lastState ? lastState.settings : undefined,
        };
        syncOptions(lastState.settings);
      } else {
        lastState = {
          connected: false,
          lastError:
            "probe returned no result: " + JSON.stringify(res),
        };
      }
      render();
    } catch (e) {
      lastState = {
        connected: false,
        lastError: "probe failed: " + ((e && e.message) || e),
      };
      render();
    } finally {
      testBtn.disabled = false;
      testBtn.textContent = label;
    }
  });
}

async function onOptionChange() {
  const patch = {
    interceptOnClick: optClick ? !!optClick.checked : true,
    takeoverBrowserDownloads: optTakeover ? !!optTakeover.checked : true,
  };
  const res = await sendMessage({ type: "set-settings", settings: patch });
  if (res && res.state) {
    lastState = res.state;
    syncOptions(res.state.settings);
  } else {
    setNote(
      "err",
      "Couldn't save that setting: " + ((res && res.error) || "no response"),
    );
  }
  render();
}

if (optClick) optClick.addEventListener("change", onOptionChange);
if (optTakeover) optTakeover.addEventListener("change", onOptionChange);
