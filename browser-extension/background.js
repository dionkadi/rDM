// MV3 service worker for the DM Download Grabber extension.
// Connects to the native-messaging host (`com.app.dm.native`) and forwards
// detected media/download URLs to the running DM app.

const HOST_NAME = "com.app.dm.native";

let port = null;

function connect() {
  if (port) return;
  try {
    port = chrome.runtime.connectNative(HOST_NAME);
    port.onMessage.addListener(() => {
      // The host may ack; nothing to act on.
    });
    port.onDisconnect.addListener(() => {
      port = null;
      if (chrome.runtime.lastError) {
        console.warn(
          "DM native host disconnected:",
          chrome.runtime.lastError.message,
        );
      }
    });
  } catch (e) {
    console.warn("DM native host connect failed:", e);
  }
}

function send(payload) {
  connect();
  if (port) {
    try {
      port.postMessage(payload);
    } catch (e) {
      console.warn("DM send failed:", e);
      port = null;
    }
  }
}

// Forward real browser downloads (e.g. "Save link as") to DM.
chrome.downloads.onCreated.addListener((item) => {
  send({ type: "download", url: item.url, filename: item.filename });
});

// Messages from content scripts / popup.
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (!msg) return;
  if (msg.type === "media" && msg.urls && msg.urls.length) {
    send({ type: "capture", url: msg.pageUrl, urls: msg.urls });
  } else if (msg.type === "grab") {
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

chrome.runtime.onInstalled.addListener(connect);
connect();
