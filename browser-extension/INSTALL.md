# DM Browser Extension — Install Guide

The DM browser extension is an MV3 extension that captures
media and download URLs from the pages you visit and forwards
them to the DM app over a local TCP socket (`127.0.0.1:9157`).
**For most users, the install is three steps.** Firefox has a
longer path because Firefox MV3 can't reliably open
`ws://127.0.0.1` sockets (Firefox upgrades them to `wss://` and
fails) — Firefox users run the small `dm-native-host` helper
binary instead.

The pieces (for reference):

```
browser-extension/
├── manifest.json                       # Chrome / Edge / Brave / Arc + Firefox 109+ (unified)
├── background.js                       # MV3 background page (picks transport at load time)
├── content.js                          # Scans every page for <video>/<source>/<a>
├── popup.html / popup.js               # Toolbar popup with host status + "Test host connection"
├── icons/icon.svg                      # Shared icon (rasterize for production)
├── com.app.dm.native.firefox.json      # Native-messaging host template (Firefox only)
├── INSTALL.md                          # This file
└── scripts/                            # Package scripts (.zip + .xpi); not shipped
```

## Chromium browsers — Chrome, Edge, Brave, Arc, Vivaldi, Opera

The extension talks to DM directly over WebSocket on
`ws://127.0.0.1:9157/`. **No native binary, no host manifest, no
registry edits, no extension-ID copy-paste.** Tauri already binds
that port for the legacy native host; the extension piggybacks on
the same socket.

### Steps

1. **Install the DM desktop app** (Tauri build for your platform —
   `.msi` / `.exe` on Windows, `.dmg` / `.app` on macOS, `.deb` /
   `.rpm` / `.AppImage` on Linux). Launch it once so the
   `127.0.0.1:9157` listener is up.
2. **Download the extension bundle** —
   `dm-grabber-<version>.zip` from the latest GitHub release.
   Unzip it anywhere (a permanent location is fine; the
   extension stays loaded even if you move the folder).
3. **Load it as an unpacked extension**:
   - Chrome: `chrome://extensions`
   - Edge: `edge://extensions`
   - Brave: `brave://extensions`
   - Arc: `arc://extensions`
   - Vivaldi: `vivaldi://extensions`
   - Opera: `opera://extensions`
   Then: toggle **Developer mode** (top right) → **Load unpacked** →
   pick the unzipped `browser-extension/` folder.

That's it. The toolbar icon should show a green "✓ Native host
connected" pill within a second. **There is no step 4.** No
extension ID to copy, no manifest to register, no native binary
to install.

> **Tip:** the extension does not auto-update. To pick up a new
> version, pull the new `dm-grabber-<version>.zip`, unzip it over
> the old folder, and click **Reload** on the extension card in
> `chrome://extensions`. Your settings and install-time filter
> are stored in `chrome.storage.local` and survive the reload.

## Firefox 109+

Firefox MV3 cannot open a `ws://127.0.0.1` connection (Firefox
upgrades insecure `ws://` requests to `wss://` and the connection
fails silently). DM ships a tiny **`dm-native-host`** Rust binary
that bridges stdio JSON to the same `127.0.0.1:9157` socket; the
extension talks to it via Chrome's native-messaging protocol.

### Steps

1. **Install the DM desktop app**. Launch it once.
2. **Build the native host** (or download a prebuilt one with
   your release):

   ```bash
   cargo build -p dm-native-host --release
   # → target/release/dm-native-host
   ```

3. **Place the host manifest** for Firefox (this is the
   `com.app.dm.native.firefox.json` template, with the path
   edited to your binary):

   ```bash
   mkdir -p ~/.mozilla/native-messaging-hosts
   cp browser-extension/com.app.dm.native.firefox.json \
      ~/.mozilla/native-messaging-hosts/com.app.dm.native.json
   $EDITOR ~/.mozilla/native-messaging-hosts/com.app.dm.native.json
   # set: "path": "/absolute/path/to/target/release/dm-native-host"
   ```

   The Firefox template is preconfigured with
   `dm-grabber@dm-project` in `allowed_extensions` — no ID
   substitution needed.
4. **Load the extension**:
   - Open `about:debugging#/runtime/this-firefox`
   - Click **Load Temporary Add-on…** and select
     `browser-extension/manifest.json`. The manifest carries a
     `browser_specific_settings.gecko` block, so Firefox reads
     it as a native add-on: the background runs as a
     non-persistent event page (MV3 `background.scripts`), and
     the extension ID is fixed to `dm-grabber@dm-project`.
5. The toolbar icon should show "✓ Native host connected"
   within a second.

> **Note:** Firefox "Load Temporary Add-on" installs are erased
> on browser restart. For a permanent install, package the
> extension as `.xpi` (`./scripts/package.sh <version>`) and
> install via `about:addons` → gear icon → "Install Add-on From
> File…". For AMO-signed distribution, submit through the
> Firefox Add-ons site.

## Verify the connection

1. Make sure the DM app is running (it binds `127.0.0.1:9157` on
   startup).
2. Click the DM Grabber toolbar icon — the popup should show
   **"✓ Native host connected (port 9157)"**.
3. Open the DM app → **Settings → Extensions** — the status
   panel should show the same: **Ready**, with "Last activity"
   updating whenever the extension pushes a URL.
4. Visit any page with `<video>` (e.g. a Twitch clip) and the
   URL should land in the DM queue within ~1 s.

## Troubleshooting

The popup has a **Test host connection** button that forces a
fresh connection attempt and renders the actual
`chrome.runtime.lastError.message` verbatim, plus a hint
pointing at the most likely cause.

| Popup error | What it means | Fix |
| --- | --- | --- |
| `disconnected (code 1006)` | The WebSocket to `ws://127.0.0.1:9157/` was closed unexpectedly | Make sure the DM app is running. The extension auto-reconnects with backoff, so this should clear within 5 s. |
| `WebSocket ctor failed` | The browser refused to open a WebSocket at all | Confirm `host_permissions` in `manifest.json` includes `ws://127.0.0.1:9157/*` and reload the extension. |
| `No such native application com.app.dm.native` (Firefox) | Firefox couldn't find a valid host manifest for the extension | Confirm `~/.mozilla/native-messaging-hosts/com.app.dm.native.json` exists, is valid JSON, and lists `dm-grabber@dm-project` in `allowed_extensions`. |
| `Specified native messaging host not found.` (Firefox) | The host manifest's `path` is unreachable | Verify the binary at the `path` is absolute and executable. |
| The popup flashes a red error for ~1 s on first open, then recovers | MV3 service worker cold start | Expected behaviour: the popup retries `sendMessage` with backoff for ~680 ms before surfacing any error. If the error persists past 1 s, the service worker is genuinely unreachable. |
| Extension installed but downloads don't appear | Either the host isn't reachable or the extension is mis-configured | Click **Test host connection** in the popup and read the red error block — it tells you which file/path/ID is wrong. |
| Firefox rejects the extension | Missing `browser_specific_settings` | `manifest.json` already carries the `gecko` block; if you see this, make sure you loaded the source `browser-extension/manifest.json`, not a copy that has been modified. |

## Permissions the extension asks for

| Permission | Why |
| --- | --- |
| `downloads` | Intercept real browser downloads ("Save link as") |
| `tabs` | Look up the active tab when you click the popup |
| `activeTab` | Run content scripts on the page you actually visit |
| `scripting` | Reserved for future "inject grab button" features |
| `storage` | Reserved for the future "always-grab these sites" list |
| `<all_urls>` (host) | The content script can run on any page so it can scan whatever media is there |
| `ws://127.0.0.1:9157/*` (host) | Open a WebSocket to the running DM app |

All of these are minimal; no telemetry, no remote endpoints, no
auto-update channel.
