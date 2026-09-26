// Regression tests for content.js — the click interceptor.
//
// The headline requirement: when DM isn't reachable, a click must be
// left completely alone so the browser downloads it normally.

import test from "node:test";
import assert from "node:assert/strict";
import {
  createClock,
  createChromeMock,
  createPerformanceFake,
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
    performance: options.performance,
    fetch: options.fetch,
    window: options.window,
  });
  return { clock, ctl, doc };
}

/** Ask the content script for its media list. May answer
 *  asynchronously (play-info enhancement), so this awaits. */
async function grab(ctl) {
  let res = null;
  ctl.tabsSendMessage(1, { type: "collect" }, (r) => {
    res = r;
  });
  for (let i = 0; i < 100 && res === null; i++) {
    await Promise.resolve();
  }
  return res;
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

test("the listener is not installed on non-http(s) pages", async () => {
  // The about:newtab guard. Installing a capture-phase click listener
  // there breaks the browser's own new-tab page.
  const { doc } = bootContent({
    location: { href: "about:newtab", protocol: "about:" },
  });
  assert.equal(doc.documentElement._listeners.click, undefined);
});

test("P0-2: with DM unreachable a media click is not intercepted at all", async () => {
  const { doc, ctl } = bootContent();
  const { event } = click(doc, ZIP);

  assert.equal(event.prevented, undefined, "must not call preventDefault");
  assert.equal(event.propagationStopped, undefined);
  assert.equal(event.immediateStopped, undefined);
  assert.equal(clicksSent(ctl).length, 0, "and must not send anything");
});

test("before the background has answered, the default is to stay out of the way", async () => {
  // `hostAlive` starts false on purpose: failing open lets the
  // browser download; failing closed swallows the click.
  const { doc } = bootContent();
  const { event } = click(doc, MP4);
  assert.equal(event.prevented, undefined);
});

test("with DM reachable the click is suppressed and forwarded", async () => {
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

test("P2-7: interception no longer stops propagation to the page", async () => {
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

test("DM going away mid-session re-opens the link (no reload needed)", async () => {
  const { doc, ctl } = bootContent();
  pushState(ctl, true);
  assert.equal(click(doc, ZIP).event.prevented, true);

  pushState(ctl, false);
  assert.equal(click(doc, ZIP).event.prevented, undefined);
});

test("P1-5: turning off click handling leaves links to the browser", async () => {
  const { doc, ctl } = bootContent();
  pushState(ctl, true, { interceptOnClick: false });
  assert.equal(click(doc, ZIP).event.prevented, undefined);
});

test("modifier clicks, middle clicks and synthetic clicks are never intercepted", async () => {
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

test("only download-looking links are intercepted", async () => {
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

test("a click the page already handled is left alone", async () => {
  const { doc, ctl } = bootContent();
  pushState(ctl, true);
  const { event } = click(doc, ZIP, { event: { defaultPrevented: true } });
  assert.equal(event.prevented, undefined);
});

test("non-http(s) hrefs are ignored", async () => {
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

test("P0-2: an orphaned content script stops intercepting after an extension reload", async () => {
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

test("the collect message answers with media URLs and the page hints", async () => {
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
  // One grab, one download: the grab offers the *media* file it is
  // most confident in. The zip is a real download but not media — it
  // stays for the click path (a click on it is forwarded verbatim),
  // and the plain article link is not offered at all.
  assert.deepEqual(plain(res.urls), ["https://cdn.example.test/stream.webm"]);
  assert.equal(res.referer, PAGE);
  assert.equal(res.userAgent, "Mozilla/5.0 (Test) DMTest/1.0");
  assert.equal(res.pageUrl, PAGE);
});

test("the removed auto-scrape path stays removed", async () => {
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

// ─── "Grab page media" ──────────────────────────────────────────
//
// Reported: grabbing on a bilibili watch page returned `iframe.html`
// and `player.html` instead of video. Two causes: the iframe branch
// added every iframe URL unfiltered (and `.html` is not media), and an
// MSE player never puts the real stream in the DOM at all.

/** Build a `<video>` the way an MSE player presents one. */
function mseVideo() {
  const video = new FakeElement("video");
  video.setAttribute("src", "blob:https://www.bilibili.com/8f3a-2b1c");
  video.currentSrc = "blob:https://www.bilibili.com/8f3a-2b1c";
  return video;
}

function iframe(src) {
  const el = new FakeElement("iframe");
  el.setAttribute("src", src);
  return el;
}

test("grab: a bilibili-style page yields the real streams, not the player's HTML", async () => {
  const videoUrl =
    "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/12/34/56/123456-1-30280.m4s?deadline=1&uipk=5";
  const audioUrl =
    "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/12/34/56/123456-1-30232.m4s?deadline=1&uipk=5";
  const { ctl } = bootContent({
    anchors: {
      "video, audio, source, picture source, track": [mseVideo()],
      "iframe[src]": [
        iframe("https://player.bilibili.com/player.html?aid=123"),
        iframe("https://www.bilibili.com/blackboard/iframe.html"),
      ],
    },
    // What the player fetched into the blob.
    performance: createPerformanceFake([
      "https://s1.hdslb.com/bfs/static/main.css",
      "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/12/34/56/123456-1-30280.m4s?range=0-8388607",
      videoUrl,
      audioUrl,
    ]),
  });

  const res = await grab(ctl);
  assert.ok(res, "collect must answer");
  assert.ok(
    res.urls.some((u) => u.includes("30280.m4s")),
    `expected the video stream, got ${JSON.stringify(res.urls)}`,
  );
  assert.ok(
    res.urls.some((u) => u.includes("30232.m4s")),
    "and the audio stream (DASH keeps them separate)",
  );
  for (const u of res.urls) {
    assert.ok(!/\.html(\?|$)/.test(u), `HTML must never be offered: ${u}`);
    assert.ok(!/^blob:/.test(u), `blob: is not downloadable: ${u}`);
  }
});

test("grab: one grab, one download — the pair, with range-request noise stripped", async () => {
  // The IDM contract. A page like bilibili fetches the same two streams
  // over and over as byte-range requests; every range is a distinct
  // URL, but the user wants ONE download. The grab must collapse all
  // of it into the two stream bases (query stripped — the base URL
  // serves the whole stream).
  const entries = [];
  for (let i = 0; i < 20; i++) {
    const tag = i % 2 === 0 ? "30280" : "30232";
    entries.push(
      `https://upos.example.test/upgcxcode/12/34/56/123456-1-${tag}.m4s?range=${i * 8388608}-${
        (i + 1) * 8388608 - 1
      }`,
    );
  }
  const { ctl } = bootContent({
    performance: createPerformanceFake(entries),
  });

  const res = await grab(ctl);
  assert.equal(res.urls.length, 2, `got ${JSON.stringify(res.urls)}`);
  assert.deepEqual(
    plain(res.urls).sort(),
    [
      "https://upos.example.test/upgcxcode/12/34/56/123456-1-30232.m4s",
      "https://upos.example.test/upgcxcode/12/34/56/123456-1-30280.m4s",
    ],
  );
  for (const u of res.urls) {
    assert.ok(!u.includes("?"), `the base must be sent, not a range request: ${u}`);
  }
  assert.match(res.note, /video \+ audio pair/);
});

test("grab: only the range parameter is stripped — signing params survive", async () => {
  // Regression: stripping the WHOLE query destroyed the URL's
  // credentials, and bilibili's PCDN edge answered 400 Bad Request.
  // `range` is the player's per-fetch parameter; `upsign`/`deadline`
  // &co. are what makes the URL downloadable as a whole and must stay.
  const entries = [
    "https://upos.example.test/upgcxcode/45/52/42138275245/42138275245-1-30280.m4s?upsign=abc&deadline=1790000000&range=0-8388607",
    "https://upos.example.test/upgcxcode/45/52/42138275245/42138275245-1-30280.m4s?upsign=abc&deadline=1790000000&range=8388608-16777215",
    "https://upos.example.test/upgcxcode/45/52/42138275245/42138275245-1-30216.m4s?upsign=def&deadline=1790000000&range=0-1048575",
  ];
  const { ctl } = bootContent({ performance: createPerformanceFake(entries) });
  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls).sort(), [
    "https://upos.example.test/upgcxcode/45/52/42138275245/42138275245-1-30216.m4s?upsign=def&deadline=1790000000",
    "https://upos.example.test/upgcxcode/45/52/42138275245/42138275245-1-30280.m4s?upsign=abc&deadline=1790000000",
  ]);
  for (const u of res.urls) {
    assert.ok(!/[?&]range=/.test(u), `range must be stripped: ${u}`);
    assert.ok(u.includes("upsign="), `signing params must survive: ${u}`);
  }
});

test("grab: P2P edge copies are demoted when a normal CDN base exists", async () => {
  // The player switched to an mcdn (PCDN) node mid-play, so the newest
  // request for the video stream is a P2P copy. mcdn nodes routinely
  // refuse whole-file requests (the "1 B chunk / 400 Bad Request"
  // report), so a normal CDN base of the same stream wins even when it
  // is older.
  const entries = [
    // oldest → newest (the scan reverses this list)
    "https://upos.example.test/upgcxcode/45/52/v/42138275245-1-30280.m4s?upsign=a&deadline=1",
    "https://upos.example.test/upgcxcode/45/52/a/42138275245-1-30216.m4s?upsign=b&deadline=1",
    "https://xy112x19x175x214xy.mcdn.bilivideo.cn:8082/v1/resource/upgcxcode/45/52/v/42138275245-1-30280.m4s?upsign=c&deadline=1",
  ];
  const { ctl } = bootContent({ performance: createPerformanceFake(entries) });
  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls).sort(), [
    "https://upos.example.test/upgcxcode/45/52/a/42138275245-1-30216.m4s?upsign=b&deadline=1",
    "https://upos.example.test/upgcxcode/45/52/v/42138275245-1-30280.m4s?upsign=a&deadline=1",
  ]);
  assert.ok(res.urls.every((u) => !u.includes("mcdn")), JSON.stringify(res.urls));
});

test("grab: an mcdn-only page still offers the pair, with a warning", async () => {
  // Honest fallback: when the P2P edge is all we saw, offer it but say
  // so — the user can let the video play longer and grab again.
  const entries = [
    "https://xy1x2x3x4xy.mcdn.bilivideo.cn:8082/v1/resource/v/1-1-30080.m4s?upsign=a",
    "https://xy1x2x3x4xy.mcdn.bilivideo.cn:8082/v1/resource/a/1-1-30216.m4s?upsign=b",
  ];
  const { ctl } = bootContent({ performance: createPerformanceFake(entries) });
  const res = await grab(ctl);
  assert.equal(res.urls.length, 2);
  assert.match(res.note, /P2P edge/);
});

test("grab: fragments are declined when a complete file exists", async () => {
  const video = new FakeElement("video");
  video.setAttribute("src", "https://cdn.example.test/clip.mp4");
  const { ctl } = bootContent({
    anchors: { "video, audio, source, picture source, track": [video] },
    performance: createPerformanceFake([
      "https://cdn.example.test/dash/seg.m4s?range=0-1000",
      "https://cdn.example.test/dash/seg.m4s?range=1000-2000",
    ]),
  });
  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), ["https://cdn.example.test/clip.mp4"]);
  assert.match(res.note, /2 other media request\(s\) not offered/);
});

test("grab: a lone segment with nothing else is a last-resort offer", async () => {
  const { ctl } = bootContent({
    performance: createPerformanceFake([
      "https://cdn.example.test/dash/seg.m4s?range=0-1000",
    ]),
  });
  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), ["https://cdn.example.test/dash/seg.m4s"]);
  assert.match(res.note, /single DASH segment/);
});

test("grab: an HLS-only page offers nothing (segments are not files)", async () => {
  // `.ts` segments are deliberately outside RESOURCE_MEDIA_RE — they
  // are pieces of one stream and DM cannot expand HLS yet, so an
  // HLS-only page honestly reports "no media" instead of dumping
  // dozens of fragments into the queue.
  const { ctl } = bootContent({
    performance: createPerformanceFake([
      "https://cdn.example.test/hls/0.ts",
      "https://cdn.example.test/hls/1.ts",
    ]),
  });
  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), []);
  assert.match(res.note, /No media found/);
});

test("grab: two same-directory files are offered as a pair, two random files are not", async () => {
  const mkEl = (tag, src) => {
    const el = new FakeElement(tag);
    el.setAttribute("src", src);
    return el;
  };
  const { ctl } = bootContent({
    anchors: {
      "video, audio, source, picture source, track": [
        mkEl("video", "https://cdn.example.test/v/ep1/video.mp4"),
        mkEl("audio", "https://cdn.example.test/v/ep1/audio.m4a"),
      ],
    },
    performance: createPerformanceFake([
      "https://elsewhere.example.test/trailer.mp4",
    ]),
  });
  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls).sort(), [
    "https://cdn.example.test/v/ep1/audio.m4a",
    "https://cdn.example.test/v/ep1/video.mp4",
  ]);
});

test("grab: a manifest is offered only when nothing better exists", async () => {
  const { ctl } = bootContent({
    performance: createPerformanceFake(["https://cdn.example.test/stream.m3u8"]),
  });
  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), ["https://cdn.example.test/stream.m3u8"]);
  assert.match(res.note, /manifest/);
});

test("grab: the same stream requested several times is offered once", async () => {
  // Range requests repeat the base URL; the query differs, so
  // exact-string dedupe is not enough on its own for every CDN — but
  // the identical URL must not be listed twice.
  const url = "https://cdn.example.test/movie.mp4?token=abc";
  const { ctl } = bootContent({
    performance: createPerformanceFake([url, url, url]),
  });
  const res = await grab(ctl);
  assert.equal(res.urls.filter((u) => u === url).length, 1);
});

test("grab: media elements are preferred over the network copy", async () => {
  const progressive = "https://cdn.example.test/clip.mp4";
  const video = new FakeElement("video");
  video.setAttribute("src", progressive);
  video.currentSrc = progressive;
  const { ctl } = bootContent({
    anchors: { "video, audio, source, picture source, track": [video] },
    performance: createPerformanceFake([progressive]),
  });
  assert.deepEqual(plain((await grab(ctl)).urls), [progressive]);
});

test("grab: an iframe is only offered when its URL itself looks like media", async () => {
  const { ctl } = bootContent({
    anchors: {
      "iframe[src]": [
        iframe("https://player.example.test/embed/player.html"),
        iframe("https://cdn.example.test/embedded.mp4"),
      ],
    },
  });
  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), ["https://cdn.example.test/embedded.mp4"]);
});

test("grab: a DASH page offers the newest stream pair, not every fragment", async () => {
  // 12 network entries, deliberately shuffled by kind. The pair must
  // come from the .m4s bases; the mp4 files and the manifests lose to
  // the actively-playing representation.
  const entries = [];
  for (let i = 0; i < 4; i++) entries.push(`https://cdn.example.test/seg${i}.m4s`);
  for (let i = 0; i < 2; i++) entries.push(`https://cdn.example.test/stream${i}.m3u8`);
  for (let i = 0; i < 6; i++) entries.push(`https://cdn.example.test/file${i}.mp4`);
  const { ctl } = bootContent({ performance: createPerformanceFake(entries) });

  const res = await grab(ctl);
  assert.equal(res.urls.length, 2, `got ${JSON.stringify(res.urls)}`);
  assert.ok(res.urls.every((u) => /\.m4s$/.test(u)), JSON.stringify(res.urls));
  assert.match(res.note, /video \+ audio pair/);
});

test("grab: the note says when there is nothing usable", async () => {
  const { ctl } = bootContent({
    anchors: {
      "iframe[src]": [iframe("https://player.example.test/player.html")],
    },
  });
  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), []);
  assert.match(res.note, /cross-origin iframe|No media found/);
});

test("grab: the note reports what was selected", async () => {
  const { ctl } = bootContent({
    performance: createPerformanceFake(["https://cdn.example.test/a.mp4"]),
  });
  const note = (await grab(ctl)).note;
  assert.match(note, /1 URL\(s\) selected/);
  assert.match(note, /media file/);
});

test("grab: an og:video pointing at a player page is never offered", async () => {
  // `og:video` routinely carries a *player page* URL rather than a
  // stream. The DOCUMENT_RE guard in collectMedia must keep it out no
  // matter which heuristic picked it up.
  const meta = new FakeElement("meta");
  meta.setAttribute(
    "content",
    "https://player.example.test/embed/player.html?vid=42",
  );
  const video = new FakeElement("video");
  video.setAttribute("src", "https://cdn.example.test/clip.mp4");
  const { ctl } = bootContent({
    anchors: {
      "video, audio, source, picture source, track": [video],
      'meta[property="og:video"], meta[property="og:video:url"], meta[name="twitter:player:stream"]': [
        meta,
      ],
    },
  });
  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), ["https://cdn.example.test/clip.mp4"]);
});

test("grab: media inside a same-origin iframe is found from the top frame", async () => {
  // The collect message is answered by the top frame only (an iframe
  // answering for itself would leak its own URL as `pageUrl`), so
  // same-origin embeds must be scanned from here. `contentDocument`
  // is what the browser hands over for same-origin frames.
  const innerVideo = new FakeElement("video");
  innerVideo.setAttribute("src", "https://cdn.example.test/embedded.mp4");
  const inner = new FakeDocument({
    querySelectorAllResults: {
      "video, audio, source, picture source, track": [innerVideo],
    },
  });
  const frame = new FakeElement("iframe");
  frame.setAttribute("src", "https://site.example.test/embed/frame.html");
  frame.contentDocument = inner;

  const { ctl } = bootContent({
    anchors: {
      iframe: [frame],
      "iframe[src]": [frame],
    },
  });
  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), ["https://cdn.example.test/embedded.mp4"]);
  // The embed frame's own URL must never appear as a candidate…
  assert.ok(res.urls.every((u) => !/\.html(\?|$)/.test(u)));
  // …and the page context must stay the *top* page.
  assert.equal(res.pageUrl, PAGE);
});

// ── The page's play info (bilibili playurl) ─────────────────────────
//
// Real-world report: the grab offered the AUDIO half alone because the
// video half's requests were no longer in the (finite) resource-timing
// buffer. The page's playurl API names both streams explicitly, so the
// content script re-fetches it and completes the pair — with the page's
// own signed, normal-CDN URLs instead of P2P edge copies.

const PLAYINFO_URL =
  "https://api.bilibili.com/x/player/wbi/playurl?cid=1&fnval=16&fourk=1";
const PLAYINFO_URL_PREVIEW =
  "https://api.bilibili.com/x/player/wbi/playurl?cid=777&fnval=16";

/** Minimal `fetch` stand-in keyed by URL. */
function fetchFake(responses) {
  const calls = [];
  const fn = (url, opts) => {
    calls.push({ url, opts });
    const r = responses[url] !== undefined ? responses[url] : responses.default;
    if (r instanceof Error) return Promise.reject(r);
    return Promise.resolve({
      ok: !!r,
      json: async () => r,
    });
  };
  fn.calls = calls;
  return fn;
}

function playInfoBody() {
  return {
    code: 0,
    data: {
      dash: {
        video: [
          {
            id: 30112,
            baseUrl:
              "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-30112.m4s?upsign=best",
          },
          {
            id: 30080,
            baseUrl:
              "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-30080.m4s?upsign=fresh",
            backupUrl: [
              "https://xy1x2x3x4xy.mcdn.bilivideo.cn:8082/v1/resource/75/24/41875472475/41875472475-1-30080.m4s?upsign=p2p",
            ],
          },
        ],
        audio: [
          {
            id: 30280,
            baseUrl:
              "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-30280.m4s?upsign=hq",
          },
          {
            id: 30216,
            baseUrl:
              "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-30216.m4s?upsign=fresh-audio",
          },
        ],
      },
    },
  };
}

test("grab: the page's play info completes the pair when only audio was observed", async () => {
  // The user's exact case: the resource buffer only still held the
  // audio stream; the video half was gone. The playurl API names both.
  const fetch = fetchFake({ [PLAYINFO_URL]: playInfoBody() });
  const { ctl } = bootContent({
    performance: createPerformanceFake([
      PLAYINFO_URL,
      "https://b-xxx.edge.mountaintoys.cn:4483/upgcxcode/75/24/41875472475/41875472475-1-30216.m4s?os=mcdn&upsign=old",
    ]),
    fetch,
  });

  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), [
    // No observed video tail → the API's best quality…
    "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-30112.m4s?upsign=best",
    // …and the observed audio tail matched to its API entry.
    "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-30216.m4s?upsign=fresh-audio",
  ]);
  assert.match(res.note, /video \+ audio pair from the page's play info/);
  assert.equal(fetch.calls.length, 1, "the playurl request is re-fetched once");
  assert.equal(fetch.calls[0].url, PLAYINFO_URL);
  assert.equal(
    fetch.calls[0].opts.credentials,
    "include",
    "the API needs the page's cookies",
  );
});

test("grab: observed halves are matched to their play-info entries", async () => {
  const fetch = fetchFake({ [PLAYINFO_URL]: playInfoBody() });
  const { ctl } = bootContent({
    performance: createPerformanceFake([
      PLAYINFO_URL,
      // Both halves observed on P2P edge hosts; the API also offers
      // higher qualities — the OBSERVED representation must win.
      "https://xy1x2x3x4xy.mcdn.bilivideo.cn:8082/v1/resource/75/24/41875472475/41875472475-1-30080.m4s?upsign=p2p",
      "https://xy1x2x3x4xy.mcdn.bilivideo.cn:8082/v1/resource/75/24/41875472475/41875472475-1-30216.m4s?upsign=p2p",
    ]),
    fetch,
  });

  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), [
    "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-30080.m4s?upsign=fresh",
    "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-30216.m4s?upsign=fresh-audio",
  ]);
  // The video's baseUrl is a normal CDN; the P2P backup is not chosen.
  assert.ok(res.urls.every((u) => !u.includes("mcdn")), JSON.stringify(res.urls));
});

test("grab: a play-info fetch failure keeps the heuristic result", async () => {
  const fetch = fetchFake({ [PLAYINFO_URL]: new Error("network down") });
  const entries = [
    PLAYINFO_URL,
    "https://upos.example.test/upgcxcode/12/34/56/123456-1-30280.m4s?range=0-8388607",
    "https://upos.example.test/upgcxcode/12/34/56/123456-1-30232.m4s?range=0-8388607",
  ];
  const { ctl } = bootContent({ performance: createPerformanceFake(entries), fetch });

  const res = await grab(ctl);
  assert.equal(res.urls.length, 2, `got ${JSON.stringify(res.urls)}`);
  assert.match(res.note, /video \+ audio pair — DM downloads/);
});

test("grab: legacy durl play pages offer a direct file", async () => {
  const fetch = fetchFake({
    [PLAYINFO_URL]: {
      code: 0,
      data: {
        durl: [
          {
            url: "https://upos-sz-mirrorcos.bilivideo.com/v/41875472475-1-100100.mp4?upsign=direct",
          },
        ],
      },
    },
  });
  const { ctl } = bootContent({
    performance: createPerformanceFake([PLAYINFO_URL]),
    fetch,
  });

  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), [
    "https://upos-sz-mirrorcos.bilivideo.com/v/41875472475-1-100100.mp4?upsign=direct",
  ]);
  assert.match(res.note, /direct media file from the page's play info/);
});

// ── Multi-episode pages (playlists/seasons) ─────────────────────────
//
// Real-world report: on a page whose tab had played episode A (33 min,
// cid 41875472475) and later episode B (6 min, cid 42032629796), the
// grab offered episode B's video alone, and even a cross-episode pair
// was possible — the play-info "newest response" was B's preload while
// the observed segments belonged to A. The cid grouping pins both
// sides: the heuristic pairs within the newest-observed cid only, and
// the enhancer fetches play-info responses until one contains that cid.

const PLAYINFO_B =
  "https://api.bilibili.com/x/player/wbi/playurl?cid=999&fnval=16";
const PLAYINFO_A =
  "https://api.bilibili.com/x/player/wbi/playurl?cid=1&fnval=16";

function episodeBody(cid, videoQ, audioQ) {
  return {
    code: 0,
    data: {
      dash: {
        video: [
          {
            id: videoQ,
            baseUrl: `https://upos.example.com/upgcxcode/${cid}/${cid}-1-${videoQ}.m4s?upsign=v-${cid}`,
          },
        ],
        audio: [
          {
            id: audioQ,
            baseUrl: `https://upos.example.com/upgcxcode/${cid}/${cid}-1-${audioQ}.m4s?upsign=a-${cid}`,
          },
        ],
      },
    },
  };
}

test("grab: the heuristic pair never mixes episodes", async () => {
  // Chronological order (the scan reverses it): episode A played for a
  // while (many segment requests), then ONE segment of episode B was
  // fetched (a preload or hover preview). The pair must stay within the
  // dominant cid (A — most requested), never mix A's video with B's
  // audio.
  const entries = [
    "https://upos.example.test/upgcxcode/41875472475/41875472475-1-30080.m4s?range=0-1000",
    "https://upos.example.test/upgcxcode/41875472475/41875472475-1-30080.m4s?range=1000-2000",
    "https://upos.example.test/upgcxcode/41875472475/41875472475-1-30080.m4s?range=2000-3000",
    "https://upos.example.test/upgcxcode/41875472475/41875472475-1-30216.m4s?range=0-1000",
    "https://upos.example.test/upgcxcode/42032629796/42032629796-1-30032.m4s?range=0-1000",
  ];
  const { ctl } = bootContent({ performance: createPerformanceFake(entries) });
  const res = await grab(ctl);
  // Pair order follows discovery (the audio base was requested more
  // recently than the video base here); the app's merge maps streams by
  // type, so the order is cosmetic.
  assert.deepEqual(plain(res.urls), [
    "https://upos.example.test/upgcxcode/41875472475/41875472475-1-30216.m4s",
    "https://upos.example.test/upgcxcode/41875472475/41875472475-1-30080.m4s",
  ]);
});

test("grab: a hover preview does not out-shout the watched video", async () => {
  // Real-world report: the grab merged a 14-second promo clip
  // (cid 40360152772) whose segments were fetched most recently,
  // instead of the 33-minute video (cid 41875472475) the user watched.
  // Request counts fix the selection: the watched video out-requests a
  // preview by two orders of magnitude. The preview's playurl (newest
  // in the buffer) must be skipped in favour of the watched episode's.
  const entries = [
    PLAYINFO_URL_PREVIEW,
    PLAYINFO_URL,
    // Watched video: many range requests, two streams.
    ...Array.from({ length: 12 }, (_, i) =>
      `https://upos.example.test/upgcxcode/41875472475/41875472475-1-30080.m4s?range=${i * 1000}-${(i + 1) * 1000}`,
    ),
    ...Array.from({ length: 4 }, (_, i) =>
      `https://upos.example.test/upgcxcode/41875472475/41875472475-1-30216.m4s?range=${i * 1000}-${(i + 1) * 1000}`,
    ),
    // Hover preview: 2 segment requests, fetched most recently.
    "https://upos.example.test/upgcxcode/40360152772/40360152772-1-100022.m4s?range=0-1000",
    "https://upos.example.test/upgcxcode/40360152772/40360152772-1-30216.m4s?range=0-1000",
  ];
  const fetch = fetchFake({
    [PLAYINFO_URL]: episodeBody("41875472475", 30080, 30216),
    [PLAYINFO_URL_PREVIEW]: episodeBody("40360152772", 100022, 30216),
  });
  const { ctl } = bootContent({ performance: createPerformanceFake(entries), fetch });

  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), [
    "https://upos.example.com/upgcxcode/41875472475/41875472475-1-30080.m4s?upsign=v-41875472475",
    "https://upos.example.com/upgcxcode/41875472475/41875472475-1-30216.m4s?upsign=a-41875472475",
  ]);
  assert.match(res.note, /stream 41875472475/);
});

test("grab: the enhancer fetches play-info responses until one matches the playing cid", async () => {
  const fetch = fetchFake({
    // Newest response: episode B (a preloaded next part)…
    [PLAYINFO_B]: episodeBody("42032629796", 30032, 30216),
    // …older response: episode A, the one actually playing.
    [PLAYINFO_A]: episodeBody("41875472475", 30080, 30216),
  });
  const { ctl } = bootContent({
    performance: createPerformanceFake([
      PLAYINFO_B,
      PLAYINFO_A,
      // The player is streaming episode A right now (newest segments).
      "https://b-x.edge.mountaintoys.cn:4483/upgcxcode/41875472475/41875472475-1-30080.m4s?os=mcdn&upsign=old",
      "https://b-x.edge.mountaintoys.cn:4483/upgcxcode/41875472475/41875472475-1-30216.m4s?os=mcdn&upsign=old",
    ]),
    fetch,
  });

  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), [
    "https://upos.example.com/upgcxcode/41875472475/41875472475-1-30080.m4s?upsign=v-41875472475",
    "https://upos.example.com/upgcxcode/41875472475/41875472475-1-30216.m4s?upsign=a-41875472475",
  ]);
  // B's response was fetched and rejected before A's was used.
  const fetched = fetch.calls.map((c) => c.url);
  assert.ok(fetched.indexOf(PLAYINFO_B) < fetched.indexOf(PLAYINFO_A));
  assert.match(res.note, /stream 41875472475/);
});

test("grab: a playurl without an audio track is skipped", async () => {
  const fetch = fetchFake({
    [PLAYINFO_B]: {
      // Video-only dash (e.g. a preview response) — unusable alone.
      code: 0,
      data: {
        dash: {
          video: [
            {
              id: 30032,
              baseUrl:
                "https://upos.example.com/upgcxcode/42032629796/42032629796-1-30032.m4s?upsign=b",
            },
          ],
          audio: [],
        },
      },
    },
    [PLAYINFO_A]: episodeBody("41875472475", 30080, 30216),
  });
  const { ctl } = bootContent({
    performance: createPerformanceFake([
      PLAYINFO_B,
      PLAYINFO_A,
      "https://upos.example.test/upgcxcode/41875472475/41875472475-1-30216.m4s?range=0-1000",
    ]),
    fetch,
  });

  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), [
    "https://upos.example.com/upgcxcode/41875472475/41875472475-1-30080.m4s?upsign=v-41875472475",
    "https://upos.example.com/upgcxcode/41875472475/41875472475-1-30216.m4s?upsign=a-41875472475",
  ]);
});

// ── The embedded play info (window.__playinfo__) ────────────────────
//
// Second real-world report: the grab again offered a lone VIDEO half
// (the correct episode this time). The playurl request had fallen out
// of the resource buffer (the audio half buffers completely early in a
// video, so only video ranges keep flowing), and the buffer-only
// enhancer silently gave up. bilibili embeds the same payload in
// `window.__playinfo__` on every watch page — no network, never
// dropped — so the enhancer reads that first, and failures are now
// reported in the grab note instead of being silent.

function selfWindow(playinfo) {
  const w = { __playinfo__: playinfo };
  w.top = w;
  return w;
}

test("grab: the embedded play info completes the pair without any fetch", async () => {
  const fetch = fetchFake({});
  const { ctl } = bootContent({
    // The playurl request is long gone from the buffer; only the audio
    // half's traffic was observed recently.
    performance: createPerformanceFake([
      "https://b-x.edge.mountaintoys.cn:4483/upgcxcode/75/24/41875472475/41875472475-1-30216.m4s?os=mcdn&upsign=old",
    ]),
    fetch,
    window: selfWindow({
      code: 0,
      data: {
        dash: {
          video: [
            {
              id: 100022,
              baseUrl:
                "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-100022.m4s?upsign=av1",
            },
          ],
          audio: [
            {
              id: 30216,
              baseUrl:
                "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-30216.m4s?upsign=embedded",
            },
          ],
        },
      },
    }),
  });

  const res = await grab(ctl);
  assert.deepEqual(plain(res.urls), [
    // No observed video tail → the episode's best quality entry.
    "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-100022.m4s?upsign=av1",
    // The observed audio tail matched its embedded entry.
    "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/75/24/41875472475/41875472475-1-30216.m4s?upsign=embedded",
  ]);
  assert.equal(fetch.calls.length, 0, "the embedded payload needs no network");
  assert.match(res.note, /video \+ audio pair from the page's play info/);
});

test("grab: the note explains why a play-info pair was not built", async () => {
  const fetch = fetchFake({ default: new Error("offline") });
  const { ctl } = bootContent({
    performance: createPerformanceFake([
      PLAYINFO_URL,
      "https://upos.example.test/upgcxcode/41875472475/41875472475-1-100022.m4s?range=0-1000",
    ]),
    fetch,
  });

  const res = await grab(ctl);
  // The heuristic's lone segment still stands…
  assert.deepEqual(plain(res.urls), [
    "https://upos.example.test/upgcxcode/41875472475/41875472475-1-100022.m4s",
  ]);
  // …but the reason is visible in the note instead of being silent.
  assert.match(res.note, /\(play info: play-info fetch failed\)/);
});

test("grab: the note explains a play-info response without audio", async () => {
  const fetch = fetchFake({
    [PLAYINFO_URL]: {
      code: 0,
      data: {
        dash: {
          video: [
            {
              id: 100022,
              baseUrl:
                "https://upos.example.com/upgcxcode/41875472475/41875472475-1-100022.m4s?upsign=v",
            },
          ],
          audio: [],
        },
      },
    },
  });
  const { ctl } = bootContent({
    performance: createPerformanceFake([
      PLAYINFO_URL,
      "https://upos.example.test/upgcxcode/41875472475/41875472475-1-100022.m4s?range=0-1000",
    ]),
    fetch,
  });

  const res = await grab(ctl);
  assert.match(
    res.note,
    /\(play info: play info has no audio track for stream 41875472475\)/,
  );
});

// ── Page metadata (title / date / uploader) ─────────────────────────
//
// The merged file should carry a real title and date instead of
// "Packed by Bilibili XCoder", and the filename should be
// `<title>.mkv` rather than the raw stream id. bilibili's
// __INITIAL_STATE__ carries all three reliably.

test("grab: the page's metadata rides along for the merged file", async () => {
  // Content scripts read the DOM, not page JS globals: og:title (which
  // on bilibili repeats the decorated document title) must come back
  // with the site's `_哔哩哔哩_bilibili` suffix trimmed, and the date
  // from the itemprop meta.
  const og = new FakeElement("meta");
  og.setAttribute(
    "content",
    "Some Video Title_哔哩哔哩_bilibili",
  );
  const date = new FakeElement("meta");
  date.setAttribute("content", "2024-05-01 18:00");
  const author = new FakeElement("meta");
  author.setAttribute("content", "Some Uploader");
  const { ctl } = bootContent({
    title: "Some Video Title_哔哩哔哩_bilibili",
    anchors: {
      'meta[property="og:title"]': [og],
      'meta[itemprop="uploadDate"]': [date],
      'meta[property="og:video:owner"]': [author],
    },
    performance: createPerformanceFake([
      "https://upos.example.test/upgcxcode/1/2/42/42-1-100022.m4s?range=0-1000",
    ]),
  });

  const res = await grab(ctl);
  assert.deepEqual(plain(res.meta), {
    title: "Some Video Title",
    date: "2024-05-01 18:00",
    uploader: "Some Uploader",
  });
  // The streams themselves are unchanged by the metadata harvest.
  assert.deepEqual(plain(res.urls), [
    "https://upos.example.test/upgcxcode/1/2/42/42-1-100022.m4s",
  ]);
});

test("grab: pages without metadata still collect media", async () => {
  const { ctl } = bootContent({
    performance: createPerformanceFake([
      "https://upos.example.test/upgcxcode/1/2/42/42-1-100022.m4s?range=0-1000",
    ]),
  });
  const res = await grab(ctl);
  assert.deepEqual(plain(res.meta), {});
  assert.equal(res.urls.length, 1);
});
