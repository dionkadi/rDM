// Regression tests for content.js — the click interceptor.
//
// The headline requirement: when DM isn't reachable, a click must be
// left completely alone so the browser downloads it normally.

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
} from "./harness.mjs";

const START = 1_700_000_000_000;
const PAGE = "https://site.example.test/article";
const ZIP = "https://files.example.test/archive.zip";
const MP4 = "https://files.example.test/movie.mp4";
const PLAIN = "https://site.example.test/another-article";

function bootContent(options = {}) {
  const clock = createClock(START);
  const ctl = createChromeMock({ tabs: [{ id: 1, hasContentScript: true }], ...options.chrome });
  const doc = new FakeDocument({ querySelectorAllResults: options.anchors || {} });
  loadExtension(["content.js"], {
    chrome: ctl,
    clock,
    document: doc,
    location: options.location || { href: PAGE, protocol: "https:" },
    navigator: { userAgent: "Mozilla/5.0 (Test) DMTest/1.0" },
  });
  return { clock, ctl, doc };
}

/** Push a background state update into the content script. */
function pushState(ctl, connected, settings = { interceptOnClick: true }) {
  ctl.tabsSendMessage(1, { type: "state", state: { connected, settings } });
}

function click(doc, href, over = {}) {
  const { anchor, event } = makeClick(href, over);
  doc.documentElement.dispatch("click", event);
  return { anchor, event };
}

const clicksSent = (ctl) =>
  ctl.calls.runtimeSendMessage.filter((m) => m.type === "download-click");

test("the listener is not installed on non-http(s) pages", () => {
  // The about:newtab guard. Installing a capture-phase click listener
  // there breaks the browser's own new-tab page.
  const { doc } = bootContent({
    location: { href: "about:newtab", protocol: "about:" },
  });
  assert.equal(doc.documentElement._listeners.click, undefined);
});

test("P0-2: with DM unreachable a media click is not intercepted at all", () => {
  const { doc, ctl } = bootContent();
  const { event } = click(doc, ZIP);

  assert.equal(event.prevented, undefined, "must not call preventDefault");
  assert.equal(event.propagationStopped, undefined);
  assert.equal(event.immediateStopped, undefined);
  assert.equal(clicksSent(ctl).length, 0, "and must not send anything");
});

test("before the background has answered, the default is to stay out of the way", () => {
  // `hostAlive` starts false on purpose: failing open lets the
  // browser download; failing closed swallows the click.
  const { doc } = bootContent();
  const { event } = click(doc, MP4);
  assert.equal(event.prevented, undefined);
});

test("with DM reachable the click is suppressed and forwarded", () => {
  const { doc, ctl } = bootContent();
  pushState(ctl, true);

  const { event } = click(doc, ZIP, { text: "archive" });

  assert.equal(event.prevented, true);
  const sent = clicksSent(ctl);
  assert.equal(sent.length, 1);
  assert.equal(sent[0].url, ZIP);
  assert.equal(sent[0].referer, PAGE);
  assert.equal(sent[0].userAgent, "Mozilla/5.0 (Test) DMTest/1.0");
});

test("P2-7: interception no longer stops propagation to the page", () => {
  // Sites whose download flow is JS-driven used to break, because the
  // interceptor called stopImmediatePropagation() on the page's own
  // click handlers.
  const { doc, ctl } = bootContent();
  pushState(ctl, true);

  let pageHandlerRan = false;
  doc.documentElement.addEventListener("click", () => {
    pageHandlerRan = true;
  });

  const { event } = click(doc, ZIP);

  assert.equal(event.prevented, true, "browser default still suppressed");
  assert.equal(event.propagationStopped, undefined, "but propagation is untouched");
  assert.equal(event.immediateStopped, undefined);
  assert.equal(pageHandlerRan, true, "the page's handler still runs");
});

test("DM going away mid-session re-opens the link (no reload needed)", () => {
  const { doc, ctl } = bootContent();
  pushState(ctl, true);
  assert.equal(click(doc, ZIP).event.prevented, true);

  pushState(ctl, false);
  assert.equal(click(doc, ZIP).event.prevented, undefined);
});

test("P1-5: turning off click handling leaves links to the browser", () => {
  const { doc, ctl } = bootContent();
  pushState(ctl, true, { interceptOnClick: false });
  assert.equal(click(doc, ZIP).event.prevented, undefined);
});

test("modifier clicks, middle clicks and synthetic clicks are never intercepted", () => {
  const { doc, ctl } = bootContent();
  pushState(ctl, true);

  for (const over of [
    { event: { ctrlKey: true } },
    { event: { metaKey: true } },
    { event: { shiftKey: true } },
    { event: { altKey: true } },
    { event: { button: 1 } },
    { event: { button: 2 } },
    { event: { isTrusted: false } },
  ]) {
    const { event } = click(doc, ZIP, over);
    assert.equal(event.prevented, undefined, JSON.stringify(over));
  }
});

test("only download-looking links are intercepted", () => {
  const { doc, ctl } = bootContent();
  pushState(ctl, true);

  assert.equal(click(doc, PLAIN).event.prevented, undefined, "plain article link");
  assert.equal(
    click(doc, "https://files.example.test/readme.txt").event.prevented,
    undefined,
    "unrecognised extension",
  );
  assert.equal(
    click(doc, "https://files.example.test/sheet.xlsx").event.prevented,
    undefined,
    "unrecognised extension",
  );
  assert.equal(click(doc, MP4).event.prevented, true, "media extension");
  assert.equal(
    click(doc, "https://files.example.test/thing", { downloadAttr: "thing" })
      .event.prevented,
    true,
    "explicit download attribute",
  );
});

test("a click the page already handled is left alone", () => {
  const { doc, ctl } = bootContent();
  pushState(ctl, true);
  const { event } = click(doc, ZIP, { event: { defaultPrevented: true } });
  assert.equal(event.prevented, undefined);
});

test("non-http(s) hrefs are ignored", () => {
  const { doc, ctl } = bootContent();
  pushState(ctl, true);
  for (const href of [
    "javascript:alert(1)",
    "data:application/zip;base64,AAAA",
    "ftp://files.example.test/a.zip",
  ]) {
    assert.equal(click(doc, href).event.prevented, undefined, href);
  }
});

test("P0-2: an orphaned content script stops intercepting after an extension reload", () => {
  // After a reload the old content script is still running in open
  // tabs but its runtime is gone: sendMessage would throw *after*
  // preventDefault had already eaten the click.
  const clock = createClock(START);
  const ctl = createChromeMock({
    runtimeId: null,
    tabs: [{ id: 1, hasContentScript: true }],
  });
  const doc = new FakeDocument();
  loadExtension(["content.js"], {
    chrome: ctl,
    clock,
    document: doc,
    location: { href: PAGE, protocol: "https:" },
    navigator: { userAgent: "UA" },
  });
  pushState(ctl, true);

  const { event } = click(doc, ZIP);
  assert.equal(event.prevented, undefined);
});

test("the collect message answers with media URLs and the page hints", () => {
  const zip = new FakeElement("a");
  zip.setAttribute("href", ZIP);
  const mp4 = new FakeElement("a");
  mp4.setAttribute("href", MP4);
  const articleLink = new FakeElement("a");
  articleLink.setAttribute("href", PLAIN);
  const video = new FakeElement("video");
  video.setAttribute("src", "https://cdn.example.test/stream.webm");

  const { ctl } = bootContent({
    anchors: {
      "a[href]": [zip, mp4, articleLink],
      "video, audio, source, picture source, track": [video],
    },
  });

  let res = null;
  ctl.tabsSendMessage(1, { type: "collect" }, (r) => {
    res = r;
  });

  assert.ok(res);
  assert.deepEqual(
    plain(res.urls).sort(),
    [MP4, ZIP, "https://cdn.example.test/stream.webm"].sort(),
  );
  assert.equal(res.referer, PAGE);
  assert.equal(res.userAgent, "Mozilla/5.0 (Test) DMTest/1.0");
  assert.equal(res.pageUrl, PAGE);
});

test("the removed auto-scrape path stays removed", () => {
  // A stale copy of our own content script (or a third party) sending
  // the legacy `media` message must not be forwarded.
  const { ctl } = bootContent();
  const before = ctl.calls.runtimeSendMessage.length;
  ctl.sendMessage({ type: "media", urls: ["https://files.example.test/x.zip"] }, () => {});
  assert.equal(
    ctl.calls.runtimeSendMessage.length,
    before,
    "content.js must not emit anything in response",
  );
});
