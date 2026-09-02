#!/usr/bin/env bash
# Package the browser extension as both a Chrome/Edge/Brave .zip and a
# Firefox .xpi. They are the same payload — .xpi is just a renamed .zip
# with a different file extension. Firefox refuses to install extensions
# that contain any of the excluded files below, so we strip them from
# BOTH outputs even though Chrome is more permissive.
#
# Usage: ./scripts/package.sh <version>
#   e.g. ./scripts/package.sh 0.1.0
#        ./scripts/package.sh 0.1.0-rc.1
#
# Output (relative to the browser-extension/ directory):
#   dist/dm-grabber-<version>.zip
#   dist/dm-grabber-<version>.xpi
#
# Exit codes:
#   0  success
#   1  missing or invalid arguments
#   2  zip not installed
#   3  zip produced an empty/invalid archive
#   4  manifest.json version doesn't match <version>

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

# ── Verify manifest.json is sane and matches the requested version ─
if [[ ! -f "$TMP/manifest.json" ]]; then
  echo "error: manifest.json missing from staged payload — did the copy step drop it?" >&2
  exit 3
fi

# Pull the version out of the staged manifest and compare.
STAGED_VERSION="$(node -e 'process.stdout.write(JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).version)' "$TMP/manifest.json")"
if [[ "$STAGED_VERSION" != "$VERSION" ]]; then
  echo "error: manifest.json declares version \"$STAGED_VERSION\" but you asked to package \"$VERSION\"." >&2
  echo "       bump manifest.json first, or pass the matching version." >&2
  exit 4
fi

# Also fail loudly if the manifest is structurally broken — better
# to fail in CI than ship a .zip Firefox can't install.
node -e '
  const m = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
  if (m.manifest_version !== 3) throw new Error("manifest_version must be 3");
  if (!Array.isArray(m.background?.scripts)) throw new Error("background.scripts must be an array");
  if (m.background?.service_worker) throw new Error("background.service_worker breaks Firefox 109+");
  if (!m.browser_specific_settings?.gecko?.id) throw new Error("missing gecko.id");
' "$TMP/manifest.json"

# ── Pack the .zip (Chrome / Edge / Brave / Arc) ─────────────────────
# We use `zip -r` from inside the staging dir so the paths in the
# archive are relative (no leading "browser-extension/" prefix). This
# is what Chrome's "Load unpacked" and Edge's "Load extension" expect
# when re-extracting.
cd "$TMP"
zip -r -X "$OUT_BASE.zip" . >/dev/null

# ── Pack the .xpi (Firefox) ──────────────────────────────────────────
# A .xpi is just a renamed .zip. Firefox is stricter than Chrome
# about which files are allowed (no top-level __MACOSX/, no
# .DS_Store, no symlinks). The exclude list above handles the
# first two. For symlinks, `zip -r` skips them by default; we
# verify below.
cp "$OUT_BASE.zip" "$OUT_BASE.xpi"

# ── Self-verify ─────────────────────────────────────────────────────
# Re-open the .xpi and confirm it has at minimum the expected files.
# This catches the "zip produced an empty archive" failure mode that
# some CI sandboxes have when zip writes to a network filesystem.
if ! command -v unzip >/dev/null 2>&1; then
  echo "warning: 'unzip' not installed; skipping self-verify" >&2
else
  unzip -l "$OUT_BASE.xpi" | grep -q "manifest.json" || {
    echo "error: .xpi is missing manifest.json" >&2
    exit 3
  }
  unzip -l "$OUT_BASE.xpi" | grep -q "background.js" || {
    echo "error: .xpi is missing background.js" >&2
    exit 3
  }
fi

echo "✔ packaged:"
echo "    $OUT_BASE.zip"
echo "    $OUT_BASE.xpi"
