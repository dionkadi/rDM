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
}

// Probe the background service worker for current state.
async function refresh() {
  try {
    const res = await chrome.runtime.sendMessage({ type: "get-status" });
    if (res && res.state) setStatus(res.state);
  } catch (e) {
    setStatus({
      connected: false,
      lastError:
        "background not reachable: " + (e && e.message ? e.message : e),
    });
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
  // Open the DM app (the native host won't be reachable if the app
  // isn't running, so this is the most useful "wake it up" action
  // from the extension's perspective). Tauri/WebView2 URLs are not
  // something Chrome can launch directly, so we surface the host
  // status instead and tell the user to launch the app.
  chrome.runtime.sendMessage({ type: "get-status" }, (res) => {
    if (chrome.runtime.lastError || !res || !res.state) {
      alert(
        "DM is not running.\n\n" +
          "Launch the DM app, then click this button again to verify " +
          "the native host is reachable.",
      );
    }
  });
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
