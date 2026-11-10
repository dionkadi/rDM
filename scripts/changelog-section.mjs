// Extract one version's section from CHANGELOG.md, for the GitHub
// release body.
//
//   node scripts/changelog-section.mjs v0.6.0
//   node scripts/changelog-section.mjs --self-test
//
// The changelog is the source of truth for "what changed"; a release
// that only says "installers below" makes users open the repo to find
// out what they are downloading. `release.yml` pipes this into
// `releaseBody`, so the section has to exist for the tag being
// released — a missing section fails the release rather than silently
// publishing generic notes.
//
// Matching is deliberately loose about the heading shape, because the
// file has accumulated three styles over its lifetime:
//
//   ## [0.6.0] - 2026-11-10
//   ## [0.4.2] — 2026-09-05
//   ## 0.1.0 — initial release
//
// A leading `v` in the tag is accepted and ignored (tags are `v0.6.0`,
// headings are not).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const CHANGELOG = path.join(HERE, "..", "CHANGELOG.md");

const HEADING = /^##\s+\[?v?([0-9]+\.[0-9]+\.[0-9]+(?:[-+][^\]\s]+)?)\]?/;

/**
 * @param {string} markdown full CHANGELOG.md
 * @param {string} version tag or version, with or without a leading `v`
 * @returns {string} the section body (no heading), trimmed
 * @throws when the version has no section
 */
export function extractSection(markdown, version) {
  const wanted = String(version).replace(/^v/i, "").trim();
  const lines = markdown.split(/\r?\n/);
  let start = -1;
  let found = [];
  for (let i = 0; i < lines.length; i++) {
    const match = lines[i].match(HEADING);
    if (!match) continue;
    found.push(match[1]);
    if (start === -1 && match[1] === wanted) start = i;
  }
  if (start === -1) {
    throw new Error(
      `CHANGELOG.md has no section for "${wanted}". ` +
        `Found: ${found.join(", ") || "(none)"}`,
    );
  }
  // The section ends at the next level-2 heading (a later release).
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i++) {
    if (/^##\s/.test(lines[i])) {
      end = i;
      break;
    }
  }
  let body = lines.slice(start + 1, end).join("\n").trim();
  // A horizontal rule before the next release is a separator, not
  // content; drop it so the release body does not end in a stray `---`.
  body = body.replace(/\n*-{3,}\s*$/, "").trim();
  // Collapse runs of blank lines left behind by the trim above.
  body = body.replace(/\n{3,}/g, "\n\n");
  if (!body) {
    throw new Error(`the CHANGELOG section for "${wanted}" is empty`);
  }
  return body;
}

function selfTest() {
  const fixture = [
    "# Changelog",
    "",
    "## [2.0.0] - 2026-01-01",
    "",
    "### Added",
    "",
    "- new thing",
    "",
    "---",
    "",
    "## 1.9.0 — older",
    "",
    "- old thing",
    "",
  ].join("\n");

  const two = extractSection(fixture, "2.0.0");
  if (!two.includes("### Added") || !two.includes("- new thing")) {
    throw new Error("self-test: subsections must be included");
  }
  if (two.includes("old thing") || two.endsWith("---")) {
    throw new Error("self-test: the section must stop at the next release");
  }
  if (extractSection(fixture, "v2.0.0") !== two) {
    throw new Error("self-test: a leading v must be ignored");
  }
  if (!extractSection(fixture, "1.9.0").includes("old thing")) {
    throw new Error("self-test: the bracketed/bare heading styles must both parse");
  }
  let threw = false;
  try {
    extractSection(fixture, "9.9.9");
  } catch {
    threw = true;
  }
  if (!threw) throw new Error("self-test: an unknown version must fail loudly");
  console.log("changelog-section: self-test passed");
}

function main(argv) {
  if (argv.includes("--self-test")) {
    selfTest();
    return;
  }
  const version = argv[0];
  if (!version) {
    console.error("usage: node scripts/changelog-section.mjs <version|--self-test>");
    process.exit(64);
  }
  const markdown = fs.readFileSync(CHANGELOG, "utf8");
  try {
    process.stdout.write(extractSection(markdown, version) + "\n");
  } catch (e) {
    console.error(`error: ${e.message}`);
    process.exit(1);
  }
}

// Symlink-safe "am I the entry point?" — `process.argv[1]` keeps the
// logical path while `import.meta.url` is the real one, and this repo
// lives behind a symlink. Same rule as
// `browser-extension/scripts/lib/is-main.mjs`, inlined so this script
// stays dependency-free.
function isEntryPoint() {
  const entry = process.argv[1];
  if (!entry) return false;
  try {
    return fs.realpathSync(entry) === fs.realpathSync(fileURLToPath(import.meta.url));
  } catch (_e) {
    return false;
  }
}

if (isEntryPoint()) {
  main(process.argv.slice(2));
}
