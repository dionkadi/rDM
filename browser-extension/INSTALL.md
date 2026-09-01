# DM Browser Extension — Install Guide

The DM browser extension is an MV3 service worker (Chrome / Edge / Brave /
Arc, plus Firefox 109+) that captures media and download URLs from the
pages you visit and forwards them to the DM app over a local TCP socket
(`127.0.0.1:9157`).

The pieces:

```
browser-extension/
├── manifest.json               # Chrome / Edge / Brave / Arc
├── manifest.firefox.json       # Firefox 109+ (uses Gecko id + event pages)
├── background.js               # MV3 service worker (Chrome) / event page (Firefox)
├── content.js                  # Scans every page for <video>/<source>/<a>
├── popup.html / popup.js       # Toolbar popup with host status
├── icons/icon.svg              # Shared icon (rasterize for production)
└── com.app.dm.native.json      # Native-messaging host manifest (template)
```

## 1. Build the native-messaging host

```bash
cargo build -p dm-native-host --release
# → target/release/dm-native-host
```

## 2. Register the native-messaging host

The host uses the standard Chrome / Firefox discovery paths.

### Linux (Chrome / Edge / Brave / Arc)

```bash
# Copy the host manifest and edit the `path` to point at the binary
mkdir -p ~/.config/google-chrome/NativeMessagingHosts
cp browser-extension/com.app.dm.native.json \
   ~/.config/google-chrome/NativeMessagingHosts/com.app.dm.native.json
$EDITOR ~/.config/google-chrome/NativeMessagingHosts/com.app.dm.native.json
# set: "path": "/absolute/path/to/target/release/dm-native-host"

# Repeat for Chromium / Brave if you use them:
mkdir -p ~/.config/chromium/NativeMessagingHosts
cp browser-extension/com.app.dm.native.json \
   ~/.config/chromium/NativeMessagingHosts/

mkdir -p ~/.config/BraveSoftware/Brave-Browser/NativeMessagingHosts
cp browser-extension/com.app.dm.native.json \
   ~/.config/BraveSoftware/Brave-Browser/NativeMessagingHosts/
```

### macOS

```bash
mkdir -p ~/Library/Application\ Support/Google/Chrome/NativeMessagingHosts
cp browser-extension/com.app.dm.native.json \
   ~/Library/Application\ Support/Google/Chrome/NativeMessagingHosts/
$EDITOR ~/Library/Application\ Support/Google/Chrome/NativeMessagingHosts/com.app.dm.native.json
```

### Windows (PowerShell)

```powershell
New-Item -ItemType Directory -Force -Path "$env:LOCALAPPDATA\Google\Chrome\User Data\NativeMessagingHosts"
Copy-Item browser-extension\com.app.dm.native.json `
    "$env:LOCALAPPDATA\Google\Chrome\User Data\NativeMessagingHosts\"
notepad "$env:LOCALAPPDATA\Google\Chrome\User Data\NativeMessagingHosts\com.app.dm.native.json"
# set "path" to C:\\path\\to\\dm-native-host.exe (escape backslashes)
```

### Firefox

```bash
mkdir -p ~/.mozilla/native-messaging-hosts
cp browser-extension/com.app.dm.native.json \
   ~/.mozilla/native-messaging-hosts/
$EDITOR ~/.mozilla/native-messaging-hosts/com.app.dm.native.json
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
   `browser-extension/manifest.firefox.json` (Firefox will use the
   `firefox`-specific manifest if you point it at that file; otherwise
   load `manifest.json` and Firefox will accept the Chrome manifest too
   on Firefox 109+)
3. The extension ID for Firefox is fixed to `dm-grabber@dm-project` (set
   in `manifest.firefox.json`) — no ID substitution needed in the host
   manifest. If you want to use `manifest.json` instead, copy its
   `browser_specific_settings.gecko.id` value into the host's
   `allowed_extensions` array.

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

| Symptom | Cause | Fix |
|---|---|---|
| Popup says "Native host not running" | DM app isn't running, or the `path` is wrong | Start DM; double-check the absolute `path` in the host manifest |
| Console shows `specified native messaging host not found` | The browser looked in the wrong location | Verify the manifest is in the right OS-specific directory (see above) |
| Manifest rejected (`"path" is not absolute`) | Relative path | Use an absolute path; on Windows escape backslashes (`"C:\\path\\to\\host.exe"`) |
| Extension installed but downloads don't appear | The Extension ID doesn't match the host's `allowed_origins` | Copy the actual ID from `chrome://extensions` and edit the host manifest |
| Firefox rejects the extension | Missing `browser_specific_settings` | Use `manifest.firefox.json` (or add the `gecko` block) |

## Permissions the extension asks for

| Permission | Why |
|---|---|
| `nativeMessaging` | Talk to the DM host binary |
| `downloads` | Intercept real browser downloads ("Save link as") |
| `tabs` | Look up the active tab when you click the popup |
| `activeTab` | Run content scripts on the page you actually visit |
| `scripting` | Reserved for future "inject grab button" features |
| `storage` | Reserved for the future "always-grab these sites" list |
| `<all_urls>` (host) | The content script can run on any page so it can scan whatever media is there |

All of these are minimal; no telemetry, no remote endpoints, no
auto-update channel.
