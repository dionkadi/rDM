// Is anything actually speaking WebSocket on the DM port?
//
// The Chrome extension reaches DM over `ws://127.0.0.1:9157/`. When it
// can't, the browser only ever tells us "closed with code 1006" — which
// is the *same* message whether nothing is listening, whether an old
// pre-0.4.2 DM build is listening (it spoke only line-delimited JSON and
// will silently swallow the upgrade request), whether the listener
// accepts the connection and then drops it, or whether the handshake
// itself is broken. That ambiguity is why "the app is running but the
// extension won't connect" is hard to pin down.
//
// This probe removes the ambiguity by doing what the browser does:
// TCP-connect, send a real Chrome-shaped upgrade request, and check the
// `Sec-WebSocket-Accept` is the correct SHA-1 digest. It then sends a
// close frame, so it exercises the frame layer too.
//
// **It sends no URLs to DM** — nothing lands in your download queue.
//
//   node scripts/check-host.mjs            # probe 127.0.0.1:9157
//   node scripts/check-host.mjs 9158       # probe another port
//   node scripts/check-host.mjs --self-test  # verify the probe itself

import net from "node:net";
import crypto from "node:crypto";
import { isMainModule } from "./lib/is-main.mjs";

const WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

export const EXPECTED_ACCEPT = (key) =>
  crypto
    .createHash("sha1")
    .update(key + WS_GUID)
    .digest("base64");

/// Build the exact request Chromium sends for `new WebSocket(url)`,
/// minus the headers it adds on a proxy path.
export function upgradeRequest(key, host = "127.0.0.1:9157") {
  return [
    "GET / HTTP/1.1",
    `Host: ${host}`,
    "Connection: Upgrade",
    "Pragma: no-cache",
    "Cache-Control: no-cache",
    "User-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
    "Upgrade: websocket",
    "Origin: chrome-extension://dm-test-extension",
    `Sec-WebSocket-Key: ${key}`,
    "Sec-WebSocket-Version: 13",
    "",
    "",
  ].join("\r\n");
}

/// A masked client close frame (opcode 0x8), as a browser sends it.
export function maskedCloseFrame() {
  const mask = crypto.randomBytes(4);
  const payload = Buffer.from([0x03, 0xe8]); // 1000 Normal Closure
  const header = Buffer.from([0x88, 0x80 | payload.length]);
  const masked = Buffer.from(payload.map((b, i) => b ^ mask[i & 3]));
  return Buffer.concat([header, mask, masked]);
}

/**
 * Probe `port` on `host` and classify what is listening.
 *
 * @returns {Promise<{kind: string, statusLine?: string, accept?: string,
 *   expected?: string, frameLayer?: string, message?: string}>}
 *   kind: refused | unreachable | no-response | closed | http-error
 *       | bad-accept | ok | error
 *
 * `no-response` and `closed` are deliberately separate. Both look
 * identical from a browser (`close code 1006`), but they have different
 * causes: `no-response` is something that reads the request and stays
 * silent (the pre-0.4.2 line-JSON listener), while `closed` is a server
 * that accepts the connection and then hangs up without a byte of HTTP —
 * the signature of a listener that mis-dispatched the request or could
 * not read from the socket, which is what the Windows/Winsock
 * non-blocking `accept()` bug did to every connection.
 */
export function probe(port, { host = "127.0.0.1", timeoutMs = 3000 } = {}) {
  return new Promise((resolve) => {
    const key = crypto.randomBytes(16).toString("base64");
    const expected = EXPECTED_ACCEPT(key);
    let settled = false;
    let buf = "";
    let sawHandshake = false;
    let tcpConnected = false;

    const finish = (verdict) => {
      if (settled) return;
      settled = true;
      try {
        socket.destroy();
      } catch (_e) {
        void _e;
      }
      clearTimeout(timer);
      resolve(verdict);
    };

    const socket = net.connect({ host, port });
    const timer = setTimeout(() => {
      if (!tcpConnected) {
        // The SYN never completed: nothing is there (or it is filtered).
        return finish({ kind: "unreachable", message: "TCP connect timed out" });
      }
      // Connected, but the server never answered the upgrade request.
      // This is the pre-0.4.2 DM listener signature.
      return finish({ kind: "no-response" });
    }, timeoutMs);

    socket.on("error", (err) => {
      if (err.code === "ECONNREFUSED") return finish({ kind: "refused" });
      if (err.code === "EHOSTUNREACH" || err.code === "ENETUNREACH") {
        return finish({ kind: "unreachable" });
      }
      return finish({ kind: "error", message: `${err.code || ""} ${err.message}` });
    });

    socket.on("connect", () => {
      tcpConnected = true;
      socket.write(upgradeRequest(key, `${host}:${port}`));
    });

    socket.on("data", (chunk) => {
      if (!sawHandshake) {
        buf += chunk.toString("utf8");
        if (!buf.includes("\r\n\r\n")) return; // headers still arriving
        sawHandshake = true;
        const [head] = buf.split("\r\n\r\n");
        const statusLine = head.split("\r\n")[0] || "";
        const accept = /sec-websocket-accept:\s*(\S+)/i.exec(head)?.[1] || null;

        if (!/^HTTP\/1\.1 101/.test(statusLine)) {
          return finish({ kind: "http-error", statusLine: statusLine.trim() });
        }
        if (!accept) {
          return finish({
            kind: "bad-accept",
            statusLine: statusLine.trim(),
            accept: null,
            expected,
          });
        }
        if (accept !== expected) {
          return finish({
            kind: "bad-accept",
            statusLine: statusLine.trim(),
            accept,
            expected,
          });
        }
        // Handshake is good. Prove the frame layer too: a browser sends
        // masked frames, and the server must accept one and close.
        try {
          socket.write(maskedCloseFrame());
        } catch (_e) {
          void _e;
        }
        return;
      }
      // Anything after the handshake is the server's close frame.
      finish({ kind: "ok", statusLine: "HTTP/1.1 101 Switching Protocols", frameLayer: "close-frame" });
    });

    socket.on("end", () => {
      if (!sawHandshake) {
        // The server hung up without answering: accepted and dropped.
        finish({ kind: "closed" });
      } else {
        finish({ kind: "ok", statusLine: "HTTP/1.1 101 Switching Protocols", frameLayer: "eof" });
      }
    });

    socket.on("close", () => {
      if (!settled) {
        finish(
          sawHandshake
            ? { kind: "ok", statusLine: "HTTP/1.1 101 Switching Protocols", frameLayer: "closed" }
            : { kind: "closed" },
        );
      }
    });
  });
}

// ─── Self-test ──────────────────────────────────────────────────
//
// A probe that can't tell these cases apart is worse than no probe, so
// verify it against four fake listeners: nothing, a pre-WebSocket
// listener that reads and never answers, a compliant WS server, and a
// server that answers 101 with the wrong accept key.

function startServer(handler) {
  return new Promise((resolve) => {
    const server = net.createServer(handler);
    server.listen(0, "127.0.0.1", () => {
      resolve({ server, port: server.address().port });
    });
  });
}

function wsServer({ wrongAccept = false } = {}) {
  return (socket) => {
    let answered = false;
    socket.on("data", (chunk) => {
      if (answered) {
        socket.end(Buffer.from([0x88, 0x00]));
        return;
      }
      const text = chunk.toString("utf8");
      const m = /sec-websocket-key:\s*(\S+)/i.exec(text);
      if (!m) return;
      answered = true;
      const accept = wrongAccept ? "AAAAAAAAAAAAAAAAAAAAAAAAAAA=" : EXPECTED_ACCEPT(m[1]);
      socket.write(
        "HTTP/1.1 101 Switching Protocols\r\n" +
          "Upgrade: websocket\r\n" +
          "Connection: Upgrade\r\n" +
          `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
      );
    });
  };
}

/** A pre-0.4.2 DM listener: line-delimited JSON, no HTTP anywhere. */
function legacyJsonListener(socket) {
  socket.on("data", () => {
    // Parses the `GET / HTTP/1.1` line as JSON, fails, and waits for a
    // line it understands. The upgrade request is simply swallowed.
  });
}

/**
 * The Windows/Winsock signature: the connection is accepted and then
 * dropped without any answer at all. `probe-listener --winsock
 * --no-blocking-reset` produces exactly this.
 */
function dropListener(socket) {
  socket.on("data", () => {
    socket.destroy();
  });
}

async function selfTest() {
  const results = [];
  const noListener = await startServer(() => {});
  const deadPort = noListener.port;
  noListener.server.close();

  const legacy = await startServer(legacyJsonListener);
  const dropper = await startServer(dropListener);
  const good = await startServer(wsServer());
  const bad = await startServer(wsServer({ wrongAccept: true }));

  const cases = [
    ["refused", await probe(deadPort, { timeoutMs: 1000 }), "refused"],
    ["no-response", await probe(legacy.port, { timeoutMs: 800 }), "no-response"],
    ["closed", await probe(dropper.port, { timeoutMs: 1500 }), "closed"],
    ["ok", await probe(good.port, { timeoutMs: 1500 }), "ok"],
    ["bad-accept", await probe(bad.port, { timeoutMs: 1500 }), "bad-accept"],
  ];

  for (const [label, verdict, expectedKind] of cases) {
    const pass = verdict.kind === expectedKind;
    results.push({ label, pass, verdict });
    console.log(
      `  ${pass ? "ok    " : "FAIL  "} ${label.padEnd(12)} -> ${verdict.kind}` +
        (verdict.statusLine ? ` (${verdict.statusLine})` : ""),
    );
  }

  legacy.server.close();
  dropper.server.close();
  good.server.close();
  bad.server.close();

  const failed = results.filter((r) => !r.pass);
  console.log(
    failed.length === 0
      ? `\nall ${results.length} probe cases classified correctly\n`
      : `\n${failed.length}/${results.length} probe cases WRONG\n`,
  );
  return failed.length === 0;
}

function report(port, verdict) {
  const banner = (t) => `\n${t}\n${"─".repeat(t.length)}\n`;
  const win = process.platform === "win32";
  const whoHasThePort = win
    ? "  netstat -ano | findstr :" + port
    : "  ss -ltnp | grep " + port;
  const reservedRanges = win
    ? "\n  Windows also reserves blocks of ports for Hyper-V / WSL / Docker.\n" +
      "  If your port falls in one, bind() can never succeed:\n" +
      "    netsh int ipv4 show excludedportrange protocol=tcp\n"
    : "";
  const logDir = win ? "%APPDATA%\\dm\\logs" : "$XDG_DATA_HOME/dm/logs (or ~/.local/share/dm/logs)";
  switch (verdict.kind) {
    case "ok":
      console.log(banner(`OK — 127.0.0.1:${port} is a working WebSocket server`));
      console.log(
        "The DM app answered a browser-shaped upgrade request with the\n" +
          "correct Sec-WebSocket-Accept, and accepted a masked client frame.\n" +
          `Frame layer: ${verdict.frameLayer}\n\n` +
          "So the problem is on the extension side. Check the popup:\n" +
          '  * "Receiving end does not exist"  -> the extension has no background\n' +
          "    service worker. You almost certainly loaded the repo folder in\n" +
          "    Chrome instead of the released .zip. Chrome ignores\n" +
          "    background.scripts under MV3, so nothing runs.\n" +
          "  * a red \"disconnected (code 1006)\" that comes and goes -> reload\n" +
          "    the extension on chrome://extensions (an MV3 service worker does\n" +
          "    not pick up file changes on disk), then tell us and include the\n" +
          "    extension version and your browser version.",
      );
      return 0;
    case "refused":
      console.log(banner(`NOTHING IS LISTENING on 127.0.0.1:${port}`));
      console.log(
        "No process accepted a TCP connection, so the DM app is not running\n" +
          "or it failed to bind the port. Check, in this order:\n" +
          "  1. Is the DM window actually open?\n" +
          "  2. Look for a second/stale DM instance holding the port:\n" +
          `${whoHasThePort}\n` +
          reservedRanges +
          `  3. The app logs a bind failure at startup (Settings → Extensions\n` +
          `     shows it as "Port unavailable"); the session log is under\n` +
          `     ${logDir}. Grep it for "cannot bind".`,
      );
      return 2;
    case "closed":
      console.log(banner(`PORT ${port} ACCEPTS AND THEN HANGS UP`));
      console.log(
        "Something is listening: the TCP connection succeeds, and then the\n" +
          "connection is closed without a single byte of HTTP. That is the\n" +
          "signature of a listener that mis-dispatched the browser's request\n" +
          "— classically, an accepted socket left in non-blocking mode (which\n" +
          "is what a non-blocking listener gives you on Windows: Winsock\n" +
          "inherits the flag, POSIX does not), so the very first read fails\n" +
          "and is mistaken for \"the client said nothing\".\n\n" +
          "A browser sees exactly this as close code 1006.\n\n" +
          "Fix: rebuild the DM app — `ws::accept_ready` puts accepted sockets\n" +
          "back into blocking mode (0.5.x and later). Confirm the app is the\n" +
          "one on the port and check its log:\n" +
          `  ${logDir}  (grep for "dropping an accepted connection")\n\n` +
          "To reproduce it off Windows:\n" +
          "  cd src-tauri/probe-listener && cargo run -- --winsock --no-blocking-reset\n" +
          "  node browser-extension/scripts/check-host.mjs 9158   # -> closed",
      );
      return 3;
    case "no-response":
      console.log(banner(`PORT ${port} IS OPEN BUT SILENT`));
      console.log(
        "Something accepted the TCP connection and never answered the\n" +
          "WebSocket upgrade request. This is the signature of a DM build\n" +
          "older than 0.4.2: its listener only spoke line-delimited JSON, so\n" +
          "it parses the browser's `GET / HTTP/1.1` as JSON, fails, and waits.\n" +
          "The browser gives up and reports close code 1006 - which looks\n" +
          "exactly like \"the app isn't running\".\n\n" +
          "Fix: update/rebuild the DM desktop app (the WebSocket listener\n" +
          "landed in 0.4.2), then re-run this probe.\n\n" +
          "If you built from source, confirm with:\n" +
          "  git log --oneline -1 && ls -l target/release/dm-*",
      );
      return 3;
    case "http-error":
      console.log(banner(`PORT ${port} IS OPEN BUT NOT A WebSocket ENDPOINT`));
      console.log(
        `It answered: ${verdict.statusLine}\n\n` +
          "Something else is on this port, or the listener is serving plain\n" +
          `HTTP. Check what owns it:\n${whoHasThePort}`,
      );
      return 3;
    case "bad-accept":
      console.log(banner(`HANDSHAKE ANSWERED BUT WRONG`));
      console.log(
        `  server sent:      ${verdict.accept}\n` +
          `  browser expects:  ${verdict.expected}\n\n` +
          "A browser will reject this connection, so the extension can never\n" +
          "connect. That points at a bug in the server's SHA-1 accept-key\n" +
          "calculation (src-tauri/src/ws.rs). Please report this — the accept\n" +
          "key in the request is randomised, so re-running the probe will\n" +
          "produce different values, which is itself the tell.",
      );
      return 3;
    default:
      console.log(banner(`PROBE FAILED (${verdict.kind})`));
      if (verdict.message) console.log(verdict.message);
      return 4;
  }
}

if (isMainModule(import.meta.url)) {
  const args = process.argv.slice(2);
  if (args.includes("--self-test")) {
    const ok = await selfTest();
    process.exit(ok ? 0 : 1);
  }
  const port = Number(args[0] || 9157);
  if (!Number.isInteger(port) || port <= 0 || port > 65535) {
    console.error(`usage: node scripts/check-host.mjs [port] | --self-test`);
    process.exit(64);
  }
  const verdict = await probe(port);
  const code = report(port, verdict);
  console.log(
    "\n(This probe sends no URLs to DM — nothing was added to the queue.)",
  );
  process.exit(code);
}
