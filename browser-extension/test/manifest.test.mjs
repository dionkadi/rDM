// Manifest and packaging invariants.
//
// The test that matters most here is the permission-coverage scan:
// the P0-1 bug was that background.js called
// `chrome.runtime.connectNative` while `manifest.json` had stopped
// declaring `nativeMessaging`. Nothing caught it for a whole
// release. This derives the required permissions from the source and
// fails if the manifest disagrees, in either direction.

import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { EXT_DIR } from "./harness.mjs";
import { toChromeManifest } from "../scripts/make-chrome-manifest.mjs";

const REPO_ROOT = path.dirname(EXT_DIR);
const read = (file) => fs.readFileSync(path.join(EXT_DIR, file), "utf8");
const manifest = JSON.parse(read("manifest.json"));
const SOURCE_FILES = ["background.js", "content.js", "popup.js"];
const source = SOURCE_FILES.map(read).join("\n");

/// Which manifest permission each API needs. Add a row when you reach
/// for a new gated API; the test will then tell you to declare it.
const PERMISSION_BY_API = {
  "chrome.downloads.": "downloads",
  "chrome.tabs.": "tabs",
  "chrome.storage.": "storage",
  "chrome.runtime.connectNative": "nativeMessaging",
  "chrome.runtime.sendNativeMessage": "nativeMessaging",
  "chrome.scripting.": "scripting",
  "chrome.cookies.": "cookies",
  "chrome.webRequest.": "webRequest",
  "chrome.contextMenus.": "contextMenus",
  "chrome.alarms.": "alarms",
  "chrome.history.": "history",
  "chrome.bookmarks.": "bookmarks",
  "chrome.notifications.": "notifications",
  "chrome.identity.": "identity",
  "chrome.idle.": "idle",
};

const uses = (api) => source.includes(api);

test("P0-1: every permission-gated API the extension calls is declared", () => {
  const declared = new Set(manifest.permissions || []);
  const missing = [];
  for (const [api, permission] of Object.entries(PERMISSION_BY_API)) {
    if (uses(api) && !declared.has(permission)) {
      missing.push(`${api} needs "${permission}"`);
    }
  }
  assert.deepEqual(
    missing,
    [],
    "manifest.json is missing permissions the code relies on",
  );
});

test("no declared permission is dead weight", () => {
  const declared = manifest.permissions || [];
  const used = new Set(
    Object.entries(PERMISSION_BY_API)
      .filter(([api]) => uses(api))
      .map(([, permission]) => permission),
  );
  const unused = declared.filter((p) => !used.has(p));
  assert.deepEqual(
    unused,
    [],
    "these permissions are declared but never used — drop them to keep the install prompt small",
  );
});

test("the popup is not exposed to every web page", () => {
  // `web_accessible_resources` let any site fingerprint that DM
  // Grabber is installed. The popup is opened by the browser itself
  // and never needs to be reachable from a page.
  assert.equal(
    "web_accessible_resources" in manifest,
    false,
    "popup.html/popup.js should not be web-accessible",
  );
});

test("the source manifest is a valid Firefox 109+ template", () => {
  assert.equal(manifest.manifest_version, 3);
  assert.ok(Array.isArray(manifest.background.scripts));
  assert.equal(
    manifest.background.service_worker,
    undefined,
    "background.service_worker breaks the Firefox event-page model",
  );
  assert.equal(manifest.browser_specific_settings.gecko.id, "dm-grabber@dm-project");
});

test("declared permissions are the minimal set we expect", () => {
  assert.deepEqual(
    [...(manifest.permissions || [])].sort(),
    ["downloads", "nativeMessaging", "storage", "tabs"],
  );
});

test("packaging keeps the two manifest shapes apart", () => {
  const pkg = read("scripts/package.sh");
  // The Chrome-shaped zip must not carry the Firefox-only permission.
  assert.match(
    pkg,
    /nativeMessaging/,
    "package.sh must reconcile the nativeMessaging divergence between the zip and the xpi",
  );
  const verify = read("scripts/verify-package.sh");
  assert.match(verify, /nativeMessaging/);
  assert.match(verify, /service_worker/);
});

test("the content script is only ever injected over http(s)", () => {
  // `<all_urls>` matches privileged about: pages too, where a
  // capture-phase click listener on documentElement breaks the page
  // (the new-tab search bar, IME input, …). The runtime guard is the
  // only thing keeping that from happening, so pin it down.
  const content = read("content.js");
  assert.match(content, /_proto === "http:" \|\| _proto === "https:"/);
});

test("nothing in the extension writes to innerHTML", () => {
  // Comments are stripped first — several of them explain *why* we
  // don't use innerHTML, which would otherwise trip this.
  const stripComments = (src) =>
    src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
  for (const file of SOURCE_FILES) {
    assert.ok(
      !stripComments(read(file)).includes("innerHTML"),
      `${file} must build DOM nodes explicitly, never via innerHTML`,
    );
  }
});

// ─── The Chromium manifest transform ────────────────────────────
//
// Loading the repo's manifest in Chrome is not a loud failure: Chrome
// ignores `background.scripts` under MV3, so the extension loads with no
// background at all and every message answers "Receiving end does not
// exist" — which reads like "the DM app isn't running" even when the
// probe says the app is fine. So the transform is load-bearing, and
// these tests pin its shape and its invocation.

test("the Chrome transform produces a loadable Chromium manifest", () => {
  const out = toChromeManifest(manifest);
  assert.equal(out.background.service_worker, "background.js");
  assert.equal(out.background.scripts, undefined);
  assert.equal(out.browser_specific_settings, undefined);
  assert.ok(
    !out.permissions.includes("nativeMessaging"),
    "nativeMessaging is Firefox-only",
  );
  // Everything else must survive the transform, or the extension loads
  // without its content script.
  assert.deepEqual(out.content_scripts, manifest.content_scripts);
  assert.deepEqual(out.host_permissions, manifest.host_permissions);
  assert.equal(out.action.default_popup, "popup.html");
  assert.deepEqual(
    [...out.permissions].sort(),
    ["downloads", "storage", "tabs"],
  );
});

test("the transform refuses an already-Chrome-shaped source", () => {
  // Otherwise the .xpi silently ends up with no service worker, which is
  // a worse failure than an error here.
  assert.throws(
    () => toChromeManifest({ ...manifest, background: { service_worker: "background.js" } }),
    /already Chrome-shaped/,
  );
  assert.throws(() => toChromeManifest({ ...manifest, background: {} }), /must be an array/);
});

test("the tooling runs when invoked through a symlinked path", (t) => {
  // Node resolves the entry module to its *real* path for
  // `import.meta.url` but leaves `process.argv[1]` exactly as given, so
  // the usual `import.meta.url === process.argv[1]` main-check silently
  // no-ops when a script is reached through a symlink. `pwd` in a shell
  // hands node precisely that shape, so `scripts/package.sh` hits this
  // whenever the checkout lives behind a symlink — and the build then
  // "succeeds" while emitting a manifest Chrome can't run. A silent
  // no-op is the worst failure mode for a build tool, hence this test.
  const scratch = path.join(REPO_ROOT, "target", "test-symlink-main");
  fs.rmSync(scratch, { recursive: true, force: true });
  fs.mkdirSync(scratch, { recursive: true });
  const linked = path.join(scratch, "scripts");
  try {
    fs.symlinkSync(path.join(EXT_DIR, "scripts"), linked, "dir");
  } catch (e) {
    fs.rmSync(scratch, { recursive: true, force: true });
    return t.skip(`symlinks unavailable: ${e.code}`);
  }

  try {
    // 1. The manifest transform must actually transform.
    const outFile = path.join(scratch, "out.json");
    execFileSync(
      process.execPath,
      [path.join(linked, "make-chrome-manifest.mjs"), path.join(EXT_DIR, "manifest.json"), outFile],
      { stdio: "pipe" },
    );
    const out = JSON.parse(fs.readFileSync(outFile, "utf8"));
    assert.equal(
      out.background.service_worker,
      "background.js",
      "transform through a symlink silently did nothing",
    );

    // 2. The probe's self-test must actually run (and pass).
    const probe = execFileSync(
      process.execPath,
      [path.join(linked, "check-host.mjs"), "--self-test"],
      { stdio: "pipe", encoding: "utf8" },
    );
    assert.match(
      probe,
      /all 5 probe cases classified correctly/,
      "probe through a symlink silently did nothing",
    );
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true });
  }
});
