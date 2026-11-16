// Extract one version's section for the GitHub release body.
//
//   node scripts/changelog-section.mjs v0.6.0
//   node scripts/changelog-section.mjs v0.8.0 --require-notes   # no fallback
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
 * @param {string} markdown full CHANGELOG.md / RELEASE_NOTES.md
 * @param {string} version tag or version, with or without a leading `v`
 * @param {string} [label] file name used in error messages
 * @returns {string} the section body (no heading), trimmed
 * @throws when the version has no section
 */
export function extractSection(markdown, version, label = "CHANGELOG.md") {
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
      `${label} has no section for "${wanted}". ` +
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
    throw new Error(`${label} has an empty section for "${wanted}"`);
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
      return {
        body: extractSection(notesMd, version, "RELEASE_NOTES.md"),
        source: "RELEASE_NOTES.md",
      };
    } catch (e) {
      missing.push(`RELEASE_NOTES.md: ${stripFilePrefix(e.message)}`);
    }
  }
  if (changelogMd && changelogMd.trim()) {
    try {
      return {
        body: extractSection(changelogMd, version, "CHANGELOG.md"),
        source: "CHANGELOG.md",
      };
    } catch (e) {
      missing.push(`CHANGELOG.md: ${stripFilePrefix(e.message)}`);
    }
  }
  throw new Error(
    `no release body for "${wanted}". ${missing.join(" | ") || "both sources are empty"}`,
  );
}

// The inner extractor names the file it parsed; strip that prefix so the
// combined message can attribute each half itself ("RELEASE_NOTES.md:
// has no section …").
function stripFilePrefix(message) {
  return message.replace(/^(?:CHANGELOG|RELEASE_NOTES)\.md /, "");
}

/**
 * Assert that one specific file carries a section for `version`.
 * `resolveSection` cannot answer that on its own: it stops at the first
 * source that works, so a version present in RELEASE_NOTES.md but missing
 * from CHANGELOG.md resolves happily. `release.sh` requires both.
 *
 * @param {string|null} markdown full file contents
 * @param {string} version
 * @param {string} label file name for the error message
 * @throws when the file is absent or has no section
 */
export function enforceSection(markdown, version, label) {
  if (!markdown || !markdown.trim()) {
    throw new Error(`${label} is missing or empty`);
  }
  extractSection(markdown, version, label);
}

/**
 * Turn the changelog fallback into a hard failure. `release.yml` keeps
 * the fallback on purpose (by tag time the binaries are already built,
 * and a long body beats a failed release), but the local bump is the
 * last cheap moment to notice that the curated highlights are missing —
 * `release.sh` calls this via `--require-notes`.
 *
 * @param {{ body: string, source: string }} resolved
 * @param {string} version
 * @throws when the body did not come from RELEASE_NOTES.md
 */
export function enforceNotes(resolved, version) {
  if (resolved.source === "RELEASE_NOTES.md") return;
  const wanted = String(version).replace(/^v/i, "").trim();
  throw new Error(
    `RELEASE_NOTES.md has no section for "${wanted}", so the release body ` +
      `would fall back to the detailed CHANGELOG.md history. Add the ` +
      `curated highlights first (or pass through release.yml, which ` +
      `tolerates the fallback).`,
  );
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

  // 6. An empty notes section falls back to the changelog, and the error
  //    names the file it actually failed on.
  const emptyNotes = ["# Release Notes", "", "## [2.0.0] - 2026-01-01", ""].join("\n");
  if (resolveSection(emptyNotes, changelogFixture, "2.0.0").source !== "CHANGELOG.md") {
    throw new Error("self-test: an empty notes section must fall back to the changelog");
  }
  let emptyErr = null;
  try {
    resolveSection(emptyNotes, "", "2.0.0");
  } catch (e) {
    emptyErr = e;
  }
  if (!emptyErr || !emptyErr.message.includes('RELEASE_NOTES.md: has an empty section for "2.0.0"')) {
    throw new Error("self-test: the empty-section error must name RELEASE_NOTES.md, not CHANGELOG.md");
  }

  // 7. --require-notes rejects a changelog-sourced body and accepts a
  //    notes-sourced one.
  if (enforceNotes(fromNotes, "2.0.0") !== undefined) {
    throw new Error("self-test: require-notes must accept a notes-sourced body");
  }
  let notesErr = null;
  try {
    enforceNotes(fromChangelog, "1.9.0");
  } catch (e) {
    notesErr = e;
  }
  if (!notesErr || !notesErr.message.includes("RELEASE_NOTES.md has no section for \"1.9.0\"")) {
    throw new Error("self-test: require-notes must reject a changelog-sourced body");
  }

  // 8. enforceSection checks ONE file. resolveSection cannot do this: it
  //    stops at the first source that resolves, so a notes-only version
  //    looks fine even when the changelog has no section.
  enforceSection(changelogFixture, "2.0.0", "CHANGELOG.md"); // must not throw
  let sectionErr = null;
  try {
    enforceSection(changelogFixture, "9.9.9", "CHANGELOG.md");
  } catch (e) {
    sectionErr = e;
  }
  if (!sectionErr || !sectionErr.message.includes('CHANGELOG.md has no section for "9.9.9"')) {
    throw new Error("self-test: enforceSection must name the file it checked");
  }
  let emptyFileErr = null;
  try {
    enforceSection("", "2.0.0", "CHANGELOG.md");
  } catch (e) {
    emptyFileErr = e;
  }
  if (!emptyFileErr || !emptyFileErr.message.includes("is missing or empty")) {
    throw new Error("self-test: enforceSection must reject an empty file");
  }
  console.log("changelog-section: self-test passed");
}

function main(argv) {
  if (argv.includes("--self-test")) {
    selfTest();
    return;
  }
  // Refuse the CHANGELOG.md fallback when asked. `release.sh` passes this
  // so a tag cannot be cut with the curated highlights missing.
  const requireNotes = argv.includes("--require-notes");
  // Assert that CHANGELOG.md *itself* carries the section — resolveSection
  // stops at the first working source and so cannot notice a notes-only
  // version.
  const requireChangelog = argv.includes("--require-changelog");
  const version = argv.find((a) => !a.startsWith("--"));
  if (!version) {
    console.error(
      "usage: node scripts/changelog-section.mjs <version> [--require-notes|--require-changelog]\n" +
        "       node scripts/changelog-section.mjs --self-test",
    );
    process.exit(64);
  }
  const readOptional = (p) => (fs.existsSync(p) ? fs.readFileSync(p, "utf8") : null);
  try {
    const notesMd = readOptional(RELEASE_NOTES);
    const changelogMd = readOptional(CHANGELOG);
    const resolved = resolveSection(notesMd, changelogMd, version);
    if (requireNotes) enforceNotes(resolved, version);
    if (requireChangelog) enforceSection(changelogMd, version, "CHANGELOG.md");
    // Human-friendly provenance on stderr; stdout stays clean for the
    // `releaseBody` capture in release.yml.
    console.error(`release body for ${version} from ${resolved.source}`);
    process.stdout.write(resolved.body + "\n");
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
