// Extract one version's section for the GitHub release body.
//
//   node scripts/changelog-section.mjs v0.6.0
//   node scripts/changelog-section.mjs --self-test
//
// Two sources, one contract (kept in sync by hand — every entry in the
// notes summarises a changelog section, never the other way round):
//
//   RELEASE_NOTES.md  concise per-release highlights — preferred
//   CHANGELOG.md      the detailed history — fallback
//
// A release body should be short when notes exist and complete when
// they don't: the section is taken from RELEASE_NOTES.md when the
// version has an entry there, otherwise from CHANGELOG.md. A version
// missing from both fails the release rather than silently publishing
// generic notes. `release.yml` pipes this into `releaseBody`.
//
// Matching is deliberately loose about the heading shape, because the
// changelog has accumulated three styles over its lifetime:
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
const RELEASE_NOTES = path.join(HERE, "..", "RELEASE_NOTES.md");

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

/**
 * Resolve the release body for `version`: prefer RELEASE_NOTES.md (the
 * concise, curated view) and fall back to CHANGELOG.md (the detailed
 * history) for versions without notes. A version missing from both
 * throws, listing what each file does have, so a broken sync is loud.
 *
 * @param {string|null} notesMd full RELEASE_NOTES.md (null/empty if absent)
 * @param {string|null} changelogMd full CHANGELOG.md (null/empty if absent)
 * @param {string} version tag or version, with or without a leading `v`
 * @returns {{ body: string, source: "RELEASE_NOTES.md" | "CHANGELOG.md" }}
 * @throws when the version has no section in either file
 */
export function resolveSection(notesMd, changelogMd, version) {
  const wanted = String(version).replace(/^v/i, "").trim();
  const missing = [];
  if (notesMd && notesMd.trim()) {
    try {
      return { body: extractSection(notesMd, version), source: "RELEASE_NOTES.md" };
    } catch (e) {
      missing.push(`RELEASE_NOTES.md: ${stripFilePrefix(e.message)}`);
    }
  }
  if (changelogMd && changelogMd.trim()) {
    try {
      return { body: extractSection(changelogMd, version), source: "CHANGELOG.md" };
    } catch (e) {
      missing.push(`CHANGELOG.md: ${stripFilePrefix(e.message)}`);
    }
  }
  throw new Error(
    `no release body for "${wanted}". ${missing.join(" | ") || "both sources are empty"}`,
  );
}

// The inner extractors name their own file in the error message; strip
// it so the combined message can attribute each half correctly.
function stripFilePrefix(message) {
  return message.replace(/^CHANGELOG\.md (has no section|the .* section)/, "$1");
}

function selfTest() {
  const notesFixture = [
    "# Release Notes",
    "",
    "## [2.0.0] - 2026-01-01",
    "",
    "- short highlight",
    "",
  ].join("\n");

  const changelogFixture = [
    "# Changelog",
    "",
    "## [Unreleased]",
    "",
    "### Added",
    "",
    "- wip thing",
    "",
    "## [2.0.0] - 2026-01-01",
    "",
    "### Added",
    "",
    "- new thing",
    "- another new thing",
    "",
    "---",
    "",
    "## 1.9.0 — older",
    "",
    "- old thing",
    "",
  ].join("\n");

  // 1. Notes win when the version has an entry there.
  const fromNotes = resolveSection(notesFixture, changelogFixture, "2.0.0");
  if (fromNotes.source !== "RELEASE_NOTES.md" || !fromNotes.body.includes("short highlight")) {
    throw new Error("self-test: notes must be preferred when the version has an entry");
  }
  if (fromNotes.body.includes("new thing")) {
    throw new Error("self-test: the notes body must not leak changelog content");
  }

  // 2. Changelog is the fallback for versions without notes.
  const fromChangelog = resolveSection(notesFixture, changelogFixture, "1.9.0");
  if (fromChangelog.source !== "CHANGELOG.md" || !fromChangelog.body.includes("old thing")) {
    throw new Error("self-test: the changelog must be the fallback source");
  }

  // 3. A version missing from both fails loudly, listing both files.
  let threw = null;
  try {
    resolveSection(notesFixture, changelogFixture, "9.9.9");
  } catch (e) {
    threw = e;
  }
  if (!threw || !threw.message.includes("RELEASE_NOTES.md") || !threw.message.includes("CHANGELOG.md")) {
    throw new Error("self-test: a version missing from both must fail with both files named");
  }

  // 4. A missing notes file degrades gracefully to the changelog.
  const noNotes = resolveSection(null, changelogFixture, "v2.0.0");
  if (noNotes.source !== "CHANGELOG.md" || !noNotes.body.includes("### Added")) {
    throw new Error("self-test: an absent notes file must fall back to the changelog");
  }

  // 5. extractSection standalone behaviour is unchanged.
  const two = extractSection(changelogFixture, "2.0.0");
  if (!two.includes("### Added") || !two.includes("- new thing")) {
    throw new Error("self-test: subsections must be included");
  }
  if (two.includes("old thing") || two.endsWith("---")) {
    throw new Error("self-test: the section must stop at the next release");
  }
  if (extractSection(changelogFixture, "v2.0.0") !== two) {
    throw new Error("self-test: a leading v must be ignored");
  }
  if (!extractSection(changelogFixture, "1.9.0").includes("old thing")) {
    throw new Error("self-test: the bracketed/bare heading styles must both parse");
  }
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
  const readOptional = (p) => (fs.existsSync(p) ? fs.readFileSync(p, "utf8") : null);
  try {
    const { body, source } = resolveSection(
      readOptional(RELEASE_NOTES),
      readOptional(CHANGELOG),
      version,
    );
    // Human-friendly provenance on stderr; stdout stays clean for the
    // `releaseBody` capture in release.yml.
    console.error(`release body for ${version} from ${source}`);
    process.stdout.write(body + "\n");
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
