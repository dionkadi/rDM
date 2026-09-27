// Mutation check: proves the regression suite actually catches the
// bugs it claims to catch.
//
// A green suite is only meaningful if it fails when the bug is
// re-introduced. For each fixed defect this script re-introduces it,
// runs the one test that should notice, and requires that test to
// fail — then restores the file. If a mutation survives, the test is
// decorative and this exits non-zero.
//
// This file is deliberately *not* named `*.test.mjs`, so
// `node --test test/*.test.mjs` does not pick it up: it rewrites
// source files (and restores them in a `finally`, but still — don't
// run it in parallel with something else).
//
//   node test/mutation-check.mjs

import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const EXT_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const MUTATIONS = [
  {
    name: "P0-3  cancel a browser download DM never received",
    file: "background.js",
    find:
      '  if (!delivered) {\n    console.warn(\n      "DM Grabber: DM unreachable — leaving the browser\'s download in place:",',
    replace:
      '  if (false) {\n    console.warn(\n      "DM Grabber: DM unreachable — leaving the browser\'s download in place:",',
    test: "test/background.test.mjs",
    pattern: "P0-3: host unreachable",
  },
  {
    name: "P0-2  intercept clicks without checking host liveness",
    file: "content.js",
    find: "  if (!hostAlive) return false;\n",
    replace: "",
    test: "test/content.test.mjs",
    pattern: "P0-2: with DM unreachable a media click is not intercepted at all",
  },
  {
    name: "P0-2  never hand an undeliverable download back",
    file: "background.js",
    find: "      const handedBack = handBackToBrowser(msg.url, msg.filename);",
    replace: "      const handedBack = false;",
    test: "test/integration.test.mjs",
    pattern: "the stale-belief race",
  },
  {
    name: "P0-1  drop the nativeMessaging permission again",
    file: "manifest.json",
    find: '    "tabs",\n    "nativeMessaging",\n',
    replace: '    "tabs",\n',
    test: "test/manifest.test.mjs",
    pattern: "P0-1: every permission-gated API",
  },
  {
    name: "P1-4  register downloads.onCreated behind an await",
    file: "background.js",
    find: "chrome.downloads.onCreated.addListener((item) => {\n  void handleCreatedDownload(item);\n});",
    replace:
      "setTimeout(() => {\n  chrome.downloads.onCreated.addListener((item) => {\n    void handleCreatedDownload(item);\n  });\n}, 0);",
    test: "test/background.test.mjs",
    pattern: "P1-4: downloads.onCreated is registered synchronously",
  },
  {
    name: "P1-5  delete the file even though the download finished",
    file: "background.js",
    find: '      if (!found || found.state !== "complete") {',
    replace: "      if (true) {",
    test: "test/background.test.mjs",
    pattern: "P1-5: a download that had already finished is never deleted",
  },
  {
    name: "P1-5  ignore the takeover opt-out",
    file: "background.js",
    find: "  if (!state.settings.takeoverBrowserDownloads) return;\n",
    replace: "",
    test: "test/background.test.mjs",
    pattern: "P1-5: the takeover opt-out actually stops the takeover",
  },
  {
    name: "P1-6  let the popup claim success unconditionally",
    file: "popup.js",
    find: "      } else if (!res.delivered) {",
    replace: "      } else if (false) {",
    test: "test/integration.test.mjs",
    pattern: "P1-6: the popup refuses to claim success",
  },
  {
    name: "P2-7  hijack the page's own click handlers again",
    file: "content.js",
    find: "  e.preventDefault();\n  // Forward the URL to the background.",
    replace:
      "  e.preventDefault();\n  e.stopImmediatePropagation();\n  // Forward the URL to the background.",
    test: "test/content.test.mjs",
    pattern: "P2-7: interception no longer stops propagation",
  },
  {
    name: "conn  let a superseded socket's close clobber the live one",
    file: "background.js",
    find: "      if (ws !== socket) return; // a superseded attempt: `ws` is the live one\n",
    replace: "",
    test: "test/background.test.mjs",
    pattern: "a stale socket's close event cannot knock out the live socket",
  },
  {
    name: "diag  blame the DM host when the background is missing",
    file: "popup.js",
    find: '    return "background";',
    replace: '    return "host";',
    test: "test/integration.test.mjs",
    pattern: "a missing background is not blamed on the DM host",
  },
  {
    name: "ship  let a manifest without a service worker reach Chrome",
    file: "scripts/make-chrome-manifest.mjs",
    find: '  out.background = { service_worker: "background.js" };',
    replace: '  out.background = { scripts: ["background.js"] };',
    test: "test/manifest.test.mjs",
    pattern: "the Chrome transform produces a loadable Chromium manifest",
  },
  {
    name: "ship  leak the Firefox-only nativeMessaging permission into the zip",
    file: "scripts/make-chrome-manifest.mjs",
    find: '  out.permissions = (out.permissions || []).filter((p) => p !== "nativeMessaging");',
    replace: '  out.permissions = (out.permissions || []).slice();',
    test: "test/manifest.test.mjs",
    pattern: "the Chrome transform produces a loadable Chromium manifest",
  },
  {
    name: "tool  silently no-op the main check under a symlinked path",
    file: "scripts/lib/is-main.mjs",
    find: "    return fs.realpathSync(entry) === fs.realpathSync(fileURLToPath(metaUrl));",
    replace: "    return entry === fileURLToPath(metaUrl);",
    test: "test/manifest.test.mjs",
    pattern: "the tooling runs when invoked through a symlinked path",
  },
  {
    // The old mutations "offer the player's HTML again" (removing the
    // DOCUMENT_RE guard / the iframe filter) became behaviorally inert
    // once the grab was curated down to extension-verified candidates:
    // an HTML URL is now excluded by construction (rank < 2), so no
    // single-layer mutation can make it reappear. The live mutants for
    // the grab contract are the pair gate and the range-request strip
    // below.
    name: "grab  stop pairing the two stream halves",
    file: "content.js",
    find: "  if (episodeBases.length >= 2) {",
    replace: "  if (false) {",
    test: "test/content.test.mjs",
    pattern: "grab: a bilibili-style page yields the real streams",
  },
  {
    name: "grab  stop scanning network activity",
    file: "content.js",
    find: '      if (name && RESOURCE_MEDIA_RE.test(name)) add(name, "network");',
    replace: '      if (false) add(name, "network");',
    test: "test/content.test.mjs",
    pattern: "grab: a bilibili-style page yields the real streams",
  },
  {
    name: "grab  ignore the page's play info (stay half-observed)",
    file: "content.js",
    find: "  for (const src of sources) {",
    replace: "  for (const src of []) {",
    test: "test/content.test.mjs",
    pattern: "grab: the page's play info completes the pair",
  },
  {
    name: "grab  send byte-range fragments instead of stream bases",
    file: "content.js",
    find: "  function withoutRangeParam(raw) {",
    replace: "  function withoutRangeParam(raw) { return raw;",
    test: "test/content.test.mjs",
    pattern: "grab: one grab, one download",
  },
  {
    // The probe's newest distinction, and the one that names the
    // Windows failure: a listener that accepts the connection and
    // then hangs up. Collapsing it back into `no-response` restores
    // exactly the ambiguity this exists to remove — that connection is
    // dropped because the request could not be *read*, not because it
    // was read and ignored, and the two need different fixes.
    name: "probe  confuse a dropped connection with a silent listener",
    file: "scripts/check-host.mjs",
    find: '        finish({ kind: "closed" });',
    replace: '        finish({ kind: "no-response", eof: true });',
    test: "test/manifest.test.mjs",
    pattern: "the tooling runs when invoked through a symlinked path",
  },
];

const results = [];
let bailed = false;

for (const m of MUTATIONS) {
  const file = path.join(EXT_DIR, m.file);
  const original = fs.readFileSync(file, "utf8");
  const mutated = original.replace(m.find, m.replace);

  if (mutated === original) {
    results.push({ name: m.name, outcome: "STALE PATTERN" });
    continue;
  }

  fs.writeFileSync(file, mutated);
  let caught = false;
  let note = "";
  try {
    const out = execFileSync(
      "node",
      ["--test", "--test-name-pattern", m.pattern, m.test],
      { cwd: EXT_DIR, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
    );
    note = out.includes("not ok") ? "reported a failure but exited 0" : "the test passed";
  } catch (e) {
    const out = `${e.stdout || ""}${e.stderr || ""}`;
    caught = out.includes("not ok");
    note = caught ? "the test failed, as it must" : "exited non-zero with no failure";
  } finally {
    // Restore unconditionally — a stray mutation here would be worse
    // than a missed mutant.
    try {
      fs.writeFileSync(file, original);
    } catch (restoreError) {
      bailed = true;
      console.error(
        `\nFATAL: could not restore ${m.file}. Restore it from git before continuing.\n`,
        restoreError,
      );
    }
  }

  results.push({ name: m.name, outcome: caught ? "CAUGHT" : `MISSED (${note})` });
}

console.log("");
for (const r of results) {
  const ok = r.outcome === "CAUGHT";
  console.log(`${ok ? "  caught " : "  MISSED "} ${r.name}${ok ? "" : " — " + r.outcome}`);
}
const missed = results.filter((r) => r.outcome !== "CAUGHT").length;
console.log(
  missed === 0
    ? `\nall ${results.length} mutations caught by the suite\n`
    : `\n${missed}/${results.length} mutations NOT caught \u2014 the suite has blind spots\n`,
);
process.exit(missed === 0 && !bailed ? 0 : 1);
