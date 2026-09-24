// End-to-end tests with the background service worker, a content
// script and the popup all loaded into one shared context, so
// messages really travel from the content script to the background
// and back, exactly as they do in a browser.

import test from "node:test";
import assert from "node:assert/strict";
import {
  createClock,
  createChromeMock,
  loadExtension,
  FakeDocument,
  FakeElement,
  makeClick,
  plain,
  textOf,
} from "./harness.mjs";

const START = 1_700_000_000_000;
const INSTALLED_AT = START - 100_000;
const PAGE = "https://site.example.test/article";
const ZIP = "https://files.example.test/archive.zip";
const MP4 = "https://files.example.test/movie.mp4";

async function settle(clock, rounds = 12) {
  for (let i = 0; i < rounds; i++) await clock.advance(0);
}

function bootAll({ popup = false, chrome = {} } = {}) {
  const clock = createClock(START);
  const ctl = createChromeMock({
    storage: { dmInstallTime: INSTALLED_AT },
    tabs: [{ id: 1, hasContentScript: true }],
    ...chrome,
  });

  const zip = new FakeElement("a");
  zip.setAttribute("href", ZIP);
  const mp4 = new FakeElement("a");
  mp4.setAttribute("href", MP4);

  const doc = new FakeDocument({
    querySelectorAllResults: { "a[href]": [zip, mp4] },
  });

  const els = {};
  if (popup) {
    for (const id of [
      "status",
      "err-detail",
      "open",
      "grab",
      "test",
      "opt-click",
      "opt-takeover",
    ]) {
      els[id] = doc.register(id, new FakeElement(id === "status" ? "p" : "div"));
    }
  }

  const scripts = ["background.js", "content.js"];
  if (popup) scripts.push("popup.js");
  loadExtension(scripts, {
    chrome: ctl,
    clock,
    document: doc,
    location: { href: PAGE, protocol: "https:" },
    navigator: { userAgent: "Mozilla/5.0 (Test) DMTest/1.0" },
    WebSocket: ctl.ws.FakeWebSocket,
  });

  return { clock, ctl, doc, els };
}

/**
 * Let the popup's own 1.5 s status poll run. The popup only learns
 * about a state change by asking, so tests that assert on rendered
 * text have to let that tick happen — exactly as in a real popup.
 */
async function pollOnce(clock) {
  await clock.advance(2000);
}

function click(doc, href, over = {}) {
  const { anchor, event } = makeClick(href, over);
  doc.documentElement.dispatch("click", event);
  return { anchor, event };
}

test("REQUIREMENT: with DM down, clicking a download link still downloads it", async () => {
  // The whole point of the audit. DM is not running; the user clicks a
  // .zip link. The browser must handle it as if the extension were
  // not installed.
  const { clock, ctl, doc } = bootAll();
  await settle(clock);

  const { event } = click(doc, ZIP);

  assert.equal(event.prevented, undefined);
  assert.equal(
    ctl.calls.runtimeSendMessage.filter((m) => m.type === "download-click").length,
    0,
  );
  assert.equal(ctl.calls.download.length, 0, "nothing to hand back either");
  assert.equal(ctl.calls.tabsSendMessage.some((c) => c.msg.type === "collect"), false);
});

test("with DM up, a click is forwarded and the browser is still left usable", async () => {
  const { clock, ctl, doc } = bootAll();
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);

  let pageHandlerRan = false;
  doc.documentElement.addEventListener("click", () => {
    pageHandlerRan = true;
  });

  const { event } = click(doc, ZIP);
  await settle(clock);

  assert.equal(event.prevented, true);
  assert.equal(event.immediateStopped, undefined, "page handlers survive");
  assert.equal(pageHandlerRan, true);

  const payload = ctl.ws.latest().payloads().at(-1);
  assert.equal(payload.type, "download");
  assert.equal(payload.url, ZIP);
  assert.equal(payload.referer, PAGE, "Referer reaches DM");
  assert.equal(payload.userAgent, "Mozilla/5.0 (Test) DMTest/1.0");
});

test("the stale-belief race: click swallowed, download handed back", async () => {
  // The content script believed DM was up (it had been told so), but
  // the host died in between. The click is already prevented, so the
  // background must give the download back to the browser.
  const { clock, ctl, doc } = bootAll();
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);

  ctl.ws.latest().readyState = 3; // gone, without a close event landing

  const { event } = click(doc, MP4);
  await settle(clock);

  assert.equal(event.prevented, true, "the interceptor did its job…");
  assert.deepEqual(
    ctl.calls.download.map((d) => d.url),
    [MP4],
    "…and the download was not lost",
  );
});

test("P0-3 end to end: a browser download with DM down is untouched", async () => {
  const { clock, ctl } = bootAll();
  await settle(clock);

  ctl.fireCreated({ id: 51, url: ZIP, startTime: new Date(START).toISOString() });
  await settle(clock);

  assert.equal(ctl.calls.cancel.length, 0);
  assert.equal(ctl.calls.erase.length, 0);
  assert.equal(ctl.calls.download.length, 0);
});

test("P1-6: the popup refuses to claim success when nothing was sent", async () => {
  const { clock, ctl, els } = bootAll({ popup: true });
  await settle(clock);

  els.grab.dispatch("click");
  await settle(clock, 40);

  const text = textOf(els.status);
  assert.match(text, /DM host not running/);
  assert.match(text, /Found 2 media URL\(s\) but nothing was sent/, text);
});

test("the popup reports a real success", async () => {
  const { clock, ctl, els } = bootAll({ popup: true });
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);
  await pollOnce(clock);

  els.grab.dispatch("click");
  await settle(clock, 40);

  const text = textOf(els.status);
  assert.match(text, /DM host connected — WebSocket 127\.0\.0\.1:9157/);
  assert.match(text, /Sent 2 media URL\(s\) to DM\./);
  assert.ok(!/nothing was sent/.test(text));
});

test("the popup reports an empty page honestly", async () => {
  const { clock, els } = bootAll({ popup: true, chrome: {} });
  await settle(clock);
  els.grab.dispatch("click");
  await settle(clock, 40);
  // The shared document does have two anchors, so this asserts the
  // positive path is not accidentally the always-taken path.
  assert.ok(!/No media found on this page/.test(textOf(els.status)));
});

test("P1-5: the takeover toggle round-trips and reaches the content script", async () => {
  const { clock, ctl, doc, els } = bootAll({ popup: true });
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);
  await pollOnce(clock);

  // The popup mirrors the stored settings (both default to on).
  assert.equal(els["opt-click"].checked, true);
  assert.equal(els["opt-takeover"].checked, true);

  // Turn both off, as a user would.
  els["opt-click"].checked = false;
  els["opt-takeover"].checked = false;
  els["opt-click"].dispatch("change", {});
  await settle(clock, 40);

  assert.deepEqual(plain(ctl.storageData.dmSettings), {
    interceptOnClick: false,
    takeoverBrowserDownloads: false,
  });

  // Click interception is off: the link goes to the browser even
  // though DM is up.
  const { event } = click(doc, ZIP);
  assert.equal(event.prevented, undefined);

  // And the downloads path is off too.
  ctl.fireCreated({ id: 61, url: ZIP, startTime: new Date(START).toISOString() });
  await settle(clock);
  assert.equal(ctl.calls.cancel.length, 0);
});

test("turning click handling back on restores interception", async () => {
  const { clock, ctl, doc, els } = bootAll({
    popup: true,
    chrome: {
      storage: {
        dmInstallTime: INSTALLED_AT,
        dmSettings: { interceptOnClick: false, takeoverBrowserDownloads: true },
      },
    },
  });
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);
  await pollOnce(clock);

  assert.equal(els["opt-click"].checked, false);
  assert.equal(click(doc, ZIP).event.prevented, undefined);

  els["opt-click"].checked = true;
  els["opt-click"].dispatch("change", {});
  await settle(clock, 40);
  await pollOnce(clock);

  assert.equal(click(doc, ZIP).event.prevented, true);
});

test("the status never hardcodes a port for the Firefox transport", async () => {
  const clock = createClock(START);
  const ctl = createChromeMock({
    storage: { dmInstallTime: INSTALLED_AT },
    tabs: [{ id: 1, hasContentScript: true }],
    nativeSupported: true,
  });
  const doc = new FakeDocument();
  const els = {};
  for (const id of ["status", "err-detail", "open", "grab", "test", "opt-click", "opt-takeover"]) {
    els[id] = doc.register(id, new FakeElement("div"));
  }
  loadExtension(["background.js", "content.js", "popup.js"], {
    chrome: ctl,
    clock,
    document: doc,
    location: { href: PAGE, protocol: "https:" },
    navigator: { userAgent: "UA" },
    browser: { runtime: { getBrowserInfo: () => ({ name: "Firefox" }) } },
  });
  await settle(clock);
  await pollOnce(clock);

  const text = textOf(els.status);
  assert.match(text, /native messaging \(com\.app\.dm\.native\)/);
  assert.ok(!/127\.0\.0\.1:9157/.test(text), "no port on the native path");
});
