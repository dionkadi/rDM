# DM Browser Extension — Install Guide

The DM browser extension is an MV3 background (service worker on
Chrome / Edge / Brave / Arc, event page on Firefox 109+) that captures
media and download URLs from the pages you visit and forwards them to
the DM app over a local TCP socket (`127.0.0.1:9157`).

The pieces:

```
browser-extension/
├── manifest.json                       # Chrome / Edge / Brave / Arc + Firefox 109+ (unified)
├── background.js                       # MV3 background page
├── content.js                          # Scans every page for <video>/<source>/<a>
├── popup.html / popup.js               # Toolbar popup with host status + "Test host connection"
├── icons/icon.svg                      # Shared icon (rasterize for production)
├── com.app.dm.native.json              # Cross-browser host manifest template
│                                       #   (use .chrome.json or .firefox.json instead for clean per-browser installs)
├── com.app.dm.native.chrome.json       # Chrome-only template (uses `allowed_origins`)
└── com.app.dm.native.firefox.json      # Firefox-only template (uses `allowed_extensions`,
                                        #   no placeholder chrome-extension:// URL that some
                                        #   Firefox versions mis-parse)
```

## 1. Build the native-messaging host

```bash
cargo build -p dm-native-host --release
# → target/release/dm-native-host
```

## 2. Register the native-messaging host

The host uses the standard Chrome / Firefox discovery paths.
**Use the browser-specific template** (`com.app.dm.native.chrome.json` for
Chrome-family browsers, `com.app.dm.native.firefox.json` for Firefox) — the
shared `com.app.dm.native.json` template carries the `REPLACE_WITH_…`
placeholder for Chrome and is convenient, but a clean per-browser file
is harder to mis-edit.

### Linux (Chrome / Edge / Brave / Arc)

```bash
# Use the Chrome-specific template (no `allowed_extensions` array, no
# Firefox-only fields). The `path` is a placeholder; the
# `allowed_origins` line MUST be replaced with your actual Chrome
# extension ID (a 32-character string from `chrome://extensions`).
cp browser-extension/com.app.dm.native.chrome.json \
   ~/.config/google-chrome/NativeMessagingHosts/com.app.dm.native.json
$EDITOR ~/.config/google-chrome/NativeMessagingHosts/com.app.dm.native.json
# set: "path": "/absolute/path/to/target/release/dm-native-host"
# set: "allowed_origins": ["chrome-extension://<YOUR 32-CHAR ID>/"]

# Repeat for Chromium / Brave if you use them:
cp browser-extension/com.app.dm.native.chrome.json \
   ~/.config/chromium/NativeMessagingHosts/com.app.dm.native.json
cp browser-extension/com.app.dm.native.chrome.json \
   ~/.config/BraveSoftware/Brave-Browser/NativeMessagingHosts/com.app.dm.native.json
```

### macOS

```bash
mkdir -p ~/Library/Application\ Support/Google/Chrome/NativeMessagingHosts
cp browser-extension/com.app.dm.native.chrome.json \
   ~/Library/Application\ Support/Google/Chrome/NativeMessagingHosts/com.app.dm.native.json
$EDITOR ~/Library/Application\ Support/Google/Chrome/NativeMessagingHosts/com.app.dm.native.json
```

### Windows (PowerShell)

```powershell
New-Item -ItemType Directory -Force -Path "$env:LOCALAPPDATA\Google\Chrome\User Data\NativeMessagingHosts"
Copy-Item browser-extension\com.app.dm.native.chrome.json `
    "$env:LOCALAPPDATA\Google\Chrome\User Data\NativeMessagingHosts\com.app.dm.native.json"
notepad "$env:LOCALAPPDATA\Google\Chrome\User Data\NativeMessagingHosts\com.app.dm.native.json"
# set "path" to C:\\path\\to\\dm-native-host.exe (escape backslashes)
```

### Firefox

**Important:** use `com.app.dm.native.firefox.json`, not the Chrome
template. Firefox's host-manifest schema only knows about
`allowed_extensions` (not `allowed_origins`); mixing both fields in
the same manifest works in current Firefox but a placeholder
`chrome-extension://REPLACE_WITH…` URL in `allowed_origins` is
needless noise. The Firefox template is preconfigured with
`dm-grabber@dm-project` in `allowed_extensions` — no ID
substitution needed.

```bash
mkdir -p ~/.mozilla/native-messaging-hosts
cp browser-extension/com.app.dm.native.firefox.json \
   ~/.mozilla/native-messaging-hosts/com.app.dm.native.json
$EDITOR ~/.mozilla/native-messaging-hosts/com.app.dm.native.json
# set: "path": "/absolute/path/to/target/release/dm-native-host"
```

## 3. Install the extension itself

### Chrome / Edge / Brave / Arc

1. Open `chrome://extensions` (or `edge://extensions`, `brave://extensions`)
2. Toggle **Developer mode** (top right)
3. Click **Load unpacked** and pick `browser-extension/`
4. **Copy the Extension ID** (a 32-character string like
   `abcdefghijklmnopqrstuvwxyzabcdef` under the extension title).
   You will need it in step 5.
5. **Edit the host manifest** (`com.app.dm.native.json` — the one you
   copied into `~/.config/google-chrome/NativeMessagingHosts/` in step 2)
   and replace the string
   `chrome-extension://REPLACE_WITH_YOUR_CHROME_EXTENSION_ID/`
   in the `allowed_origins` array with
   `chrome-extension://<the ID you copied in step 4>/`.
   This is the most commonly-skipped step; if you skip it the popup
   will show `Native host not running` and the service worker
   console will log `Access to the specified native messaging host
   is blocked`. **Chrome refuses to start a host whose manifest
   does not list the calling extension's ID in `allowed_origins`.**
6. Restart the browser (so Chrome re-reads the host manifest).

### Firefox

1. Open `about:debugging#/runtime/this-firefox`
2. Click **Load Temporary Add-on…** and select
   `browser-extension/manifest.json`. The manifest carries a
   `browser_specific_settings.gecko` block, so Firefox reads it as a
   native add-on: the background runs as a non-persistent event page
   (MV3 `background.scripts`), and the extension ID is fixed to
   `dm-grabber@dm-project` — no ID substitution needed in the host
   manifest.
3. **Important**: load the source `browser-extension/manifest.json`,
   not any copy that has been modified.
4. After loading, click the DM Grabber toolbar icon. If the popup
   shows "Native host not running", click the new **Test host
   connection** button in the popup to force a fresh `connectNative`
   and see the precise `chrome.runtime.lastError.message` from
   Firefox (the error text is also shown in a red error block in the
   popup, with a hint pointing at the most likely cause).

## 4. Verify the connection

1. Make sure the DM app is running (it binds `127.0.0.1:9157` on startup)
2. Click the DM Grabber toolbar icon — the popup should show
   **"✓ Native host connected (port 9157)"**
3. Open the DM app → **Settings → Extensions** — the status panel should
   show the same: **Ready**, with "Last activity" updating whenever the
   extension pushes a URL
4. Visit any page with `<video>` (e.g. a Twitch clip) and the URL should
   land in the DM queue within ~1 s

## Troubleshooting

The popup has a **Test host connection** button that forces a fresh
`connectNative` and renders the actual `chrome.runtime.lastError.message`
verbatim, plus a hint pointing at the most likely cause. The most
common messages you'll see and what they mean:

| Popup error | What it means | Fix |
| --- | --- | --- |
| `No such native application com.app.dm.native` (Firefox) | Firefox couldn't find a valid host manifest for the extension | Confirm `~/.mozilla/native-messaging-hosts/com.app.dm.native.json` exists, is valid JSON, and the extension ID `dm-grabber@dm-project` is in its `allowed_extensions` array (use the bundled `com.app.dm.native.firefox.json` template). |
| `Specified native messaging host not found.` (Chrome) | The host manifest is missing or its `path` is unreachable | Verify the manifest is in the right OS-specific directory and the binary at the `path` is absolute and executable. |
| `Access to the specified native messaging host is blocked.` (Chrome) | The Chrome extension ID is not in the host's `allowed_origins` | Copy the actual ID from `chrome://extensions` and edit the host manifest. Restart Chrome to re-read the manifest. |
| `"path" is not absolute` (manifest rejected) | Relative path in the host manifest | Use an absolute path; on Windows escape backslashes (`"C:\\path\\to\\host.exe"`) |
| Extension installed but downloads don't appear | Either the host isn't reachable or the extension is mis-configured | Click **Test host connection** in the popup and read the red error block — it tells you which file/path/ID is wrong. |
| Firefox rejects the extension | Missing `browser_specific_settings` | `manifest.json` already carries the `gecko` block; if you see this, make sure you loaded the source `browser-extension/manifest.json`, not a copy that has been modified. |
| The popup says "background not reachable" | The extension's background event page is not running yet | Give Firefox ~1 s after page load; the popup polls every 1.5 s. |

## Permissions the extension asks for

| Permission | Why |
| --- | --- |
| `nativeMessaging` | Talk to the DM host binary |
| `downloads` | Intercept real browser downloads ("Save link as") |
| `tabs` | Look up the active tab when you click the popup |
| `activeTab` | Run content scripts on the page you actually visit |
| `scripting` | Reserved for future "inject grab button" features |
| `storage` | Reserved for the future "always-grab these sites" list |
| `<all_urls>` (host) | The content script can run on any page so it can scan whatever media is there |

All of these are minimal; no telemetry, no remote endpoints, no
auto-update channel.
