// Regression tests for background.js — the service worker that owns
// every interception decision.
//
// Each test names the finding it locks down (see EXTENSION_AUDIT.md).
// The theme: nothing is taken away from the browser unless DM
// actually received it.

import test from "node:test";
import assert from "node:assert/strict";
import {
  createClock,
  createChromeMock,
  bootBackground,
  plain,
} from "./harness.mjs";

const START = 1_700_000_000_000;
const INSTALLED_AT = START - 100_000;
const iso = (ms) => new Date(ms).toISOString();

/** Let queued promise chains run without moving the clock. */
async function settle(clock, rounds = 8) {
  for (let i = 0; i < rounds; i++) await clock.advance(0);
}

function boot(options = {}) {
  const clock = createClock(START);
  const ctl = createChromeMock({
    storage: { dmInstallTime: INSTALLED_AT },
    tabs: [{ id: 1, hasContentScript: true }],
    ...options.chrome,
  });
  bootBackground(ctl, { clock, ...options.boot });
  return { clock, ctl };
}

const ZIP = "https://files.example.test/archive.zip";

test("P1-4: downloads.onCreated is registered synchronously, before any await", () => {
  // MV3 requires event listeners at the top level of the worker. The
  // old code registered this one inside a `.then()` after reading
  // storage, which meant the event that woke the worker missed it.
  const { ctl } = boot();
  assert.equal(ctl.listeners.onCreated.length, 1);
  assert.equal(ctl.listeners.onMessage.length, 1);
});

test("host reachable: the download is forwarded and then taken over", async () => {
  const { clock, ctl } = boot();
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);

  ctl.fireCreated({ id: 7, url: ZIP, startTime: iso(START) });
  await settle(clock);

  const payloads = ctl.ws.latest().payloads();
  assert.equal(payloads.length, 1);
  assert.equal(payloads[0].type, "download");
  assert.equal(payloads[0].url, ZIP);
  assert.deepEqual(ctl.calls.cancel, [7], "browser copy cancelled only after delivery");
  assert.deepEqual(ctl.calls.removeFile, [7], "the truncated partial is removed");
  assert.deepEqual(ctl.calls.erase, [7], "and the history row is erased");
});

test("P0-3: host unreachable → the browser's download is left completely alone", async () => {
  const { clock, ctl } = boot();
  await settle(clock);
  // The socket is created but never opens: DM is not running.

  ctl.fireCreated({ id: 9, url: ZIP, startTime: iso(START) });
  await settle(clock);

  assert.equal(ctl.calls.cancel.length, 0, "must not cancel what it cannot deliver");
  assert.equal(ctl.calls.erase.length, 0);
  assert.equal(ctl.calls.download.length, 0, "the browser is already downloading it");
  assert.ok(
    ctl.logs.warn.some((line) => line.includes("leaving the browser's download in place")),
    "and says so",
  );
});

test("P1-5: a download that had already finished is never deleted from disk", async () => {
  const { clock, ctl } = boot();
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);

  ctl.fireCreated({ id: 11, url: ZIP, startTime: iso(START), state: "complete" });
  await settle(clock);

  assert.deepEqual(ctl.calls.cancel, [11]);
  assert.equal(ctl.calls.removeFile.length, 0, "complete file → removeFile must not run");
  assert.deepEqual(ctl.calls.erase, [11], "but the phantom row still goes away");
});

test("a download that predates the install is still dropped", async () => {
  const { clock, ctl } = boot();
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);

  ctl.fireCreated({ id: 13, url: ZIP, startTime: iso(INSTALLED_AT - 400_000) });
  await settle(clock);

  assert.equal(ctl.calls.cancel.length, 0);
  assert.equal(ctl.ws.latest().payloads().length, 0);
});

test("P1-4: a download that wakes a cold worker is still forwarded", async () => {
  // The download starts a few seconds *before* this worker boots, so
  // its `startTime` is earlier than any `Date.now()` taken at boot.
  // Using the boot time as a stand-in for the install time (the old
  // design) would silently drop it.
  const clock = createClock(START);
  const ctl = createChromeMock({
    storage: { dmInstallTime: INSTALLED_AT },
    tabs: [{ id: 1, hasContentScript: true }],
    holdStorageInitially: true, // storage has not resolved yet at boot
  });
  bootBackground(ctl, { clock });
  await settle(clock);

  ctl.fireCreated({ id: 21, url: ZIP, startTime: iso(START - 5_000) });
  await settle(clock);
  assert.equal(ctl.calls.cancel.length, 0, "must wait for the real install time");

  ctl.releaseStorage();
  await settle(clock);

  // It got *past* the install-time filter — the only complaint is the
  // unreachable transport, never the "predates install" drop.
  assert.ok(
    ctl.logs.warn.some((line) => line.includes("leaving the browser's download in place")),
  );
});

test("a download that predates the install is dropped even when storage is slow", async () => {
  const clock = createClock(START);
  const ctl = createChromeMock({
    storage: { dmInstallTime: INSTALLED_AT },
    tabs: [{ id: 1, hasContentScript: true }],
    holdStorageInitially: true,
  });
  bootBackground(ctl, { clock });
  await settle(clock);

  ctl.fireCreated({ id: 23, url: ZIP, startTime: iso(INSTALLED_AT - 400_000) });
  ctl.releaseStorage();
  await settle(clock);

  assert.equal(ctl.logs.warn.length, 0, "dropped quietly, as a pre-install download");
  assert.equal(ctl.calls.cancel.length, 0);
});

test("P0-2: an undeliverable click is handed back to the browser instead of vanishing", async () => {
  const { clock, ctl } = boot();
  await settle(clock);
  const url = "https://files.example.test/paper.pdf";

  const res = await new Promise((resolve) =>
    ctl.sendMessage({ type: "download-click", url, filename: "paper.pdf" }, resolve),
  );

  assert.equal(res.delivered, false);
  assert.equal(res.handedBack, true);
  assert.deepEqual(plain(ctl.calls.download), [
    { url, saveAs: false, filename: "paper.pdf" },
  ]);
});

test("the handed-back download does not loop back in through onCreated", async () => {
  const { clock, ctl } = boot();
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);
  const url = "https://files.example.test/paper.pdf";

  ctl.ws.latest().readyState = 3; // host died without the close event landing
  await new Promise((resolve) =>
    ctl.sendMessage({ type: "download-click", url, filename: "paper.pdf" }, resolve),
  );
  assert.equal(ctl.calls.download.length, 1);

  // The browser now reports that very download starting.
  ctl.fireCreated({ id: 31, url, startTime: iso(START) });
  await settle(clock);

  assert.equal(ctl.calls.cancel.length, 0, "our own fallback must not be re-intercepted");
});

test("an unsafe filename is dropped from the fallback rather than failing the download", async () => {
  const { clock, ctl } = boot();
  await settle(clock);
  const url = "https://files.example.test/nested";

  await new Promise((resolve) =>
    ctl.sendMessage(
      { type: "download-click", url, filename: "../../etc/passwd" },
      resolve,
    ),
  );

  assert.equal(ctl.calls.download.length, 1);
  assert.equal("filename" in ctl.calls.download[0], false);
});

test("the click path forwards Referer and User-Agent to DM", async () => {
  const { clock, ctl } = boot();
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);

  const url = "https://files.example.test/movie.mp4";
  const res = await new Promise((resolve) =>
    ctl.sendMessage(
      {
        type: "download-click",
        url,
        filename: "movie.mp4",
        referer: "https://paywall.example.test/watch",
        userAgent: "Mozilla/5.0 (Test) DMTest/1.0",
      },
      resolve,
    ),
  );

  assert.equal(res.delivered, true);
  const payload = ctl.ws.latest().payloads().at(-1);
  assert.equal(payload.referer, "https://paywall.example.test/watch");
  assert.equal(payload.userAgent, "Mozilla/5.0 (Test) DMTest/1.0");
});

test("P0-1: Firefox reports the missing nativeMessaging permission instead of failing silently", async () => {
  const clock = createClock(START);
  const ctl = createChromeMock({
    storage: { dmInstallTime: INSTALLED_AT },
    tabs: [{ id: 1, hasContentScript: true }],
    nativeSupported: false, // no "nativeMessaging" permission ⇒ API absent
  });
  bootBackground(ctl, {
    clock,
    browser: { runtime: { getBrowserInfo: () => ({ name: "Firefox" }) } },
  });
  await settle(clock);

  const res = await new Promise((resolve) =>
    ctl.sendMessage({ type: "get-status" }, resolve),
  );

  assert.equal(res.state.transportKind, "native");
  assert.equal(res.state.connected, false);
  assert.match(res.state.lastError, /connectNative is not a function/);
});

test("P0-1: with the permission present the Firefox path connects and delivers", async () => {
  const clock = createClock(START);
  const ctl = createChromeMock({
    storage: { dmInstallTime: INSTALLED_AT },
    tabs: [{ id: 1, hasContentScript: true }],
    nativeSupported: true,
  });
  bootBackground(ctl, {
    clock,
    browser: { runtime: { getBrowserInfo: () => ({ name: "Firefox" }) } },
  });
  await settle(clock);

  const status = await new Promise((resolve) =>
    ctl.sendMessage({ type: "get-status" }, resolve),
  );
  assert.equal(status.state.connected, true, "connectNative now succeeds");

  const url = "https://files.example.test/clip.mp4";
  const res = await new Promise((resolve) =>
    ctl.sendMessage({ type: "download-click", url, filename: "clip.mp4" }, resolve),
  );
  assert.equal(res.delivered, true);
  assert.equal(ctl.calls.connectNative[0], "com.app.dm.native");
});

test("P1-5: the takeover opt-out actually stops the takeover", async () => {
  const { clock, ctl } = boot({
    chrome: {
      storage: {
        dmInstallTime: INSTALLED_AT,
        dmSettings: { interceptOnClick: true, takeoverBrowserDownloads: false },
      },
    },
  });
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);

  ctl.fireCreated({ id: 41, url: ZIP, startTime: iso(START) });
  await settle(clock);

  assert.equal(ctl.calls.cancel.length, 0);
  assert.equal(ctl.ws.latest().payloads().length, 0);
});

test("probe reports the real state and never hangs the popup", async () => {
  const { clock, ctl } = boot();
  await settle(clock);

  let down = null;
  ctl.sendMessage({ type: "probe" }, (r) => {
    down = r;
  });
  ctl.ws.latest().simulateClose(1006);
  await clock.advance(1500);
  assert.ok(down && down.probe, "probe must always answer");
  assert.equal(down.probe.ok, false);

  let up = null;
  ctl.sendMessage({ type: "probe" }, (r) => {
    up = r;
  });
  ctl.ws.latest().simulateOpen();
  await clock.advance(1500);
  assert.ok(up && up.probe);
  assert.equal(up.probe.ok, true);
});

test("a disconnected badge state is published to content scripts", async () => {
  const { clock, ctl } = boot();
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);
  const pushes = () =>
    ctl.calls.tabsSendMessage.filter((c) => c.msg.type === "state");
  assert.equal(pushes().at(-1).msg.state.connected, true);

  ctl.ws.latest().simulateClose(1006);
  await settle(clock);
  assert.equal(pushes().at(-1).msg.state.connected, false);
});
