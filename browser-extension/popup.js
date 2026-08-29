// Popup UI: manually grab the current tab's media, or focus the DM app.
const status = document.getElementById("status");

document.getElementById("grab").addEventListener("click", async () => {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  chrome.runtime.sendMessage({ type: "grab", tabId: tab.id });
  status.textContent = "Sent to DM";
});

document.getElementById("open").addEventListener("click", () => {
  chrome.runtime.sendMessage({ type: "open" });
});
