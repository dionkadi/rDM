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

function bootAll({ popup = false, chrome = {}, elements = {} } = {}) {
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
    querySelectorAllResults: { "a[href]": [zip, mp4], ...elements },
  });

  const els = {};
  if (popup) {
    for (const id of [
      "status",
      "err-detail",
      "host-dot",
      "host-label",
      "host-state",
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

/**
 * Drive only the popup with a specific background error, so every
 * `hintForError` branch can be exercised without depending on which
 * transport the background happened to fall back to. The popup's first
 * `get-status` probe races this listener registration (cold start), so
 * we let the 80/200/400 ms retry budget play out before asserting.
 */
async function popupWithHostError(lastError) {
  const clock = createClock(START);
  const ctl = createChromeMock({ tabs: [{ id: 1, hasContentScript: true }] });
  const doc = new FakeDocument();
  const els = {};
  for (const id of [
    "status", "err-detail", "host-dot", "host-label", "host-state",
    "open", "grab", "test", "opt-click", "opt-takeover",
  ]) {
    els[id] = doc.register(id, new FakeElement("div"));
  }
  loadExtension(["popup.js"], {
    chrome: ctl,
    clock,
    document: doc,
    location: { href: PAGE, protocol: "https:" },
    navigator: { userAgent: "UA" },
  });
  ctl.chrome.runtime.onMessage.addListener((msg, _sender, reply) => {
    if (msg && msg.type === "get-status") {
      reply({
        state: {
          connected: false,
          transportKind: "native",
          lastSentAt: 0,
          lastSentCount: 0,
          lastError,
        },
      });
      return true;
    }
    return false;
  });
  // Cold-start retries (680 ms) plus the 1.5 s poll tick.
  await clock.advance(3000);
  return els;
}

function collectTags(node, out = []) {
  if (node.tagName) out.push(node.tagName);
  for (const child of node.children || []) collectTags(child, out);
  return out;
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
  // The grab is curated (one logical download: the mp4 — the zip link
  // is not media and loses to it), so exactly one URL was selected.
  assert.match(text, /Found 1 media URL\(s\) but nothing was sent/, text);
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
  assert.match(text, /Sent 1 media URL\(s\) to DM\./);
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

test("a playing MSE page is reported as a player, not as 'no media'", async () => {
  // The reported bug, end to end: the page has a <video>, it plays,
  // and the popup still said "No media found on this page". The
  // message now names what was actually found.
  const el = new FakeElement("video");
  el.setAttribute("src", "blob:https://site.example.test/1f2e");
  const { clock, els } = bootAll({
    popup: true,
    elements: {
      // Drop the shared page's mp4 anchor: the point is a page whose
      // only media is the blob-backed player.
      "a[href]": [],
      "video, audio, source, picture source, track": [el],
    },
  });
  await settle(clock);

  els.grab.dispatch("click");
  await settle(clock, 40);

  const text = textOf(els.status);
  assert.match(text, /blob:/, text);
  assert.match(text, /press play/, text);
  assert.ok(!/No media found on this page/.test(text), text);
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

test("a missing background is not blamed on the DM host", async () => {
  // The single most likely real-world cause of "the app is running but
  // the popup won't connect": a Chromium MV3 extension whose manifest
  // declares only `background.scripts` (Chrome ignores it) has no
  // service worker at all. The popup used to render that as
  // "DM host not running", pointing the user at the wrong program.
  const clock = createClock(START);
  const ctl = createChromeMock({
    noBackground: true,
    tabs: [{ id: 1, hasContentScript: true }],
  });
  const doc = new FakeDocument();
  const els = {};
  for (const id of [
    "status",
    "err-detail",
    "host-dot",
    "host-label",
    "host-state",
    "open",
    "grab",
    "test",
    "opt-click",
    "opt-takeover",
  ]) {
    els[id] = doc.register(id, new FakeElement("div"));
  }
  loadExtension(["popup.js"], {
    chrome: ctl,
    clock,
    document: doc,
    location: { href: PAGE, protocol: "https:" },
    navigator: { userAgent: "UA" },
  });
  // Let the cold-start retry budget (80+200+400 ms) play out.
  await clock.advance(3000);

  const text = textOf(els.status);
  assert.match(text, /Extension background isn't running/);
  assert.ok(!/DM host not running/.test(text), text);
  assert.match(textOf(els["err-detail"]), /background\.scripts/, "and says why");
});

test("a host that refuses the connection points at the probe", async () => {
  // `close code 1006` is the only thing the browser ever reports, and it
  // cannot distinguish "not running" from "running but never answered
  // the upgrade" (a pre-0.4.2 DM build). So the popup must send the user
  // to something that can.
  const { clock, ctl, els } = bootAll({ popup: true });
  await settle(clock);
  ctl.ws.latest().simulateClose(1006);
  await pollOnce(clock);

  const text = textOf(els.status);
  assert.match(text, /DM host not running/);
  assert.match(textOf(els["err-detail"]), /check-host\.mjs/, "points at the probe");
  assert.match(textOf(els["err-detail"]), /1006/);
});

// UI_REVIEW #28: the host state belongs in the title row as a dot +
// label, with one line of detail beneath it — not in a block at the
// bottom. Both states are pinned separately: the background reconnects
// eagerly, so driving one boot through "down → up" is not a stable
// sequence to assert against.
test("popup: host hint prose survives every diagnostic branch", async () => {
  // R2-01: four hintForError branches mix plain prose strings with
  // {tag, text} nodes. renderMarkup used to call createElement(undefined)
  // on the strings, silently dropping the entire repair instruction and
  // leaving empty <undefined> elements. Firefox users got a run-together
  // list of paths instead of a diagnosis.
  const cases = [
    [
      "No such native application com.app.dm.native",
      /Firefox can't find a valid host manifest/,
    ],
    ["Host executable not found", /The host manifest points to a binary/],
    [
      "Access denied for native messaging host",
      /The extension isn't allowed to use this host/,
    ],
    ["Host manifest is malformed", /The host manifest is malformed/],
  ];
  for (const [message, expected] of cases) {
    const els = await popupWithHostError(message);
    const detail = textOf(els["err-detail"]);
    assert.match(detail, /Browser said: /, message);
    assert.match(detail, expected, message);
    assert.equal(
      collectTags(els["err-detail"]).filter((t) => t === "UNDEFINED").length,
      0,
      `${message} must not create <undefined> elements`,
    );
  }
});

test("fake DOM rejects invalid tags so popup rendering bugs cannot hide", () => {
  // R2-05: the harness used to turn createElement(undefined) into a
  // harmless <UNDEFINED> element, so a rendering bug could pass the
  // whole suite. It must fail loudly instead.
  const doc = new FakeDocument();
  assert.throws(() => doc.createElement(undefined), /invalid tag/);
  assert.throws(() => doc.createElement(""), /invalid tag/);
});

test("popup: every remaining host hint branch still renders guidance", async () => {
  // R2-05: the other six exits of hintForError/getStatus were never
  // asserted either. Keep all ten branches pinned so a future edit
  // cannot quietly break one transport and leave the others green.
  const cases = [
    [
      "Could not establish connection. Receiving end does not exist.",
      /service worker never started/,
    ],
    ["WebSocket ctor failed: blocked", /browser refused to open the WebSocket/],
    [
      "chrome.runtime.connectNative is not a function",
      /missing the nativeMessaging/,
    ],
    ["close code 1006", /check-host\.mjs/],
    ["an error the popup does not recognise", /Browser said:/],
    ["", /The host process never reported a status/],
  ];
  for (const [message, expected] of cases) {
    const els = await popupWithHostError(message);
    assert.match(
      textOf(els["err-detail"]),
      expected,
      message || "(empty lastError)",
    );
  }
});

test("popup: title-row dot reads Disconnected when the host is down", async () => {
  const { clock, els } = bootAll({ popup: true });
  await settle(clock);
  await pollOnce(clock);

  assert.equal(els["host-dot"].className, "host-dot err");
  assert.equal(textOf(els["host-label"]), "Disconnected");
  assert.match(els["host-state"].title, /DM host not running/);
  assert.match(textOf(els.status), /DM host not running/);
});

test("popup: title-row dot reads Connected once the host answers", async () => {
  const { clock, ctl, els } = bootAll({ popup: true });
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);
  await pollOnce(clock);

  assert.equal(els["host-dot"].className, "host-dot ok");
  assert.equal(textOf(els["host-label"]), "Connected");
  assert.match(els["host-state"].title, /DM host connected/);
  assert.match(textOf(els.status), /DM host connected/);
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
  for (const id of ["status", "err-detail", "host-dot", "host-label", "host-state", "open", "grab", "test", "opt-click", "opt-takeover"]) {
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
