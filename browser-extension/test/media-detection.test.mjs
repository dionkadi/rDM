// Media detection: the paths that used to answer "no media found on
// this page" while the page was visibly playing video.
//
// Every test here names the specific drop it locks down:
//   * a relative `src` rejected for not being `^https?:`
//   * an MSE player whose only source is `blob:`
//   * a stream URL with no file extension (Content-Type is the only clue)
//   * a cross-origin iframe (invisible to the content script, visible
//     to the background's webRequest log)
//   * resource entries displaced from the browser's buffer
//   * cid grouping narrowing the candidate set to nothing

import test from "node:test";
import assert from "node:assert/strict";
import {
  createClock,
  createChromeMock,
  createPerformanceFake,
  loadExtension,
  bootBackground,
  FakeDocument,
  FakeElement,
  plain,
} from "./harness.mjs";

const START = 1_700_000_000_000;
const INSTALLED_AT = START - 100_000;
const PAGE = "https://site.example.test/watch";

async function settle(clock, rounds = 12) {
  for (let i = 0; i < rounds; i++) await clock.advance(0);
}

/** Boot content.js alone, with a document and a resource timeline. */
function bootContent({ doc, performance, window, observer, fetch } = {}) {
  const clock = createClock(START);
  const ctl = createChromeMock({ tabs: [{ id: 1, hasContentScript: true }] });
  const document = doc || new FakeDocument();
  loadExtension(["content.js"], {
    chrome: ctl,
    clock,
    document,
    location: { href: PAGE, protocol: "https:" },
    navigator: { userAgent: "Mozilla/5.0 (Test) DMTest/1.0" },
    performance: performance || createPerformanceFake([]),
    window,
    fetch,
    PerformanceObserver: observer,
  });
  return { clock, ctl, document };
}

/** Ask for media; `hints` mirrors what the background sends on grab. */
async function collect(ctl, hints) {
  let res = null;
  ctl.tabsSendMessage(1, hints ? { type: "collect", hints } : { type: "collect" }, (r) => {
    res = r;
  });
  for (let i = 0; i < 60 && res === null; i++) await Promise.resolve();
  return res;
}

function video(src, currentSrc) {
  const el = new FakeElement("video");
  el.setAttribute("src", src);
  if (currentSrc) el.currentSrc = currentSrc;
  return el;
}

/** A PerformanceObserver stand-in that a test can fire by hand. */
function createObserverFake() {
  const instances = [];
  class FakeObserver {
    constructor(cb) {
      this.cb = cb;
      instances.push(this);
    }
    observe() {}
    disconnect() {}
    trigger(names) {
      this.cb({ getEntries: () => names.map((name) => ({ name })) });
    }
  }
  return {
    FakeObserver,
    instances,
    latest: () => instances[instances.length - 1] || null,
  };
}

// ─── Content script ─────────────────────────────────────────────

test("a relative <video src> is resolved, not rejected", async () => {
  // `<video src="/v/clip.mp4" preload="none">` has no `currentSrc`
  // until playback starts, so the attribute is the only source. It was
  // discarded for not starting with http(s) — which is why these pages
  // only worked after the user pressed play.
  const doc = new FakeDocument({
    querySelectorAllResults: {
      "video, audio, source, picture source, track": [video("/v/clip.mp4")],
    },
  });
  const { ctl } = bootContent({ doc });
  const res = await collect(ctl);
  assert.deepEqual(plain(res.urls), ["https://site.example.test/v/clip.mp4"]);
  assert.match(res.note, /1 URL\(s\) selected/);
});

test("an MSE player is reported as a player, not as 'no media'", async () => {
  const doc = new FakeDocument({
    querySelectorAllResults: {
      "video, audio, source, picture source, track": [
        video("blob:https://site.example.test/1f2e"),
      ],
    },
  });
  const { ctl } = bootContent({ doc });
  const res = await collect(ctl);

  assert.deepEqual(plain(res.urls), []);
  assert.equal(res.diag.players, 1);
  assert.equal(res.diag.blobPlayers, 1);
  assert.match(res.note, /blob:/);
  assert.match(res.note, /press play/);
  assert.ok(!/^No media found/.test(res.note), res.note);
});

test("a typed hint makes an extensionless stream downloadable", async () => {
  // The YouTube shape: `/videoplayback?…` has no extension at all, so
  // neither the DOM nor the URL heuristics can describe it. The
  // background knows it is `video/mp4` from the response headers.
  const { ctl } = bootContent();
  const res = await collect(ctl, [
    { url: "https://cdn.example.test/videoplayback?itag=137", kind: "file" },
  ]);

  assert.deepEqual(plain(res.urls), [
    "https://cdn.example.test/videoplayback?itag=137",
  ]);
  assert.equal(res.diag.hints, 1);
  const fileNote = /media file/;
  assert.match(res.note, fileNote);
});

test("a typed hint marked manifest is offered as a manifest", async () => {
  const { ctl } = bootContent();
  const res = await collect(ctl, [
    { url: "https://cdn.example.test/live/index", kind: "manifest" },
  ]);

  assert.deepEqual(plain(res.urls), ["https://cdn.example.test/live/index"]);
  assert.match(res.note, /manifest/);
});

test("a same-origin iframe's own resource timeline is scanned", async () => {
  // `performance` is per-window: the top frame's buffer can never
  // contain an embedded player's requests, even when its DOM is
  // readable.
  const innerDoc = new FakeDocument();
  const frame = new FakeElement("iframe");
  frame.setAttribute("src", "https://site.example.test/embed/player");
  frame.contentDocument = innerDoc;
  frame.contentWindow = {
    performance: createPerformanceFake([
      "https://cdn.example.test/hls/master.m3u8",
    ]),
  };
  const doc = new FakeDocument({
    querySelectorAllResults: { "iframe": [frame] },
  });

  const { ctl } = bootContent({ doc });
  const res = await collect(ctl);

  assert.deepEqual(plain(res.urls), ["https://cdn.example.test/hls/master.m3u8"]);
  assert.equal(res.diag.frames, 2);
});

test("entries the browser buffer dropped survive in the extension's ring", async () => {
  // Once the resource buffer is full the browser stops recording NEW
  // entries — the requests made while watching are the ones lost. The
  // observer mirrors them before that can happen.
  const observer = createObserverFake();
  const { ctl } = bootContent({
    performance: createPerformanceFake([]),
    observer: observer.FakeObserver,
  });

  const live = observer.latest();
  assert.ok(live, "content.js must install a PerformanceObserver at load");
  live.trigger(["https://cdn.example.test/late/stream.m3u8"]);

  const res = await collect(ctl);
  assert.deepEqual(plain(res.urls), [
    "https://cdn.example.test/late/stream.m3u8",
  ]);
});

test("a base with an unparseable cid is not dropped by episode grouping", async () => {
  // The dominant cid belongs to the video half; the audio half's name
  // does not embed one. Grouping used to discard it, leaving a lone
  // segment (or nothing) instead of the pair the page was playing.
  const { ctl } = bootContent({
    performance: createPerformanceFake([
      "https://cdn.example.test/v/42138275245-1-30080.m4s?range=0-1",
      "https://cdn.example.test/v/42138275245-1-30080.m4s?range=1-2",
      "https://cdn.example.test/v/42138275245-1-30080.m4s?range=2-3",
      "https://cdn.example.test/v/audio-no-cid.m4s?range=0-1",
    ]),
  });

  const res = await collect(ctl);
  assert.equal(res.urls.length, 2, JSON.stringify(plain(res.urls)));
  assert.ok(res.urls.some((u) => /audio-no-cid/.test(u)), JSON.stringify(plain(res.urls)));
  assert.equal(res.pair, true);
});

test("page globals are read through wrappedJSObject", async () => {
  // The isolated world does not see page expandos. Firefox exposes
  // them through `wrappedJSObject`; without that indirection the whole
  // "embedded play info" source is dead code.
  const playinfo = {
    data: {
      dash: {
        video: [{ baseUrl: "https://cdn.example.test/v/9-1-30080.m4s?sign=v" }],
        audio: [{ baseUrl: "https://cdn.example.test/v/9-1-30216.m4s?sign=a" }],
      },
    },
  };
  const win = { wrappedJSObject: { __playinfo__: playinfo } };
  win.top = win;

  const { ctl } = bootContent({
    window: win,
    performance: createPerformanceFake([]),
    fetch: async () => ({ ok: true, json: async () => playinfo }),
  });

  const res = await collect(ctl);
  assert.equal(res.urls.length, 2, JSON.stringify(plain(res.urls)));
  assert.ok(res.urls.every((u) => /sign=/.test(u)), JSON.stringify(plain(res.urls)));
});

// ─── Background: hints, notes and the pair flag ─────────────────

function bootBackgroundOnly(options = {}) {
  const clock = createClock(START);
  const ctl = createChromeMock({
    storage: { dmInstallTime: INSTALLED_AT },
    tabs: [{ id: 1, hasContentScript: true }],
    ...options.chrome,
  });
  bootBackground(ctl, { clock });
  return { clock, ctl };
}

/** Pretend the content script answered `collect` with this payload. */
function answerCollect(ctl, payload) {
  ctl.chrome.runtime.onMessage.addListener((msg, _sender, respond) => {
    if (msg && msg.type === "collect") respond(payload);
  });
}

async function grab(clock, ctl) {
  let res = null;
  ctl.sendMessage({ type: "grab", tabId: 1 }, (r) => {
    res = r;
  });
  await settle(clock, 30);
  return res;
}

test("an empty grab keeps the content script's note and reason", async () => {
  const { clock, ctl } = bootBackgroundOnly();
  await settle(clock);
  answerCollect(ctl, {
    urls: [],
    note: "Found 1 media element(s) on this page, but their source is a blob: URL",
    diag: { players: 1, blobPlayers: 1 },
  });

  const res = await grab(clock, ctl);
  assert.equal(res.ok, true);
  assert.equal(res.reason, "player_without_url");
  assert.match(res.note, /blob:/, "the note must survive the empty path");
  assert.equal(res.diag.players, 1);
});

test("a grab forwards the pair flag to the host", async () => {
  const { clock, ctl } = bootBackgroundOnly();
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);
  answerCollect(ctl, {
    urls: ["https://cdn.example.test/v/1-1-30080.m4s", "https://cdn.example.test/v/1-1-30216.m4s"],
    pair: true,
    pageUrl: PAGE,
    note: "2 URL(s) selected",
  });

  const res = await grab(clock, ctl);
  assert.equal(res.delivered, true);
  const payload = ctl.ws.latest().payloads()[0];
  assert.equal(payload.type, "capture");
  assert.equal(payload.pair, true, "native-host reads msg.pair; dropping it un-pairs the grab");
});

test("webRequest hints reach the content script", async () => {
  const { clock, ctl } = bootBackgroundOnly();
  await settle(clock);
  answerCollect(ctl, { urls: [] });

  ctl.fireResponse({
    url: "https://cdn.example.test/videoplayback?itag=137",
    contentType: "video/mp4",
  });
  ctl.fireResponse({
    url: "https://cdn.example.test/hls/master.m3u8",
    contentType: "application/vnd.apple.mpegurl",
  });

  await grab(clock, ctl);
  const sent = ctl.calls.tabsSendMessage.find((c) => c.msg.type === "collect");
  assert.ok(sent, "the grab must ask the content script");
  const urls = plain(sent.msg.hints).map((h) => h.url).sort();
  assert.deepEqual(urls, [
    "https://cdn.example.test/hls/master.m3u8",
    "https://cdn.example.test/videoplayback?itag=137",
  ]);
});

test("HLS fragments are never offered as files", async () => {
  const { clock, ctl } = bootBackgroundOnly();
  await settle(clock);
  answerCollect(ctl, { urls: [] });

  ctl.fireResponse({
    url: "https://cdn.example.test/hls/seg-1.ts",
    contentType: "video/mp2t",
  });
  ctl.fireResponse({
    url: "https://cdn.example.test/blob",
    contentType: "application/octet-stream",
  });

  await grab(clock, ctl);
  const sent = ctl.calls.tabsSendMessage.find((c) => c.msg.type === "collect");
  assert.deepEqual(plain(sent.msg.hints), []);
});

test("a navigation clears what the tab had loaded", async () => {
  const { clock, ctl } = bootBackgroundOnly();
  await settle(clock);
  answerCollect(ctl, { urls: [] });

  ctl.fireResponse({
    url: "https://cdn.example.test/movie.mp4",
    contentType: "video/mp4",
  });
  ctl.fireTabUpdated(1, { status: "loading" });

  await grab(clock, ctl);
  const sent = ctl.calls.tabsSendMessage.find((c) => c.msg.type === "collect");
  assert.deepEqual(plain(sent.msg.hints), []);
});

test("the log outlives the service worker via storage.session", async () => {
  // The popup wakes a fresh worker *after* the media was played; an
  // in-memory-only log would be empty exactly when it is needed.
  const { clock, ctl } = bootBackgroundOnly();
  await settle(clock);

  ctl.fireResponse({
    url: "https://cdn.example.test/movie.mp4",
    contentType: "video/mp4",
  });
  await clock.advance(600);

  const stored = ctl.sessionData.dmMediaLog;
  assert.ok(stored, "the log must be persisted");
  assert.deepEqual(
    plain(stored["1"]).map((e) => e.url),
    ["https://cdn.example.test/movie.mp4"],
  );
  assert.equal(plain(stored["1"])[0].kind, "file");
});

test("a tab without the page script is reported as such, not as empty", async () => {
  const { clock, ctl } = bootBackgroundOnly({
    chrome: { tabs: [{ id: 1, hasContentScript: false }] },
  });
  await settle(clock);

  const res = await grab(clock, ctl);
  assert.equal(res.ok, true);
  assert.equal(res.reason, "no_content_script");
  assert.match(res.note, /reload the tab/i);
});

// ─── End to end: media only the background can see ──────────────

test("media inside a cross-origin iframe is found through the log", async () => {
  // The reported failure: the player is an embed on another origin, so
  // the content script sees neither its DOM nor its timeline. The
  // request is still visible to webRequest, which is the whole point of
  // moving observation up.
  const clock = createClock(START);
  const ctl = createChromeMock({
    storage: { dmInstallTime: INSTALLED_AT },
    tabs: [{ id: 1, hasContentScript: true }],
  });
  const doc = new FakeDocument();
  loadExtension(["background.js", "content.js"], {
    chrome: ctl,
    clock,
    document: doc,
    location: { href: PAGE, protocol: "https:" },
    navigator: { userAgent: "Mozilla/5.0 (Test) DMTest/1.0" },
    performance: createPerformanceFake([]),
    WebSocket: ctl.ws.FakeWebSocket,
  });
  await settle(clock);
  ctl.ws.latest().simulateOpen();
  await settle(clock);

  ctl.fireResponse({
    url: "https://player.other-origin.test/stream/8f3a?token=x",
    contentType: "video/mp4",
    frameId: 7,
  });

  const res = await grab(clock, ctl);
  assert.equal(res.ok, true);
  assert.equal(res.urls, 1, JSON.stringify(res));
  assert.equal(res.delivered, true);
  assert.match(res.note, /media file/);
  assert.equal(ctl.ws.latest().payloads()[0].urls[0], "https://player.other-origin.test/stream/8f3a?token=x");
});
