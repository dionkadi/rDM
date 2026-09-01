// Popup UI: grab the current tab's media and surface the native host status.
const status = document.getElementById("status");

function setStatus(state) {
  status.className = "";
  if (!state) {
    status.textContent = "Checking…";
    return;
  }
  if (state.connected) {
    status.className = "ok";
    const sent = state.lastSentCount || 0;
    const ago =
      state.lastSentAt > 0
        ? `${Math.round((Date.now() - state.lastSentAt) / 1000)}s ago`
        : "never";
    status.innerHTML =
      `✓ Native host connected (port 9157).<br>` +
      `Sent <b>${sent}</b> ${sent === 1 ? "request" : "requests"} (last: ${ago}).`;
  } else {
    status.className = "err";
    status.innerHTML =
      `✗ Native host not running.<br>` +
      `<small>${(state.lastError || "Start DM, then reload the extension.").toString()}</small>`;
  }
}

// Probe the background service worker for current state.
async function refresh() {
  try {
    const res = await chrome.runtime.sendMessage({ type: "get-status" });
    if (res && res.state) setStatus(res.state);
  } catch (e) {
    setStatus({ connected: false, lastError: "background not reachable" });
  }
}
refresh();
setInterval(refresh, 1500);

document.getElementById("grab").addEventListener("click", async () => {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab) return;
  chrome.runtime.sendMessage({ type: "grab", tabId: tab.id }, () => {
    setStatus({ connected: true, lastSentAt: Date.now(), lastSentCount: 1 });
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
        "the native host is reachable."
      );
    }
  });
});
