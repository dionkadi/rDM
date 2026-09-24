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
├── manifest.json                       # Firefox 109+ template. scripts/package.sh
│                                       #   rewrites it (background.service_worker,
│                                       #   no gecko block, no nativeMessaging) for
│                                       #   the Chromium .zip — see "Manifest shapes"
├── background.js                       # MV3 background page (picks transport at load time)
├── content.js                          # Scans every page for <video>/<source>/<a>
├── popup.html / popup.js               # Toolbar popup: host status, behaviour switches,
│                                       #   "Test host connection"
├── icons/icon.svg                      # Shared icon (rasterize for production)
├── com.app.dm.native.firefox.json      # Native-messaging host template (Firefox only)
├── test/                               # Regression suite; not shipped
├── INSTALL.md                          # This file
└── scripts/                            # Package scripts (.zip + .xpi); not shipped
```

> **Manifest shapes.** The `manifest.json` in the repo is the
> **Firefox** template (`background.scripts` + `browser_specific_settings.gecko`;
> `background.service_worker` is rejected by Firefox 109-127). The
> Chromium `.zip` is not built from it directly — `scripts/package.sh`
> emits a Chrome-shaped copy with `background.service_worker`, no gecko
> block, and no `nativeMessaging` permission.
>
> This matters if you load the extension by hand: pointing Chrome at the
> repo's `browser-extension/` folder gives you a manifest with no
> background context at all (Chrome ignores `background.scripts` in MV3),
> so nothing works and the popup reports "background not reachable".
> **Always install Chromium builds from the released `.zip`.**

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
   `com.app.dm.native.firefox.json` template, with the placeholder
   `path` edited to your binary):

   ```bash
   mkdir -p ~/.mozilla/native-messaging-hosts
   cp browser-extension/com.app.dm.native.firefox.json \
      ~/.mozilla/native-messaging-hosts/com.app.dm.native.json
   $EDITOR ~/.mozilla/native-messaging-hosts/com.app.dm.native.json
   # replace the placeholder:
   #   "path": "/absolute/path/to/dm-native-host"
   # with the real absolute path, e.g.
   #   "path": "/home/you/dm/target/release/dm-native-host"
   ```

   The Firefox template is preconfigured with
   `dm-grabber@dm-project` in `allowed_extensions` — no ID
   substitution needed.

   > The `.xpi` requests the `nativeMessaging` permission. It must —
   > without it Firefox makes `runtime.connectNative` an undefined
   > function and the extension can never reach DM at all. If you
   > build the extension yourself, don't strip that permission from
   > the Firefox manifest.
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

## What the extension will and won't do to your browser

The popup has two switches, both **on** by default:

| Switch | On | Off |
| --- | --- | --- |
| **Take over media link clicks** | Clicking a `.zip` / `.pdf` / `.mp4`-style link sends it to DM instead of the browser | Those links behave exactly as they would without the extension |
| **Take over all browser downloads** | Downloads DM *didn't* get a click for — right-click "Save link as", Ctrl-click, another extension's download — are redirected to DM | DM only ever sees URLs you clicked, or grabbed from the popup |

The invariant behind both switches: **nothing is taken away from the
browser unless DM actually received the URL.**

* If the DM app isn't running, links are left completely alone — the
  browser downloads them normally. (The toolbar badge shows `!`.)
* If a URL can't be delivered mid-flight, the extension re-issues the
  download through the browser itself rather than dropping it.
* Only real user clicks are intercepted; a page's own programmatic
  clicks are not. Interception also no longer stops the event from
  reaching the page, so sites whose download flow is JS-driven keep
  working.

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

## "The DM app is running, but the extension won't connect"

This is the most confusing failure mode, because the browser gives you
almost nothing to go on. Every way the WebSocket can fail reports the
same thing — `close code 1006` — whether nothing is listening, whether
something is listening but never answers the upgrade, or whether the
handshake itself is broken.

Run the bundled probe. It does what a browser does (TCP connect, upgrade
request, verify `Sec-WebSocket-Accept`, exercise a masked frame) and tells
you which case you're in:

```bash
node browser-extension/scripts/check-host.mjs
```

**It sends no URLs to DM** — nothing lands in your download queue.

| Probe says | What's wrong | Fix |
| --- | --- | --- |
| `NOTHING IS LISTENING on 127.0.0.1:9157` | The app isn't running, or it failed to bind the port (often a stale second instance) | Start DM. Check `ss -ltnp \| grep 9157`. Grep the session log for `listener bind failed`. |
| `PORT 9157 IS OPEN BUT SILENT` | Something is listening but never answers the WebSocket upgrade — the signature of a **DM build older than 0.4.2**, whose listener only spoke line-delimited JSON | Update/rebuild the DM desktop app. The WebSocket listener landed in 0.4.2. |
| `PORT 9157 IS OPEN BUT NOT A WebSocket ENDPOINT` | Some other program owns the port | Find it with `ss -ltnp \| grep 9157` |
| `HANDSHAKE ANSWERED BUT WRONG` | The server's `Sec-WebSocket-Accept` is wrong, so browsers reject the connection | That's a bug in `src-tauri/src/ws.rs` — please report it |
| `OK — 127.0.0.1:9157 is a working WebSocket server` | The app side is fine | It's the extension side. Read the popup's message: if it says **"Extension background isn't running"** you installed the repo folder instead of the released `.zip` (Chrome ignores `background.scripts` under MV3, so there is no background at all). |

If the app side checks out and you want to test the extension *without*
the GUI, `src-tauri/probe-listener/` runs the real listener as a plain
terminal program on port 9158:

```bash
cd src-tauri/probe-listener && cargo run
node browser-extension/scripts/check-host.mjs 9158
```

## Troubleshooting

The popup has a **Test host connection** button that forces a
fresh connection attempt and renders the actual
`chrome.runtime.lastError.message` verbatim, plus a hint
pointing at the most likely cause.

| Popup error | What it means | Fix |
| --- | --- | --- |
| `disconnected (code 1006)` | Nothing completed a WebSocket handshake on `127.0.0.1:9157` | Run `node browser-extension/scripts/check-host.mjs` — it distinguishes the four causes above. While it's disconnected, media links are left to the browser. |
| `Could not establish connection. Receiving end does not exist.` | The extension has **no background service worker**, so the popup can't even ask it for status. Almost always: you loaded the repo's `browser-extension/` folder in Chrome | Install the released `dm-grabber-<version>.zip` (see the top of this file), then Reload on `chrome://extensions`. |
| `WebSocket ctor failed` | The browser refused to open a WebSocket at all | Reload the extension. This is *not* caused by `host_permissions` — extension pages aren't gated by host permissions for WebSockets. Check for a restrictive `content_security_policy`. |
| `connectNative is not a function` (Firefox) | The extension was built without the `nativeMessaging` permission | Install the released `.xpi`, or add `"nativeMessaging"` to `permissions` in the Firefox `manifest.json` and reload. |
| `No such native application com.app.dm.native` (Firefox) | Firefox couldn't find a valid host manifest for the extension | Confirm `~/.mozilla/native-messaging-hosts/com.app.dm.native.json` exists, is valid JSON, and lists `dm-grabber@dm-project` in `allowed_extensions`. |
| `Specified native messaging host not found.` (Firefox) | The host manifest's `path` is unreachable — often because the placeholder was never edited | Verify the binary at the `path` is absolute and executable. |
| The popup flashes a red error for ~1 s on first open, then recovers | MV3 service worker cold start | Expected behaviour: the popup retries `sendMessage` with backoff for ~680 ms before surfacing any error. If the error persists past 1 s, the service worker is genuinely unreachable. |
| Extension installed but downloads don't appear | Either the host isn't reachable or the extension is mis-configured | Click **Test host connection** in the popup and read the red error block — it tells you which file/path/ID is wrong. |
| "Grab page media" says *nothing was sent* | The page had media but DM was unreachable | Start the DM app and try again. Nothing was queued or lost. |
| Firefox rejects the extension | Missing `browser_specific_settings` | `manifest.json` already carries the `gecko` block; if you see this, make sure you loaded the source `browser-extension/manifest.json`, not a copy that has been modified. |

## Permissions the extension asks for

The two shipped manifests ask for different sets, because the two
transports are different:

| Permission | `.zip` (Chromium) | `.xpi` (Firefox) | Why |
| --- | --- | --- | --- |
| `downloads` | yes | yes | Take over downloads the browser starts on its own, and hand one back when DM is unreachable |
| `tabs` | yes | yes | Find the active tab from the popup, and push connection state to content scripts |
| `storage` | yes | yes | The install-time filter and the behaviour switches |
| `nativeMessaging` | — | **yes** | Firefox has no usable localhost WebSocket path, so it talks to `dm-native-host` over stdio. Without this permission `runtime.connectNative` doesn't exist and the extension cannot reach DM at all. |
| `<all_urls>` (host) | yes | yes | The content script runs on any http(s) page so it can see media links |
| `ws://127.0.0.1:9157/*` (host) | yes | yes | Declared for the WebSocket transport. Informational — see the note below. |

> Host permissions do not gate WebSocket connections made from an
> extension's own background page; the extension's `connect-src` CSP
> does, and MV3 leaves it unset. The `ws://` entry is harmless and kept
> so the intent is legible, but it is not what makes the socket work.
> What *does* matter is that the DM app is listening on `127.0.0.1:9157`.

No telemetry, no remote endpoints, no auto-update channel.
