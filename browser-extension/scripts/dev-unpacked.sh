#!/usr/bin/env bash
# Build a Chrome-loadable *directory* for "Load unpacked".
#
# Why this exists: the repo's manifest.json is the **Firefox template**
# (`background.scripts`). Chrome ignores that key under Manifest V3, so
# pointing "Load unpacked" at the repo folder gives you an extension with
# **no background service worker** — the popup opens, content scripts
# inject, and every message fails with
#
#     Could not establish connection. Receiving end does not exist.
#
# which looks exactly like "the DM app isn't running". It is not: run
# `node scripts/check-host.mjs` and DM will answer fine.
#
# This reuses scripts/package.sh (so the directory is byte-identical to
# what a user installs, and inherits its manifest-shape and
# no-dev-files checks) and then unpacks the Chrome-shaped .zip into
# dist/chrome-unpacked/.
#
# Usage: ./scripts/dev-unpacked.sh
# Then:  chrome://extensions -> Developer mode -> Load unpacked -> paste
#        the path it prints.
#
# Re-run it after editing background.js/content.js/popup.js — the
# directory is a copy, not a symlink farm.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
VERSION="$(node -e 'process.stdout.write(JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).version)' "$EXT_DIR/manifest.json")"
OUT_DIR="$EXT_DIR/dist/chrome-unpacked"

if ! command -v unzip >/dev/null 2>&1; then
  echo "error: 'unzip' is required but not installed." >&2
  exit 1
fi

# Reuse the packaging path rather than re-implementing the staging and
# manifest transform. It prints its own progress.
"$SCRIPT_DIR/package.sh" "$VERSION"

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"
unzip -q -o "$EXT_DIR/dist/dm-grabber-${VERSION}.zip" -d "$OUT_DIR"

# Fail loudly here rather than letting the user discover it as
# "Receiving end does not exist" in Chrome ten minutes later.
node -e '
  const fs = require("fs");
  const m = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
  if (typeof m.background?.service_worker !== "string") {
    console.error(
      "error: staged manifest has no background.service_worker — Chrome would " +
      "load this extension with no background at all."
    );
    process.exit(1);
  }
  if (m.background?.scripts) {
    console.error("error: staged manifest still has background.scripts");
    process.exit(1);
  }
  if ((m.permissions || []).includes("nativeMessaging")) {
    console.error("error: staged manifest requests nativeMessaging (Firefox-only)");
    process.exit(1);
  }
  // Icons must be raster for Chrome: an .svg in `icons` leaves the
  // extension with no icon of its own (and an error entry on
  // chrome://extensions), which is not a loud failure.
  const iconPaths = [
    ...Object.values(m.icons || {}),
    ...Object.values(m.action?.default_icon || {}),
  ];
  if (iconPaths.length === 0) {
    console.error("error: staged manifest declares no icons");
    process.exit(1);
  }
  const svg = iconPaths.filter((p) => /\.svg$/i.test(p));
  if (svg.length > 0) {
    console.error(
      "error: staged Chrome manifest points at SVG icons (" +
        svg.join(", ") +
        ") — Chrome does not support them."
    );
    process.exit(1);
  }
' "$OUT_DIR/manifest.json"

# Also prove the entry points are actually there — a manifest pointing
# at a file that was never staged is the next-most-common way to end up
# with a silently inert extension.
for file in background.js content.js popup.js popup.html manifest.json \
            icons/icon-16.png icons/icon-32.png icons/icon-48.png icons/icon-128.png; do
  if [[ ! -f "$OUT_DIR/$file" ]]; then
    echo "error: $file is missing from $OUT_DIR" >&2
    exit 1
  fi
done

cat <<EOF

✔ Chrome-unpacked build ready (v${VERSION})

  1. Open  chrome://extensions
  2. Enable "Developer mode" (top right)
  3. "Load unpacked"  ->  paste this path:

     $OUT_DIR

  The DM app must be running. The toolbar icon should show a green
  "✓ DM host connected — WebSocket 127.0.0.1:9157" pill within a second.

  Edited background.js / content.js / popup.js? Re-run this script, then
  click Reload on the extension card.

EOF
