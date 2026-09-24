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
