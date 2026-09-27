//! Runs the **real** `src-tauri/src/ws.rs` listener as a terminal
//! program, using the **real** helpers the app's listener dispatches with
//! (`ws::accept_ready` + `ws::peek_is_websocket`).
//!
//! Why: the browser reports `close code 1006` for every possible way a
//! WebSocket can fail, so "the app is running but the extension won't
//! connect" is undecidable from the browser side. This lets you test the
//! server half on its own, without building or launching the Tauri app.
//!
//!   cargo run                                  # 127.0.0.1:9158
//!   cargo run -- 9159                          # another port
//!   cargo run -- --winsock                     # accepted sockets
//!                                              #   non-blocking, like
//!                                              #   Winsock hands them out
//!   cargo run -- --winsock --no-blocking-reset # the Windows failure mode
//!
//! Then: `node ../../browser-extension/scripts/check-host.mjs 9158`
//!
//! `--winsock --no-blocking-reset` is the important one: it reproduces
//! **the Windows bug** — a non-blocking accepted socket, with nothing
//! putting it back into blocking mode — on Linux/macOS, where it would
//! otherwise be unobservable. Against that listener the probe reports
//! `closed` (the connection is accepted and dropped, which is what Chrome
//! shows as `disconnected (code 1006)`), while plain `--winsock` reports
//! `ok`, because `ws::accept_ready` is what fixes it.

// `#[path]` on a crate root resolves relative to the crate root's
// directory (`probe-listener/src/`), not to this file — hence the two
// hops up to `src-tauri/`.
#[path = "../../src/ws.rs"]
mod ws;

use std::io::{self, BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::thread;

/// Port 9158 by default: never collides with a running DM app (9157), so
/// a stray `cargo run` can't be mistaken for the real listener.
const DEFAULT_PORT: u16 = 9158;

#[derive(Clone, Copy)]
struct Options {
    port: u16,
    /// Make accepted sockets non-blocking, the way Winsock does on its own
    /// when the listener is non-blocking. On POSIX `accept()` hands back a
    /// blocking socket, so the harness has to ask explicitly.
    winsock: bool,
    /// Whether to put accepted sockets back into blocking mode
    /// (`ws::accept_ready`) — what the app does. `false` reproduces the
    /// pre-fix behaviour.
    blocking_reset: bool,
}

fn parse_args() -> Options {
    let mut opts = Options {
        port: DEFAULT_PORT,
        winsock: false,
        blocking_reset: true,
    };
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--winsock" => opts.winsock = true,
            "--no-blocking-reset" => opts.blocking_reset = false,
            other => match other.parse() {
                Ok(p) => opts.port = p,
                Err(_) => eprintln!("probe-listener: ignoring unknown argument `{other}`"),
            },
        }
    }
    opts
}

fn main() {
    let opts = parse_args();

    let listener = match TcpListener::bind(("127.0.0.1", opts.port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("probe-listener: bind failed on 127.0.0.1:{}: {e}", opts.port);
            std::process::exit(1);
        }
    };
    println!("dm: native-host listener on 127.0.0.1:{}", opts.port);
    if opts.winsock && !opts.blocking_reset {
        println!(
            "!! Windows failure mode: accepted sockets are non-blocking and\n\
             !! nothing resets them. The probe should report `closed`."
        );
    } else if opts.winsock {
        println!("   (accepted sockets start non-blocking, then ws::accept_ready runs)");
    }
    println!(
        "probe it with: node browser-extension/scripts/check-host.mjs {}",
        opts.port
    );

    for conn in listener.incoming().flatten() {
        thread::spawn(move || serve(conn, opts));
    }
}

fn serve(stream: TcpStream, opts: Options) {
    if opts.winsock {
        if let Err(e) = stream.set_nonblocking(true) {
            eprintln!("probe-listener: set_nonblocking failed: {e}");
            return;
        }
    }

    let prepared: io::Result<BufReader<TcpStream>> = if opts.blocking_reset {
        ws::accept_ready(stream)
    } else {
        // Exactly what the app used to do: trust whatever mode the socket
        // arrived in.
        Ok(BufReader::new(stream))
    };
    let mut reader = match prepared {
        Ok(reader) => reader,
        Err(e) => {
            eprintln!("probe-listener: dropping connection: {e}");
            return;
        }
    };

    // The dispatch is the app's own helper — this harness no longer keeps a
    // copy, so it cannot drift out of step with `native_host.rs`.
    if !ws::peek_is_websocket(&mut reader) {
        // Legacy path: the dm-native-host binary speaks line-delimited
        // JSON. A mis-dispatched connection lands here and dies, which is
        // the symptom being reproduced.
        println!("legacy: line-JSON path");
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => println!("legacy: {}", line.trim_end()),
                Err(e) => {
                    println!("legacy: read failed: {e}");
                    return;
                }
            }
        }
        return;
    }

    let mut writer = match reader.get_ref().try_clone() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("ws: failed to clone stream: {e}");
            return;
        }
    };

    match ws::handshake(&mut reader, &mut writer) {
        Ok(ws::HandshakeOutcome::WebSocket) => {}
        other => {
            println!("ws: handshake not completed: {other:?}");
            return;
        }
    }
    println!("ws: client connected");

    loop {
        match ws::read_text_frame(&mut reader) {
            Ok(Some(payload)) => {
                println!("ws: frame {payload}");
                // The real server acks so the extension knows the payload
                // was accepted; mirror that.
                let _ = ws::write_text_frame(&mut writer, r#"{"ok":true}"#);
            }
            Ok(None) => {
                let _ = ws::write_close_frame(&mut writer);
                println!("ws: clean close");
                break;
            }
            Err(e) => {
                println!("ws: disconnected: {e}");
                break;
            }
        }
    }
}
