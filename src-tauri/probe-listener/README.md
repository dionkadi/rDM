# probe-listener — run the real WebSocket listener without the GUI

A throwaway-but-useful harness that runs **the actual
`src-tauri/src/ws.rs`** listener code (plus the accept/dispatch logic from
`src-tauri/src/native_host.rs`) as a plain terminal program, so you can
test the browser extension's Chromium transport without building or
launching the Tauri app.

It exists because of one specific, recurring question:

> The DM app is running, the popup says it isn't. Which side is broken?

The browser can't answer that — `close code 1006` is the only thing a
WebSocket ever reports, whether nothing is listening, whether the app is
a pre-0.4.2 build that only speaks line-delimited JSON, or whether the
handshake is broken. So verify the two sides separately:

```bash
# 1. Run the protocol server standalone.
cd src-tauri/probe-listener
cargo run            # listens on 127.0.0.1:9158 (pass a port to override)

# 2. Ask it whether it really speaks WebSocket.
node ../../browser-extension/scripts/check-host.mjs 9158
#    -> OK — 127.0.0.1:9158 is a working WebSocket server

# 3. (optional) Point the extension at it, to test the extension end
#    without the app. Change WS_URL in browser-extension/background.js
#    to 9158, reload the extension, and click a media link.
```

Port **9158**, not 9157, on purpose: it will not collide with a running
DM app, and a stray `cargo run` can't be mistaken for the real thing.

## What this is verified against

`node browser-extension/scripts/check-host.mjs 9158` reports **OK** against
this listener, and a browser-shaped client (Chrome's exact handshake
headers including `Origin`, then a masked text frame carrying the
extension's real payload) gets its frame delivered, an `{"ok":true}` ack
back, and a clean close. If that ever stops being true, the bug is in
`ws.rs`, not in the extension.

## Caveat: the dispatch here is a *copy*

`native_host.rs` owns the real "peek the first byte, then pick WebSocket
or line-delimited JSON" decision, but it can't be imported here — it
depends on `tauri::AppHandle`. The dispatch in `src/main.rs` mirrors it.
If you change the real one, change this one; this harness is a debugging
aid, not the source of truth.

This crate is deliberately **not** a member of the DM workspace (see the
`exclude` in the root `Cargo.toml`) and is not shipped in any release
artifact.
