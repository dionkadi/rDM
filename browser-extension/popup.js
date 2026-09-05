// Popup UI: grab the current tab's media and surface the native host status.
//
// The status pill at the bottom shows the *headline*; when the host is
// down we also surface a more visible error block with the actual
// `chrome.runtime.lastError.message` Firefox/Chrome returned. On
// Firefox that message is usually the generic "No such native
// application com.app.dm.native" — which means the host manifest
// could not be located, the extension's ID isn't in
// `allowed_extensions`, or the binary path is wrong. The "Test host
// connection" button forces a fresh `connectNative` and reports the
// result verbatim, so the user can see what's actually wrong
// without opening the browser console.
const status = document.getElementById("status");
const errDetail = document.getElementById("err-detail");
const openBtn = document.getElementById("open");

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

// Map a `chrome.runtime.lastError.message` to a one-line hint
// pointing at the most likely fix. The messages below are exactly
// what Chrome and Firefox emit; see NativeMessaging.sys.mjs in
// Firefox (the `_throwGenericError` helper) and the Chrome
// "Native Messaging" docs.
function hintForError(msg) {
  if (!msg) return null;
  const m = msg.toLowerCase();
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

function setStatus(state) {
  status.className = "";
  clearChildren(errDetail);
  errDetail.classList.remove("shown");
  if (!state) {
    clearChildren(status);
    appendText(status, "Checking…");
    // While the first probe is in flight we don't know if the host
    // is up; leave the "Open DM app" button alone (whatever its
    // previous state was).
    return;
  }
  if (state.connected) {
    status.className = "ok";
    clearChildren(status);
    const sent = state.lastSentCount || 0;
    const ago =
      state.lastSentAt > 0
        ? `${Math.round((Date.now() - state.lastSentAt) / 1000)}s ago`
        : "never";
    appendText(status, "✓ Native host connected (port 9157).\n");
    appendText(
      status,
      `Sent ${sent} ${sent === 1 ? "request" : "requests"} (last: ${ago}).`,
    );
    // The "Open DM app" button can't actually launch a Tauri app
    // from a browser (no `chrome.app` API in MV3; the Tauri
    // `tauri://` scheme is unknown to Chrome). When the host is up
    // we keep the button enabled as a no-op so the user can click
    // it without seeing a modal that blocks the rest of the popup
    // (the previous `alert()` was unclosable in MV3 popup context).
    if (openBtn) {
      openBtn.disabled = true;
      openBtn.title =
        "DM is already running (the native host is reachable). " +
        "Use the DM window directly — this browser extension can't " +
        "launch the Tauri app.";
    }
    return;
  }
  // Not connected. Show the headline + a detail block with the
  // actual `chrome.runtime.lastError.message` so the user can
  // diagnose without opening the browser console.
  status.className = "err";
  clearChildren(status);
  appendText(status, "✗ Native host not running.");
  const detail = (state.lastError || "").toString();
  if (detail) {
    renderMarkup(errDetail, [
      { tag: "b", text: "Browser said: " },
      { tag: "code", text: detail },
    ]);
    const hint = hintForError(detail);
    if (hint) {
      const hintEl = document.createElement("span");
      hintEl.className = "hint";
      renderMarkup(hintEl, hint);
      errDetail.appendChild(hintEl);
    }
    errDetail.classList.add("shown");
  } else {
    const hintEl = document.createElement("span");
    hintEl.className = "hint";
    appendText(hintEl, "The host process never reported a status. Click ");
    const b = document.createElement("b");
    appendText(b, "Test host connection");
    hintEl.appendChild(b);
    appendText(hintEl, " below to retry and see the actual error.");
    errDetail.appendChild(hintEl);
    errDetail.classList.add("shown");
  }
  // When the host is down, give the user a way to re-check from
  // here: the "Open DM app" button no longer fires a modal alert;
  // it's repurposed as a one-click re-probe so the user can confirm
  // whether launching the DM app on their desktop has fixed the
  // problem. The button label and title explain what it does.
  if (openBtn) {
    openBtn.disabled = false;
    openBtn.textContent = "Re-check host";
    openBtn.title =
      "The DM app doesn't seem to be running. Launch it on your " +
      "desktop, then click here to re-check the connection.";
  }
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
        // SW was still in the middle of its boot sequence
        // (`loadInstallTime().then(registerDownloadsListener)…`)
        // when the message arrived. Treat as a cold-start transient.
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
      setStatus(r.state);
      return;
    }
    // We never got a state from the SW. Show the actual error
    // from the last attempt — it's the most informative one, and
    // for cold-start exhaustion it'll be the genuine
    // "Receiving end does not exist" the user can paste into a
    // bug report.
    setStatus({
      connected: false,
      lastError: r.error || "background not reachable",
    });
  } finally {
    probeInFlight = false;
  }
}
refresh();
setInterval(refresh, 1500);

document.getElementById("grab").addEventListener("click", async () => {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab) return;
  chrome.runtime.sendMessage({ type: "grab", tabId: tab.id }, () => {
    setStatus({
      connected: true,
      lastSentAt: Date.now(),
      lastSentCount: 1,
    });
  });
});

document.getElementById("open").addEventListener("click", () => {
  // The previous version of this handler called `alert(...)` when
  // the host was down. `alert()` in an MV3 popup is a synchronous,
  // unclosable native dialog that takes focus away from the popup
  // and (in some Chromium versions) cannot be dismissed without
  // killing the popup. That's the worst possible UX for a status
  // indicator.
  //
  // Instead we re-issue the status probe. The next `setInterval`
  // tick (1.5 s) will pick up the result and re-render the status
  // pill. If the user has just launched the DM app on their
  // desktop, this is exactly what they want — a single click to
  // confirm the host is now reachable.
  refresh();
});

document.getElementById("test").addEventListener("click", async () => {
  const btn = document.getElementById("test");
  btn.disabled = true;
  btn.textContent = "Testing…";
  try {
    const res = await chrome.runtime.sendMessage({ type: "probe" });
    if (res && res.probe) {
      setStatus({
        connected: !!res.probe.ok,
        lastError: res.probe.ok
          ? null
          : res.probe.lastError || "host did not respond",
      });
    } else {
      setStatus({
        connected: false,
        lastError: "probe returned no result: " + JSON.stringify(res),
      });
    }
  } catch (e) {
    setStatus({
      connected: false,
      lastError: "probe failed: " + (e && e.message ? e.message : e),
    });
  } finally {
    btn.disabled = false;
    btn.textContent = "Test host connection";
  }
});
