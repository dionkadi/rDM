# probe-listener — run the real WebSocket listener without the GUI

A throwaway-but-useful harness that runs **the actual
`src-tauri/src/ws.rs`** listener code — including the app's own
`ws::accept_ready` / `ws::peek_is_websocket` dispatch helpers — as a plain
terminal program, so you can test the browser extension's Chromium
transport without building or launching the Tauri app.

It exists because of one specific, recurring question:

> The DM app is running, the popup says it isn't. Which side is broken?

The browser can't answer that — `close code 1006` is the only thing a
WebSocket ever reports, whether nothing is listening, whether the app is
a pre-0.4.2 build that only speaks line-delimited JSON, whether the
listener accepts the connection and drops it, or whether the handshake
is broken. So verify the two sides separately:

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

## Flags: reproducing the Windows failure anywhere

```bash
cargo run -- --winsock                      # accepted sockets start
                                            #   non-blocking (Winsock),
                                            #   ws::accept_ready runs
cargo run -- --winsock --no-blocking-reset  # ...and nothing resets them
```

`--winsock` applies the state Windows' `accept()` produces for free when
the listener is non-blocking: the accepted socket inherits the flag.
POSIX never does that, which is why this class of bug only ever appeared
on Windows. `--no-blocking-reset` then skips `ws::accept_ready`, i.e.
reproduces the pre-fix app. Against that listener the probe reports
**`closed`** — the accepted connection is dropped without a byte of HTTP,
which is what Chrome shows as `disconnected (code 1006)` — and the
harness prints to its own log:

```
legacy: line-JSON path
legacy: read failed: Resource temporarily unavailable (os error 11)
```

That pair is the whole bug: the request was mis-dispatched, the read
failure was read as EOF, the connection died silently. With just
`--winsock` (the fixed code) the same probe reports `ok` and a clean
close.

## What this is verified against

`node browser-extension/scripts/check-host.mjs 9158` reports **OK** against
this listener, and a browser-shaped client (Chrome's exact handshake
headers including `Origin`, then a masked text frame carrying the
extension's payload) gets its frame delivered, an `{"ok":true}` ack, and a
clean close. If that ever stops being true, the bug is in `ws.rs`, not in
the extension.

## No more duplicated dispatch

The harness used to carry its own copy of the app's "peek the first byte,
then pick WebSocket or line-delimited JSON" logic (it couldn't import
`native_host.rs`, which depends on `tauri::AppHandle`) — a copy that could
silently drift. The dispatch now lives in `ws.rs` as `accept_ready` +
`peek_is_websocket`, and both the app and this harness call it, so there is
nothing left to keep in step.

`ws.rs` itself stays dependency-free (`std` + `sha1` + `base64`, no
`log`, no Tauri) precisely so it can be included here with `#[path]`.

This crate is deliberately **not** a member of the DM workspace (see the
`exclude` in the root `Cargo.toml`) and is not shipped in any release
artifact.
