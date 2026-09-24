// Symlink-safe "am I the entry module?" check.
//
// The usual idiom is:
//
//     if (fileURLToPath(import.meta.url) === process.argv[1]) main();
//
// and it is silently wrong under symlinks. Node resolves the entry
// module to its *real* path for `import.meta.url`, but leaves
// `process.argv[1]` exactly as it was given. So:
//
//   node scripts/tool.mjs                       -> equal, main() runs
//   node /home/me/link-to-repo/scripts/tool.mjs  -> NOT equal, main() never
//                                                   runs, exit code 0, no
//                                                   output, no error
//
// A silent no-op is the worst possible failure for a build or diagnostic
// script: `set -e` sees success and carries on with stale output. This
// repo lives behind a symlink (`/home/tomg/Codes/…` -> `/home/tomg/Other/…`)
// and `pwd` returns the logical path, so shell scripts hand node exactly
// the argv[1] shape that trips it.
//
// Compare realpaths instead.

import fs from "node:fs";
import { fileURLToPath } from "node:url";

/**
 * True when the module identified by `metaUrl` is the process entry point.
 * Pass the *caller's* `import.meta.url` — this file's own URL would be
 * compared against a path that never equals it.
 *
 * Never throws — a missing or unreadable path just means "not main".
 *
 * @param {string} metaUrl the caller's `import.meta.url`
 */
export function isMainModule(metaUrl) {
  const entry = process.argv[1];
  if (!entry) return false;
  try {
    return fs.realpathSync(entry) === fs.realpathSync(fileURLToPath(metaUrl));
  } catch (_e) {
    return false;
  }
}
