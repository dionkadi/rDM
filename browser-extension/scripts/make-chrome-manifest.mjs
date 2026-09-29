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
import path from "node:path";
import { isMainModule } from "./lib/is-main.mjs";

/** The icon sizes the Chrome build ships, as PNG. */
export const CHROME_ICON_SIZES = [16, 32, 48, 128];

const isSvgPath = (p) => typeof p === "string" && /\.svg$/i.test(p);

/**
 * Every file the manifest points at, so the CLI can prove they exist.
 * A manifest referencing a file that was never staged is the
 * next-most-common way to end up with an extension that loads and does
 * nothing.
 *
 * @param {object} manifest
 * @returns {string[]}
 */
export function referencedAssets(manifest) {
  return [
    manifest.background?.service_worker,
    manifest.action?.default_popup,
    ...Object.values(manifest.icons || {}),
    ...Object.values(manifest.action?.default_icon || {}),
  ].filter((p) => typeof p === "string");
}

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

  // **Icons must be raster for Chrome.** "SVG files are not supported
  // for any icons declared in the manifest" (Chrome's manifest docs);
  // Firefox is happy with the SVG, which is why the template points
  // there. Pointing Chrome at it does not fail the load, it just leaves
  // the extension with no icon of its own — a generic letter tile in the
  // toolbar and a "Could not load icon" entry in chrome://extensions'
  // error list, which is exactly the screen a user opens when something
  // is wrong. Rewrite both icon places to the PNG set the build ships.
  const declaredIcons = [
    ...Object.values(source.icons || {}),
    ...Object.values(source.action?.default_icon || {}),
  ];
  if (declaredIcons.some(isSvgPath)) {
    const pngIcons = {};
    for (const size of CHROME_ICON_SIZES) {
      pngIcons[String(size)] = `icons/icon-${size}.png`;
    }
    out.icons = { ...pngIcons };
    if (out.action) {
      out.action = { ...out.action, default_icon: { ...pngIcons } };
    }
  }

  return out;
}

function main(argv) {
  const [source, output] = argv;
  if (!source || !output) {
    console.error("usage: node scripts/make-chrome-manifest.mjs <source> <output>");
    process.exit(64);
  }
  const parsed = JSON.parse(fs.readFileSync(source, "utf8"));
  const chrome = toChromeManifest(parsed);

  // Fail here rather than letting Chrome load an extension whose
  // background never starts or whose icons are missing.
  const dir = path.dirname(path.resolve(source));
  const missing = referencedAssets(chrome).filter(
    (rel) => !fs.existsSync(path.join(dir, rel)),
  );
  if (missing.length > 0) {
    console.error(
      "error: the Chrome manifest references files that are not there: " +
        missing.join(", ") +
        "\n  (Chrome cannot use an .svg icon — the build must ship " +
        CHROME_ICON_SIZES.map((s) => `icons/icon-${s}.png`).join(", ") +
        ")",
    );
    process.exit(1);
  }

  fs.writeFileSync(output, JSON.stringify(chrome, null, 2) + "\n");
}

if (isMainModule(import.meta.url)) {
  main(process.argv.slice(2));
}
