// A dependency-free test harness for the DM browser extension.
//
// There is no browser and no jsdom in this repo (and CI does not run
// `npm install` for the extension job), so the harness builds the
// three things the extension needs out of plain objects:
//
//   * a fake `chrome.*` that records calls and can invoke the
//     listeners the extension registers, so tests can *drive* real
//     events (`downloads.onCreated`, a tab click, a popup message)
//     instead of reaching into the extension's internals;
//   * a controllable clock, so reconnect backoff / queue TTL /
//     note expiry are deterministic instead of wall-clock dependent;
//   * a minimal DOM + a fake WebSocket.
//
// `background.js` and `content.js` are additionally loaded into the
// *same* vm context on purpose: they then share one `chrome` mock,
// which means `chrome.runtime.sendMessage` from the content script
// really does reach the background's `onMessage` listener. The tests
// exercise the real message plumbing rather than a stub of it.

import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

export const EXT_DIR = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

/**
 * Round-trip a value through JSON so it can be compared with
 * `assert.deepEqual` from `node:assert/strict`. Objects created inside
 * the vm context carry that realm's `Object.prototype`, and strict
 * deep-equality checks the prototype — so a plain `{a: 1}` from the
 * extension never equals an identical literal from the test file.
 */
export const plain = (value) => JSON.parse(JSON.stringify(value));

// ─── Clock ──────────────────────────────────────────────────────

/**
 * Deterministic timer/date source. `advance()` runs due timers in
 * order and lets microtasks settle between them, so `await
 * clock.advance(1000)` behaves like a second of real time with all
 * promises resolved.
 */
export function createClock(start = 1_700_000_000_000) {
  let now = start;
  let seq = 0;
  const tasks = new Map();

  async function advance(ms) {
    const target = now + ms;
    for (;;) {
      let next = null;
      for (const [id, task] of tasks) {
        if (task.at <= target && (next === null || task.at < next[1].at)) {
          next = [id, task];
        }
      }
      if (next === null) break;
      tasks.delete(next[0]);
      now = Math.max(now, next[1].at);
      next[1].fn();
      // Let promise callbacks that the timer kicked off run.
      for (let i = 0; i < 8; i++) await Promise.resolve();
    }
    now = target;
    for (let i = 0; i < 8; i++) await Promise.resolve();
  }

  class FakeDate extends Date {
    static now() {
      return now;
    }
    constructor(...args) {
      if (args.length === 0) super(now);
      else super(...args);
    }
  }

  return {
    now: () => now,
    advance,
    Date: FakeDate,
    setTimeout(fn, ms = 0) {
      const id = ++seq;
      tasks.set(id, { at: now + Math.max(0, ms), fn });
      return id;
    },
    clearTimeout(id) {
      tasks.delete(id);
    },
    setInterval(fn, ms) {
      // The extension's only interval is the popup's 1.5s status
      // poll. Recording it (rather than running it) keeps tests
      // deterministic; tests that care drive `refresh()` via a click.
      const id = ++seq;
      tasks.set(id, { at: now + Math.max(1, ms), fn, interval: Math.max(1, ms) });
      return id;
    },
    clearInterval(id) {
      tasks.delete(id);
    },
    pendingTimers: () => tasks.size,
  };
}

// ─── WebSocket ──────────────────────────────────────────────────

const WS_OPEN = 1;
const WS_CLOSED = 3;

export function createWebSocketFake() {
  const instances = [];

  class FakeWebSocket {
    constructor(url) {
      this.url = url;
      this.readyState = 0;
      this.sent = [];
      this._listeners = {};
      instances.push(this);
    }
    addEventListener(type, fn) {
      (this._listeners[type] ||= []).push(fn);
    }
    removeEventListener(type, fn) {
      const list = this._listeners[type];
      if (!list) return;
      const i = list.indexOf(fn);
      if (i >= 0) list.splice(i, 1);
    }
    send(data) {
      if (this.readyState !== WS_OPEN) throw new Error("WebSocket is not open");
      this.sent.push(data);
    }
    close() {
      this.readyState = WS_CLOSED;
      this._fire("close", { code: 1000 });
    }
    _fire(type, event) {
      for (const fn of [...(this._listeners[type] || [])]) fn(event || {});
    }
    /** Test control: complete the handshake. */
    simulateOpen() {
      this.readyState = WS_OPEN;
      this._fire("open", {});
    }
    /** Test control: the host went away. */
    simulateClose(code = 1006) {
      this.readyState = WS_CLOSED;
      this._fire("error", {});
      this._fire("close", { code });
    }
    /** Parsed payloads the extension has written to the socket. */
    payloads() {
      return this.sent.map((raw) => JSON.parse(raw));
    }
  }

  FakeWebSocket.OPEN = WS_OPEN;
  FakeWebSocket.CONNECTING = 0;
  FakeWebSocket.CLOSED = WS_CLOSED;

  return {
    FakeWebSocket,
    instances,
    latest: () => instances[instances.length - 1] || null,
    reset: () => {
      instances.length = 0;
    },
  };
}

// ─── chrome.* mock ──────────────────────────────────────────────

/**
 * @param {object} [options]
 * @param {object} [options.storage]      initial chrome.storage.local
 * @param {string} [options.runtimeId]    set to undefined to model an
 *                                        invalidated (orphaned) runtime
 * @param {Array}  [options.tabs]         [{id, hasContentScript}]
 * @param {boolean}[options.wsSupported]  false ⇒ `new WebSocket` throws
 */
export function createChromeMock(options = {}) {
  const opts = {
    storage: {},
    tabs: [{ id: 1, hasContentScript: true }],
    wsSupported: true,
    ...options,
  };

  const logs = { warn: [], info: [], error: [], log: [] };
  const listeners = {
    onMessage: [],
    onInstalled: [],
    onStartup: [],
    onCreated: [],
    onChanged: [],
  };

  const storageData = { ...opts.storage };
  const downloadsById = new Map();
  let downloadIdSeq = 0;

  let lastError = null;
  let recordRuntimeSends = true;
  const setLastError = (msg) => {
    lastError = { message: msg };
  };
  const clearLastError = () => {
    lastError = null;
  };

  const calls = {
    download: [],
    cancel: [],
    erase: [],
    removeFile: [],
    search: [],
    tabsQuery: [],
    tabsSendMessage: [],
    runtimeSendMessage: [],
    setBadgeText: [],
    connectNative: [],
  };

  // Optional gate that lets a test hold `storage.local.get` open, so
  // the "service worker was woken before storage resolved" window can
  // be reproduced deterministically.
  let storageHeld = !!options.holdStorageInitially;
  let releaseStorageGate = null;
  const storageGate = new Promise((resolve) => {
    releaseStorageGate = resolve;
  });

  const wsFake = createWebSocketFake();

  /** Deliver a runtime message to every registered onMessage listener. */
  function deliverMessage(msg, sender, cb) {
    let responded = false;
    let keepAlive = false;
    const sendResponse = (res) => {
      responded = true;
      if (cb) cb(res);
    };
    for (const handler of listeners.onMessage) {
      let r;
      try {
        r = handler(msg, sender || {}, sendResponse);
      } catch (e) {
        r = undefined;
        logs.error.push(String((e && e.stack) || e));
      }
      if (r === true) keepAlive = true;
    }
    if (!keepAlive && !responded && cb) cb(undefined);
  }

  const chrome = {
    runtime: {
      // `"runtimeId" in options` (rather than an undefined check) so a
      // test can model an *orphaned* content script whose runtime was
      // invalidated by an extension reload.
      id: "runtimeId" in options ? options.runtimeId : "dm-test-extension",
      get lastError() {
        return lastError;
      },
      onMessage: { addListener: (fn) => listeners.onMessage.push(fn) },
      onInstalled: { addListener: (fn) => listeners.onInstalled.push(fn) },
      onStartup: { addListener: (fn) => listeners.onStartup.push(fn) },
      sendMessage(msg, cb) {
        clearLastError();
        // Model an extension whose service worker never started — e.g.
        // a Chromium MV3 extension whose manifest declares only
        // `background.scripts`, which Chrome ignores.
        if (opts.noBackground) {
          setLastError(
            "Could not establish connection. Receiving end does not exist.",
          );
          if (cb) cb(undefined);
          return undefined;
        }
        // Only record messages the *extension* sends. Messages
        // injected by a test (via `ctl.sendMessage`) are not part of
        // the extension's behaviour.
        if (recordRuntimeSends) calls.runtimeSendMessage.push(msg);
        deliverMessage(msg, { id: chrome.runtime.id }, cb);
        return undefined;
      },
      connectNative(name) {
        clearLastError();
        calls.connectNative.push(name);
        if (!opts.nativeSupported) {
          throw new TypeError("chrome.runtime.connectNative is not a function");
        }
        if (opts.nativeFails) {
          setLastError("No such native application " + name);
        }
        const port = {
          _listeners: { message: [], disconnect: [] },
          _posted: [],
          onMessage: { addListener: (fn) => port._listeners.message.push(fn) },
          onDisconnect: {
            addListener: (fn) => port._listeners.disconnect.push(fn),
          },
          postMessage(msg) {
            if (opts.nativePostFails) throw new Error("postMessage failed");
            port._posted.push(msg);
          },
          disconnect() {
            clearLastError();
            setTimeout(() => {
              for (const fn of port._listeners.disconnect) fn();
            }, 0);
          },
        };
        return port;
      },
    },

    downloads: {
      onCreated: { addListener: (fn) => listeners.onCreated.push(fn) },
      download(definition, cb) {
        clearLastError();
        calls.download.push(definition);
        if (opts.downloadFails) {
          setLastError("Invalid filename");
          if (cb) cb(undefined);
          return;
        }
        const id = ++downloadIdSeq;
        downloadsById.set(id, {
          id,
          url: definition.url,
          filename: "derived-from-url.bin",
          state: "in_progress",
          startTime: new Date().toISOString(),
        });
        if (cb) cb(id);
      },
      cancel(id, cb) {
        clearLastError();
        calls.cancel.push(id);
        if (opts.cancelFails) setLastError("Download already cancelled");
        const item = downloadsById.get(id);
        // Cancelling an already-finished download is not something the
        // browser lets you do, so the state is left alone.
        if (item && item.state !== "complete") item.state = "interrupted";
        if (cb) cb();
      },
      erase(query, cb) {
        clearLastError();
        calls.erase.push(query && query.id);
        if (query && query.id != null) downloadsById.delete(query.id);
        if (cb) cb([query && query.id]);
      },
      removeFile(id, cb) {
        clearLastError();
        calls.removeFile.push(id);
        const item = downloadsById.get(id);
        if (item && item.state === "complete") {
          setLastError("File is complete; refusing to remove");
        }
        if (cb) cb();
      },
      search(query, cb) {
        clearLastError();
        calls.search.push(query);
        const item = downloadsById.get(query && query.id);
        if (cb) cb(item ? [item] : []);
      },
    },

    tabs: {
      query(queryInfo, cb) {
        clearLastError();
        calls.tabsQuery.push(queryInfo);
        const result = opts.tabs.map((t) => ({ id: t.id }));
        // Chromium returns a Promise when no callback is supplied, and
        // the popup relies on that (`await chrome.tabs.query(…)`).
        if (cb) {
          cb(result);
          return undefined;
        }
        return Promise.resolve(result);
      },
      sendMessage(tabId, msg, cb) {
        clearLastError();
        calls.tabsSendMessage.push({ tabId, msg });
        const tab = opts.tabs.find((t) => t.id === tabId);
        if (!tab || tab.hasContentScript === false) {
          setLastError(
            "Could not establish connection. Receiving end does not exist.",
          );
          if (cb) cb(undefined);
          return;
        }
        deliverMessage(msg, { tab }, cb);
      },
    },

    action: {
      setBadgeText(details) {
        clearLastError();
        calls.setBadgeText.push(details);
      },
      setBadgeBackgroundColor(details) {
        clearLastError();
        calls.setBadgeText.push(details);
      },
    },

    storage: {
      local: {
        async get(key) {
          clearLastError();
          if (storageHeld) await storageGate;
          if (key == null) return { ...storageData };
          const keys = Array.isArray(key) ? key : [key];
          const out = {};
          for (const k of keys) if (k in storageData) out[k] = storageData[k];
          return out;
        },
        async set(obj) {
          clearLastError();
          if (storageHeld) await storageGate;
          for (const [k, v] of Object.entries(obj)) storageData[k] = v;
        },
      },
      onChanged: { addListener: (fn) => listeners.onChanged.push(fn) },
    },
  };

  // ── Test control surface ──────────────────────────────────────
  const ctl = {
    chrome,
    calls,
    logs,
    storageData,
    ws: wsFake,
    listeners,
    /// Fire `downloads.onCreated` like the browser would.
    fireCreated(item) {
      const full = {
        id: ++downloadIdSeq,
        url: "https://files.example.test/archive.zip",
        filename: "archive.zip",
        state: "in_progress",
        startTime: new Date().toISOString(),
        ...item,
      };
      downloadsById.set(full.id, full);
      for (const fn of [...listeners.onCreated]) fn(full);
      return full;
    },
    fireInstalled(details) {
      for (const fn of [...listeners.onInstalled]) fn(details);
    },
    fireStartup() {
      for (const fn of [...listeners.onStartup]) fn();
    },
    fireStorageChanged(changes, area = "local") {
      for (const fn of [...listeners.onChanged]) fn(changes, area);
    },
    /// The popup / a content script sending a runtime message.
    sendMessage(msg, cb) {
      recordRuntimeSends = false;
      try {
        return chrome.runtime.sendMessage(msg, cb);
      } finally {
        recordRuntimeSends = true;
      }
    },
    tabsSendMessage(tabId, msg, cb) {
      return chrome.tabs.sendMessage(tabId, msg, cb);
    },
    setLastError,
    clearLastError,
    getLastError: () => lastError,
    downloads: downloadsById,
    /** Hold `storage.local.get/set` open until `releaseStorage()`. */
    holdStorage() {
      storageHeld = true;
    },
    releaseStorage() {
      storageHeld = false;
      if (releaseStorageGate) releaseStorageGate();
    },
    isStorageHeld: () => storageHeld,
  };

  return ctl;
}

// ─── DOM ────────────────────────────────────────────────────────

export class FakeElement {
  constructor(tag = "div") {
    this.tagName = String(tag).toUpperCase();
    this.nodeType = 1;
    this.children = [];
    this.attrs = {};
    this._text = "";
    this._listeners = {};
    this.style = {};
    this.className = "";
    this.disabled = false;
    this.title = "";
    this.checked = false;
    this.href = null;
    // Resolved URL of a media element (see HTMLMediaElement.currentSrc).
    // Distinct from the `src` attribute: for a `<video>` with a `<source>`
    // list, or one whose relative src was resolved against the document.
    this.currentSrc = null;
  }
  get firstChild() {
    return this.children[0] ?? null;
  }
  appendChild(child) {
    this.children.push(child);
    return child;
  }
  removeChild(child) {
    const i = this.children.indexOf(child);
    if (i >= 0) this.children.splice(i, 1);
    return child;
  }
  setAttribute(key, value) {
    this.attrs[key] = String(value);
    if (key === "href") this.href = String(value);
  }
  getAttribute(key) {
    return key in this.attrs ? this.attrs[key] : null;
  }
  hasAttribute(key) {
    return key in this.attrs;
  }
  addEventListener(type, fn) {
    (this._listeners[type] ||= []).push(fn);
  }
  removeEventListener(type, fn) {
    const list = this._listeners[type];
    if (!list) return;
    const i = list.indexOf(fn);
    if (i >= 0) list.splice(i, 1);
  }
  get textContent() {
    return this._text + this.children.map(textOf).join("");
  }
  set textContent(value) {
    this._text = String(value);
    this.children = [];
  }
  dispatch(type, event) {
    for (const fn of [...(this._listeners[type] || [])]) fn(event || {});
  }
  get classList() {
    const self = this;
    const read = () =>
      new Set(String(self.className).split(/\s+/).filter(Boolean));
    return {
      add: (c) => {
        const s = read();
        s.add(c);
        self.className = [...s].join(" ");
      },
      remove: (c) => {
        const s = read();
        s.delete(c);
        self.className = [...s].join(" ");
      },
      contains: (c) => read().has(c),
      toggle: (c) => {
        const s = read();
        if (s.has(c)) s.delete(c);
        else s.add(c);
        self.className = [...s].join(" ");
      },
    };
  }
}

export function textOf(node) {
  if (!node) return "";
  if (node.nodeType === 3) return node._text ?? "";
  return node.textContent ?? "";
}

export class FakeDocument {
  constructor(options = {}) {
    this.documentElement = new FakeElement("html");
    this.body = new FakeElement("body");
    this.readyState = "complete";
    this._byId = new Map();
    this._listeners = {};
    this.querySelectorAllResults = options.querySelectorAllResults || {};
  }
  createElement(tag) {
    return new FakeElement(tag);
  }
  createTextNode(text) {
    return { nodeType: 3, _text: String(text), textContent: String(text) };
  }
  getElementById(id) {
    return this._byId.get(id) ?? null;
  }
  /** Test helper: register an element as if `id="…"` were in the HTML. */
  register(id, element) {
    const el = element || new FakeElement("div");
    this._byId.set(id, el);
    return el;
  }
  querySelectorAll(selector) {
    return this.querySelectorAllResults[selector] ?? [];
  }
  addEventListener(type, fn) {
    (this._listeners[type] ||= []).push(fn);
  }
  removeEventListener() {}
}

/**
 * Build the anchor + click event the content script's capture-phase
 * listener will see. `over` lets a test set ctrlKey / button /
 * isTrusted / etc.
 */
export function makeClick(href, over = {}) {
  const anchor = new FakeElement("a");
  anchor.setAttribute("href", href);
  anchor.href = href;
  if (over.downloadAttr) anchor.setAttribute("download", over.downloadAttr);
  if (over.text) anchor._text = over.text;
  const event = {
    type: "click",
    button: 0,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    altKey: false,
    isTrusted: true,
    defaultPrevented: false,
    target: anchor,
    composedPath: () => [anchor],
    preventDefault() {
      this.defaultPrevented = true;
      this.prevented = true;
    },
    stopPropagation() {
      this.propagationStopped = true;
    },
    stopImmediatePropagation() {
      this.immediateStopped = true;
    },
    ...over.event,
  };
  return { anchor, event };
}

/**
 * Minimal `performance` stand-in for the content script's network scan
 * (`getEntriesByType("resource")`), which is how MSE players'
 * real stream URLs are discovered.
 */
export function createPerformanceFake(resourceUrls = []) {
  return {
    getEntriesByType(type) {
      if (type !== "resource") return [];
      return resourceUrls.map((name) => ({ name, entryType: "resource" }));
    },
  };
}

// ─── Loader ─────────────────────────────────────────────────────

/**
 * Run one or more extension scripts inside a single fresh vm context
 * so they share `chrome`, `document`, etc. — the way they share a
 * browser.
 */
export function loadExtension(scripts, env) {
  const clock = env.clock || createClock();
  // Accept either a control object (which carries `.chrome` plus the
  // recording arrays) or the bare `chrome` API.
  const chromeApi = env.chrome && env.chrome.chrome ? env.chrome.chrome : env.chrome;
  // Where the extension's console output is captured.
  const logs = env.logs ||
    env.chrome?.logs || { warn: [], info: [], error: [], log: [] };
  const sandbox = {
    console: {
      warn: (...a) => logs.warn.push(a.map(String).join(" ")),
      info: (...a) => logs.info.push(a.map(String).join(" ")),
      error: (...a) => logs.error.push(a.map(String).join(" ")),
      log: (...a) => logs.log.push(a.map(String).join(" ")),
    },
    setTimeout: clock.setTimeout,
    clearTimeout: clock.clearTimeout,
    setInterval: clock.setInterval,
    clearInterval: clock.clearInterval,
    Date: clock.Date,
    chrome: chromeApi,
  };
  if (env.WebSocket !== undefined) sandbox.WebSocket = env.WebSocket;
  if (env.location !== undefined) sandbox.location = env.location;
  if (env.document !== undefined) sandbox.document = env.document;
  if (env.navigator !== undefined) sandbox.navigator = env.navigator;
  if (env.performance !== undefined) sandbox.performance = env.performance;
  if (env.browser !== undefined) sandbox.browser = env.browser;

  vm.createContext(sandbox);
  for (const script of scripts) {
    const file = path.join(EXT_DIR, script);
    const src = fs.readFileSync(file, "utf8");
    vm.runInContext(src, sandbox, { filename: file });
  }
  return sandbox;
}

/**
 * Boot the background service worker (plus, optionally, a content
 * script or the popup) the way the browser does.
 */
export function bootBackground(chromeCtl, options = {}) {
  const clock = options.clock || createClock();
  const wsFake = chromeCtl.ws;
  const env = {
    // Accept either the control object or the bare `chrome` API.
    chrome: chromeCtl.chrome ?? chromeCtl,
    logs: chromeCtl.logs,
    clock,
    WebSocket: options.noWebSocket
      ? undefined
      : (options.WebSocket ?? wsFake.FakeWebSocket),
    location: options.location ?? {
      href: "https://example.test/page",
      protocol: "https:",
    },
    document: options.document,
    navigator: { userAgent: "Mozilla/5.0 (Test) DMTest/1.0" },
    browser: options.browser,
  };
  const sandbox = loadExtension(options.scripts ?? ["background.js"], env);
  return { sandbox, clock, wsFake };
}
