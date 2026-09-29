#!/usr/bin/env bash
# Sanity-check the packaged browser extension. Used by the
# GitHub Actions release workflow's "Verify packages" step.
#
# Verifies both archives are well-formed and that the
# manifest.json at the root is the right shape for its target:
#   * `dm-grabber-<ver>.zip`  — Chrome-shaped
#     (background.service_worker, no gecko block). This is the
#     ONLY valid MV3 background key for Chrome; the previous
#     `background.scripts` shape was strictly rejected.
#   * `dm-grabber-<ver>.xpi`  — Firefox-shaped
#     (background.scripts + browser_specific_settings.gecko.id).
#     Firefox 109+ reads the scripts array as a non-persistent
#     event page; the gecko block fixes the extension ID to
#     dm-grabber@dm-project so the host manifest can list it
#     without an ID copy-paste.
#
# The packaging step in `scripts/package.sh` already runs an
# in-script self-verify before the zip is created. This script
# is the second-line check at the end of the CI job: if the
# .zip / .xpi got swapped, renamed, or rebuilt by a manual
# `zip`/`unzip` step, the failure surfaces here, NOT in the
# package step. That's the only way to catch the v0.4.1
# regression (Chrome's MV3 parser rejected the .zip's
# background.scripts) before it ships to a user.
#
# Usage: ./verify-package.sh <version-without-v>
#   e.g. ./verify-package.sh 0.4.2

set -euo pipefail

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
  echo "usage: $0 <version-without-v>" >&2
  exit 1
fi

ZIP="dm-grabber-${VERSION}.zip"
XPI="dm-grabber-${VERSION}.xpi"

echo "── contents of $ZIP ──"
unzip -l "$ZIP" | head -30
echo
echo "── contents of $XPI ──"
unzip -l "$XPI" | head -30
echo
echo "── sanity: $ZIP manifest.json valid JSON? ──"
python3 -c "import json, sys, zipfile; z = zipfile.ZipFile('$ZIP'); m = json.loads(z.read('manifest.json')); assert m['manifest_version'] == 3, m; print('OK', m['name'], m['version'])"
echo "── dev-only files must not ship ──"
for archive in "$ZIP" "$XPI"; do
  leaked="$(unzip -l "$archive" | awk '{print $4}' | grep -E '^(test/|scripts/)|\.test\.mjs$|^package\.(sh|ps1)$' || true)"
  if [[ -n "$leaked" ]]; then
    echo "error: $archive ships development files:" >&2
    echo "$leaked" >&2
    exit 1
  fi
done
echo "OK no development files"

echo "── shape: $ZIP is Chrome-shaped (background.service_worker)? ──"
unzip -p "$ZIP" manifest.json | python3 -c "import json, sys; m = json.load(sys.stdin); assert isinstance(m.get('background', {}).get('service_worker'), str), 'zip must have background.service_worker (Chrome MV3)'; assert 'scripts' not in m.get('background', {}), 'zip must NOT have background.scripts (Chrome rejects it)'; assert 'gecko' not in m.get('browser_specific_settings', {}), 'zip must NOT have browser_specific_settings.gecko'; assert 'nativeMessaging' not in m.get('permissions', []), 'zip must NOT request nativeMessaging (Chromium uses the WebSocket transport)'; print('OK chrome-shaped')"
echo "── icons: $ZIP ships PNG icons that are actually in the archive? ──"
# Chrome does not accept SVG icons, so pointing the Chrome manifest at
# the SVG leaves the extension with a generic letter tile in the toolbar
# and an error entry on chrome://extensions. Assert the zip's icons are
# raster AND present — a manifest naming a file that isn't in the zip is
# the same class of silent failure.
python3 - "$ZIP" <<'PY'
import json, sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    m = json.loads(z.read("manifest.json"))
    names = set(z.namelist())
    declared = sorted(
        {p for p in list(m.get("icons", {}).values())
         + list(m.get("action", {}).get("default_icon", {}).values())}
    )
    assert declared, "zip manifest declares no icons"
    svg = [p for p in declared if p.lower().endswith(".svg")]
    assert not svg, f"zip manifest points at SVG icons, which Chrome ignores: {svg}"
    absent = [p for p in declared if p not in names]
    assert not absent, f"zip manifest names icons that are not in the zip: {absent}"
    print("OK icons:", ", ".join(declared))
PY
echo "── shape: $XPI is Firefox-shaped (background.scripts + gecko.id)? ──"
unzip -p "$XPI" manifest.json | python3 -c "import json, sys; m = json.load(sys.stdin); assert isinstance(m.get('background', {}).get('scripts'), list), 'xpi must have background.scripts (Firefox 109+ event-page)'; assert 'service_worker' not in m.get('background', {}), 'xpi must NOT have background.service_worker'; assert m.get('browser_specific_settings', {}).get('gecko', {}).get('id'), 'xpi must have browser_specific_settings.gecko.id'; assert 'nativeMessaging' in m.get('permissions', []), 'xpi must request nativeMessaging or runtime.connectNative is not a function in Firefox'; print('OK firefox-shaped')"
