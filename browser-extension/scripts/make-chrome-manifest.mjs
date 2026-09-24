// Turn the Firefox-shaped source manifest into the Chromium-shaped one.
//
// The repo's `browser-extension/manifest.json` is the **Firefox
// template**: `background.scripts` (Firefox 109-127 rejects
// `background.service_worker`), plus a `browser_specific_settings.gecko`
// block. Chromium needs the opposite, and loading the repo folder
// directly in Chrome does *not* fail loudly — Chrome ignores
// `background.scripts` under Manifest V3, so the extension loads with
// **no background context at all**: the popup opens, content scripts
// inject, and every `runtime.sendMessage` answers
//
//     Could not establish connection. Receiving end does not exist.
//
// which reads exactly like "the DM app isn't running" even when it is.
// Hence this single implementation, used by both `package.sh` (for the
// `.zip`) and `scripts/dev-unpacked.sh` (for a Load-unpacked directory).
//
//   node scripts/make-chrome-manifest.mjs <source> <output>

import fs from "node:fs";
import { isMainModule } from "./lib/is-main.mjs";

/**
 * @param {object} source parsed Firefox-shaped manifest
 * @returns {object} Chromium-shaped manifest
 * @throws if `source` is not the Firefox template
 */
export function toChromeManifest(source) {
  if (source.manifest_version !== 3) {
    throw new Error("manifest_version must be 3");
  }
  // Check this before the shape check below: a source that is already
  // Chrome-shaped is a specific, likely mistake (someone flipped the
  // manifest by hand) and deserves its own message. If we let the
  // scripts check catch it first, the .xpi would end up with no service
  // worker — a much worse failure than an error here.
  if (source.background?.service_worker) {
    throw new Error(
      "source manifest is already Chrome-shaped; restore " +
        "`background.scripts` for the Firefox template",
    );
  }
  if (!Array.isArray(source.background?.scripts)) {
    throw new Error(
      "source manifest.background.scripts must be an array (Firefox template)",
    );
  }
  const out = { ...source };
  out.background = { service_worker: "background.js" };
  // Chrome logs a warning for the gecko block.
  delete out.browser_specific_settings;
  // Only the Firefox native-messaging transport needs this; the
  // Chromium path talks to DM over WebSocket. Shipping it anyway just
  // enlarges the install prompt.
  out.permissions = (out.permissions || []).filter((p) => p !== "nativeMessaging");
  return out;
}

function main(argv) {
  const [source, output] = argv;
  if (!source || !output) {
    console.error("usage: node scripts/make-chrome-manifest.mjs <source> <output>");
    process.exit(64);
  }
  const parsed = JSON.parse(fs.readFileSync(source, "utf8"));
  fs.writeFileSync(output, JSON.stringify(toChromeManifest(parsed), null, 2) + "\n");
}

if (isMainModule(import.meta.url)) {
  main(process.argv.slice(2));
}
