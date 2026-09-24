// Fail the build on a `var(--x)` that nothing defines.
//
// CSS custom properties fail *silently*. An undefined `--x` makes the
// whole declaration invalid at computed-value time, so
//
//     .btn.primary { background: var(--color-accent); color: #fff; }
//
// simply drops the background and inherits the less specific rule —
// white text on the near-white elevated surface, i.e. an invisible
// button, with nothing in the console. Neither `svelte-check` nor
// Vite's build has anything to say about it. That is exactly how the
// CaptureDialog "Download" button disappeared: the
// `--color-accent*` aliases lived inside `[data-theme="light"]`
// only (so dark mode was already wrong), and an unrelated edit that
// deleted those three lines broke light mode too.
//
// So: every `var(--name)` in `src/` must either be defined somewhere
// in `src/`, or supply a fallback (`var(--name, value)`).
//
//   node scripts/check-css-vars.mjs
//
// Exits 1 with a file:line list when something is undefined.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SRC = path.join(ROOT, "src");
const EXTS = new Set([".css", ".svelte", ".ts", ".js", ".html"]);

// Vendored / generated trees we don't own.
const SKIP_DIRS = new Set(["node_modules", ".svelte-kit", "dist", "build", ".git"]);

function* walk(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (SKIP_DIRS.has(entry.name)) continue;
      yield* walk(path.join(dir, entry.name));
      continue;
    }
    if (EXTS.has(path.extname(entry.name))) yield path.join(dir, entry.name);
  }
}

// A definition is a declaration (`--x:`) or a Svelte style directive
// (`style:--x={…}`). Both appear literally in the source, so one
// regex per form is enough — no CSS parser required, and a false
// definition only makes the check slightly more permissive.
const DEF_RE = /(^|[\s;"{])--([a-zA-Z0-9_-]+)\s*:/g;
const DEF_DIRECTIVE_RE = /style:--([a-zA-Z0-9_-]+)\s*=/g;
// `var(--x)` or `var(--x, fallback)`.
const USE_RE = /var\(\s*--([a-zA-Z0-9_-]+)\s*(,)?/g;

/**
 * Strip comments before scanning.
 *
 * Load-bearing: a *commented-out* `--color-accent: …` must not count as
 * a definition, or the guard would happily pass while the property is
 * undefined — the exact false negative this script exists to prevent.
 * `//` is only stripped when not preceded by `:` so that `https://…`
 * inside a URL survives.
 *
 * (This is regex-based, so it is not a full lexer — but the failure
 * mode it guards against is a missing definition, and over-stripping
 * can only make the check stricter.)
 */
function stripComments(src) {
  return src
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/(^|[^:])\/\/[^\n]*/gm, "$1 ");
}

const defined = new Map(); // name -> first definition site
const usages = []; // {name, file, line, hasFallback}

for (const file of walk(SRC)) {
  const text = stripComments(fs.readFileSync(file, "utf8"));
  const rel = path.relative(ROOT, file);

  for (const re of [DEF_RE, DEF_DIRECTIVE_RE]) {
    re.lastIndex = 0;
    let m;
    while ((m = re.exec(text)) !== null) {
      const name = re === DEF_RE ? m[2] : m[1];
      if (!defined.has(name)) {
        defined.set(name, `${rel}:${text.slice(0, m.index).split("\n").length}`);
      }
    }
  }

  USE_RE.lastIndex = 0;
  let u;
  while ((u = USE_RE.exec(text)) !== null) {
    usages.push({
      name: u[1],
      hasFallback: !!u[2],
      file: rel,
      line: text.slice(0, u.index).split("\n").length,
    });
  }
}

const undefinedNoFallback = usages.filter(
  (u) => !defined.has(u.name) && !u.hasFallback,
);
const undefinedWithFallback = usages.filter(
  (u) => !defined.has(u.name) && u.hasFallback,
);

const fileCount = new Set(usages.map((u) => u.file)).size;

if (undefinedNoFallback.length > 0) {
  console.error(
    `\n${undefinedNoFallback.length} CSS custom propert${
      undefinedNoFallback.length === 1 ? "y is" : "ies are"
    } used but never defined (and has no fallback):\n`,
  );
  const byName = new Map();
  for (const u of undefinedNoFallback) {
    if (!byName.has(u.name)) byName.set(u.name, []);
    byName.get(u.name).push(`${u.file}:${u.line}`);
  }
  for (const [name, sites] of byName) {
    console.error(`  --${name}`);
    for (const site of sites) console.error(`      ${site}`);
  }
  console.error(
    "\nAn undefined custom property makes the whole declaration invalid, so\n" +
      "the element silently falls back to a less specific rule — which is how\n" +
      "white-on-white buttons happen. Define it (in the theme-agnostic `:root`\n" +
      "in src/app.css if it should follow the theme) or add a fallback.\n",
  );
  process.exit(1);
}

console.log(
  `CSS vars OK — ${usages.length} var() usage(s) across ${fileCount} file(s); ` +
    `${defined.size} propert${defined.size === 1 ? "y" : "ies"} defined` +
    (undefinedWithFallback.length
      ? `; ${undefinedWithFallback.length} rely on a fallback`
      : "") +
    ".",
);
