#!/usr/bin/env bash
# Package the browser extension as a Chrome/Edge/Brave .zip and a
# Firefox .xpi. The payload is identical, but the manifest.json at
# the root is rewritten to match the target's MV3 background-page
# model:
#   * Chrome / Edge / Brave / Arc — `background.service_worker` is
#     the ONLY valid MV3 key. The previous source manifest used
#     `background.scripts`, which Chrome's MV3 parser strictly
#     rejects with `'background.scripts' requires manifest version
#     of 2 or lower` — the extension silently failed to install
#     on those browsers.
#   * Firefox 109+ — reads `background.scripts` as a non-persistent
#     event page. `background.service_worker` (added in Firefox
#     121) is also accepted, but the
#     `browser_specific_settings.gecko` block forces event-page
#     mode regardless, so we keep the scripts-array form for
#     maximum compatibility.
#
# The source `manifest.json` is the Firefox-shaped template
# (`background.scripts` + `browser_specific_settings.gecko`). It
# stays Firefox-shaped so devs can sideload on Firefox without
# running the package script. The Chrome manifest is generated
# from the same source at pack time.
#
# Usage: ./scripts/package.sh <version>
#   e.g. ./scripts/package.sh 0.1.0
#        ./scripts/package.sh 0.1.0-rc.1
#
# Output (relative to the browser-extension/ directory):
#   dist/dm-grabber-<version>.zip     (Chrome-shaped manifest)
#   dist/dm-grabber-<version>.xpi     (Firefox-shaped manifest)
#
# Exit codes:
#   0  success
#   1  missing or invalid arguments
#   2  zip not installed
#   3  zip produced an empty/invalid archive
#   4  manifest.json version doesn't match <version>
#   5  source manifest is structurally broken
#   6  Chrome-shaped manifest fails its own structural check

set -euo pipefail

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
  echo "usage: $0 <version>" >&2
  echo "  e.g. $0 0.1.0" >&2
  exit 1
fi

# Strip the leading "v" if present (matches the tag format used by
# .github/workflows/release.yml). Firefox's about:addons displays the
# version as-is, so a literal "v0.1.0" would look wrong.
VERSION="${VERSION#v}"

# Resolve paths relative to this script so the package step works
# whether it's invoked from the repo root, from browser-extension/,
# or from CI (which always uses browser-extension as the working dir).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$EXT_DIR/dist"

if ! command -v zip >/dev/null 2>&1; then
  echo "error: 'zip' is not installed. On Debian/Ubuntu: sudo apt-get install -y zip" >&2
  echo "       On Fedora:                  sudo dnf install -y zip" >&2
  echo "       On macOS:                   brew install zip" >&2
  exit 2
fi

mkdir -p "$DIST_DIR"
OUT_BASE="$DIST_DIR/dm-grabber-$VERSION"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ── Stage the payload ────────────────────────────────────────────────
# Copy everything except the files we explicitly exclude. Using
# rsync-style copy would be nicer, but cp -r is more portable and
# the extension payload is tiny.
shopt -s dotglob
for entry in "$EXT_DIR"/*; do
  name="$(basename "$entry")"
  case "$name" in
  # Build / VCS / OS junk — never ship these.
  .git) continue ;;
  dist) continue ;; # the output dir itself
  node_modules) continue ;;
  scripts) continue ;; # this very script; lives in the repo
    # for re-runnability, not for users
  *.log) continue ;;
  .DS_Store) continue ;;
  Thumbs.db) continue ;;

  # The native-messaging host manifest templates belong to the
  # host itself, not the extension. The user installs them
  # separately (see INSTALL.md). Shipping them inside the .zip
  # confuses people into thinking the extension needs them.
  com.app.dm.native*.json) continue ;;

  # INSTALL.md is intentionally kept in the package — Firefox
  # addon reviewers and end users may want to read it. If you
  # want to drop it, add `INSTALL.md` to the skip list above.

  *) cp -R "$entry" "$TMP/" ;;
  esac
done
shopt -u dotglob

# ── Verify the source manifest is sane ──────────────────────────────
# The source manifest is the Firefox-shaped template. We validate
# it as a Firefox manifest: it must have a `scripts` array under
# `background` and a `gecko.id` so the local-dev sideload on
# Firefox 109+ still works.
SRC_MANIFEST="$EXT_DIR/manifest.json"
if [[ ! -f "$SRC_MANIFEST" ]]; then
  echo "error: source manifest.json missing from $EXT_DIR" >&2
  exit 3
fi

STAGED_VERSION="$(node -e 'process.stdout.write(JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).version)' "$SRC_MANIFEST")"
if [[ "$STAGED_VERSION" != "$VERSION" ]]; then
  echo "error: source manifest.json declares version \"$STAGED_VERSION\" but you asked to package \"$VERSION\"." >&2
  echo "       bump manifest.json first, or pass the matching version." >&2
  exit 4
fi

node -e '
  const m = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
  if (m.manifest_version !== 3) throw new Error("manifest_version must be 3");
  if (!Array.isArray(m.background?.scripts)) throw new Error("source manifest.background.scripts must be an array (Firefox template)");
  if (m.background?.service_worker) throw new Error("source manifest.background.service_worker breaks Firefox 109+ event-page model");
  if (!m.browser_specific_settings?.gecko?.id) throw new Error("source manifest missing browser_specific_settings.gecko.id (Firefox template)");
' "$SRC_MANIFEST" || exit 5

# ── Generate the Chrome-shaped manifest ─────────────────────────────
# Chrome MV3 strictly rejects `background.scripts` (the error
# message in the user's chrome://extensions page is literally
# "'background.scripts' requires manifest version of 2 or lower").
# We rewrite the staged `manifest.json` for the .zip pass:
#   * `background.scripts`   -> `background.service_worker: "background.js"`
#   * `browser_specific_settings.gecko` is removed (Chrome ignores
#     it but logs a warning; better to omit it entirely).
# The Firefox .xpi uses the source manifest as-is.
CHROME_MANIFEST="$TMP/manifest.json" # already in the staged tree
node -e '
  const fs = require("fs");
  const f = process.argv[1];
  const m = JSON.parse(fs.readFileSync(f, "utf8"));
  // Refuse to overwrite if the source is already Chrome-shaped
  // (that means the dev flipped it manually and we would silently
  // emit a Firefox-shaped .xpi with no service-worker).
  if (m.background?.service_worker) {
    throw new Error("source manifest is already Chrome-shaped; restore `background.scripts` for the Firefox template");
  }
  // Construct the Chrome variant.
  const out = { ...m };
  out.background = { service_worker: "background.js" };
  delete out.browser_specific_settings;
  fs.writeFileSync(f, JSON.stringify(out, null, 2) + "\n");
' "$CHROME_MANIFEST" || exit 6

# ── Pack the .zip (Chrome / Edge / Brave / Arc) ─────────────────────
# The staged `manifest.json` is now Chrome-shaped (service_worker,
# no gecko block). zip -r from inside the staging dir so the paths
# in the archive are relative (no leading "browser-extension/"
# prefix). This is what Chrome's "Load unpacked" and Edge's "Load
# extension" expect when re-extracting.
cd "$TMP"
zip -r -X "$OUT_BASE.zip" . >/dev/null

# ── Pack the .xpi (Firefox) ──────────────────────────────────────────
# Firefox needs the source-shaped manifest (background.scripts +
# gecko.id). We re-stage the original manifest by re-copying it on
# top of the (now Chrome-shaped) staged one, then re-pack.
# A .xpi is just a renamed .zip. Firefox is stricter than Chrome
# about which files are allowed (no top-level __MACOSX/, no
# .DS_Store, no symlinks). The exclude list above handles the
# first two. For symlinks, `zip -r` skips them by default.
cp "$SRC_MANIFEST" "$TMP/manifest.json"
cd "$TMP"
zip -r -X "$OUT_BASE.xpi" . >/dev/null

# ── Self-verify ─────────────────────────────────────────────────────
# Re-open the .xpi and confirm it has at minimum the expected files,
# and that the .zip's manifest has `service_worker` (the regression
# we just fixed) and the .xpi's manifest has `scripts` (Firefox
# 109+ event-page compat). This catches the "zip produced an empty
# archive" failure mode that some CI sandboxes have when zip
# writes to a network filesystem, and the "we accidentally shipped
# the wrong manifest shape" failure mode that bit us in v0.4.1.
if ! command -v unzip >/dev/null 2>&1; then
  echo "warning: 'unzip' not installed; skipping self-verify" >&2
else
  unzip -l "$OUT_BASE.zip" | grep -q "manifest.json" || {
    echo "error: .zip is missing manifest.json" >&2
    exit 3
  }
  unzip -l "$OUT_BASE.zip" | grep -q "background.js" || {
    echo "error: .zip is missing background.js" >&2
    exit 3
  }
  unzip -l "$OUT_BASE.xpi" | grep -q "manifest.json" || {
    echo "error: .xpi is missing manifest.json" >&2
    exit 3
  }
  unzip -l "$OUT_BASE.xpi" | grep -q "background.js" || {
    echo "error: .xpi is missing background.js" >&2
    exit 3
  }

  # Confirm the two manifests are the right shape. `unzip -p` writes
  # the file's contents to stdout; node then asserts the right key.
  unzip -p "$OUT_BASE.zip" manifest.json | node -e '
    let buf = "";
    process.stdin.on("data", (c) => { buf += c; });
    process.stdin.on("end", () => {
      const m = JSON.parse(buf);
      if (typeof m.background?.service_worker !== "string") {
        console.error("error: .zip manifest.background.service_worker is not a string"); process.exit(1);
      }
      if (m.background?.scripts) {
        console.error("error: .zip manifest must NOT have background.scripts (Chrome rejects it)"); process.exit(1);
      }
      if (m.browser_specific_settings?.gecko) {
        console.error("error: .zip manifest must NOT have browser_specific_settings.gecko (Chrome logs a warning)"); process.exit(1);
      }
    });
  ' || exit 6

  unzip -p "$OUT_BASE.xpi" manifest.json | node -e '
    let buf = "";
    process.stdin.on("data", (c) => { buf += c; });
    process.stdin.on("end", () => {
      const m = JSON.parse(buf);
      if (!Array.isArray(m.background?.scripts)) {
        console.error("error: .xpi manifest.background.scripts is not an array (Firefox 109+ requires this)"); process.exit(1);
      }
      if (m.background?.service_worker) {
        console.error("error: .xpi manifest must NOT have background.service_worker (Firefox 109+ event-page model expects scripts)"); process.exit(1);
      }
      if (!m.browser_specific_settings?.gecko?.id) {
        console.error("error: .xpi manifest missing browser_specific_settings.gecko.id"); process.exit(1);
      }
    });
  ' || exit 6
fi

echo "✔ packaged:"
echo "    $OUT_BASE.zip   (Chrome-shaped: background.service_worker)"
echo "    $OUT_BASE.xpi   (Firefox-shaped: background.scripts + gecko.id)"
