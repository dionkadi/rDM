// Downloads store — reactive state for the download list
import { writable, derived, get } from "svelte/store";
import type { CapturedUrl, Download, FrontendEvent } from "../types";
import * as api from "../api";

// ── Capture queue (IDM-style) ───────────────────────────────────
//
// When the browser extension forwards a URL, the Tauri side emits a
// `Captured` event on the `download-event` channel. We push it into
// `captureQueue`; the `CaptureDialog` component renders the most
// recent capture and lets the user confirm category / path /
// filename before the URL is queued. The queue is a small array
// (usually 0–2 items; rare to have more than one pending at a time)
// so we keep it simple — no separate "pending approvals" store.

export const captureQueue = writable<CapturedUrl[]>([]);
const seenCaptureNonces = new Set<string>();

export function enqueueCapture(c: CapturedUrl): void {
  // Dedupe: the same URL can arrive twice in quick succession
  // (e.g. the user double-clicks a link, the extension fires two
  // onCreated events, or two tabs both surface the same media).
  // The Rust side stamps each event with a monotonic nonce; if
  // we've already seen it, drop the duplicate.
  if (c.nonce && seenCaptureNonces.has(c.nonce)) return;
  if (c.nonce) seenCaptureNonces.add(c.nonce);
  // Cap the queue to 10 entries — the dialog shows the head, and
  // anything beyond 10 is almost certainly a misbehaving extension
  // flooding the channel. Old entries are dropped silently.
  captureQueue.update((q) => {
    const next = [...q, c];
    if (next.length > 10) next.shift();
    return next;
  });
}

export function popCapture(): void {
  captureQueue.update((q) => q.slice(1));
}

export function clearCaptures(): void {
  captureQueue.set([]);
}

// ── Core state ──────────────────────────────────────────────────────
export const downloads = writable<Download[]>([]);
export const speedMap = writable<Record<string, number>>({});
export const aggregateSpeed = writable(0);
export const isLoading = writable(false);
export const error = writable("");

// ── Live speed tracking ────────────────────────────────────────────
const prevMap: Record<string, number> = {};
let monitorTimer: ReturnType<typeof setInterval> | null = null;

export function startSpeedMonitor(): void {
  stopSpeedMonitor();
  monitorTimer = setInterval(() => {
    const next: Record<string, number> = {};
    let agg = 0;
    const list = get(downloads);
    for (const d of list) {
      if (d.status === "downloading" || d.status === "connecting") {
        const prev = prevMap[d.id] ?? d.downloaded;
        const s = Math.max(0, d.downloaded - prev);
        next[d.id] = s;
        agg += s;
      }
    }
    // Update prev map for next tick
    for (const d of list) prevMap[d.id] = d.downloaded;
    speedMap.set(next);
    aggregateSpeed.set(agg);
  }, 1000);
}

export function stopSpeedMonitor(): void {
  if (monitorTimer) clearInterval(monitorTimer);
  monitorTimer = null;
}

// ── CRUD operations ────────────────────────────────────────────────
export async function refreshDownloads(): Promise<void> {
  isLoading.set(true);
  try {
    const list = await api.listDownloads();
    downloads.set(list);
    error.set("");
  } catch (e) {
    error.set(String(e));
  } finally {
    isLoading.set(false);
  }
}

export async function addDownload(opts: {
  url: string;
  category?: string | null;
  filename?: string | null;
  speedLimit?: number | null;
  checksum?: { algorithm: string; expected: string } | null;
}): Promise<void> {
  try {
    await api.addDownload(opts);
    await refreshDownloads();
  } catch (e) {
    error.set(String(e));
    throw e;
  }
}

export async function pauseDownload(id: string): Promise<void> {
  await api.pauseDownload(id);
  await refreshDownloads();
}

export async function resumeDownload(id: string): Promise<void> {
  await api.resumeDownload(id);
  await refreshDownloads();
}

export async function cancelDownload(id: string): Promise<void> {
  await api.cancelDownload(id);
  await refreshDownloads();
}

export async function removeDownload(id: string): Promise<void> {
  await api.removeDownload(id);
  await refreshDownloads();
}

export async function setDownloadSpeedLimit(
  id: string,
  limit: number | null,
): Promise<void> {
  await api.setSpeedLimit(id, limit);
  await refreshDownloads();
}

// ── Bulk operations ──────────────────────────────────────────────
//
// `Promise.allSettled` rather than `Promise.all`: a single bad id
// (e.g. a row that was removed in the same tick) shouldn't abort
// the whole batch and surface as a hard error. Instead we collect
// failures and report them via a single toast. One `refreshDownloads`
// at the end is enough — the engine's `StatusChanged` events will
// also have fired for each row, but the store-level merge handles
// dedupe.

/** Apply an action to a list of ids in parallel; return the
 *  number of successful applications and the first error message
 *  (if any). The caller decides whether the error warrants a toast. */
async function bulkApply(
  ids: readonly string[],
  action: (id: string) => Promise<void>,
): Promise<{ ok: number; failed: number; firstError: string | null }> {
  const results = await Promise.allSettled(ids.map((id) => action(id)));
  let ok = 0;
  let failed = 0;
  let firstError: string | null = null;
  for (const r of results) {
    if (r.status === "fulfilled") ok++;
    else {
      failed++;
      if (!firstError) firstError = String(r.reason);
    }
  }
  return { ok, failed, firstError };
}

export async function bulkPause(ids: readonly string[]): Promise<void> {
  const r = await bulkApply(ids, pauseDownload);
  await refreshDownloads();
  if (r.failed > 0) throw new Error(`${r.ok} paused, ${r.failed} failed: ${r.firstError}`);
}

export async function bulkResume(ids: readonly string[]): Promise<void> {
  const r = await bulkApply(ids, resumeDownload);
  await refreshDownloads();
  if (r.failed > 0) throw new Error(`${r.ok} resumed, ${r.failed} failed: ${r.firstError}`);
}

export async function bulkRemove(ids: readonly string[]): Promise<void> {
  const r = await bulkApply(ids, removeDownload);
  await refreshDownloads();
  if (r.failed > 0) throw new Error(`${r.ok} removed, ${r.failed} failed: ${r.firstError}`);
}

export async function bulkSetLimit(
  ids: readonly string[],
  limit: number | null,
): Promise<void> {
  const r = await bulkApply(ids, (id) => setDownloadSpeedLimit(id, limit));
  await refreshDownloads();
  if (r.failed > 0) throw new Error(`${r.ok} updated, ${r.failed} failed: ${r.firstError}`);
}

// ── Reorder / priority ──────────────────────────────────────
//
// `reorder` calls the Rust `reorder_downloads` command, which
// assigns fresh `sort_key` values and emits per-row
// `StatusChanged` events. The events carry the new `sort_key`
// and the frontend store's merge handler picks them up — we
// don't need to manually re-sort or call `refreshDownloads`
// because the events are the source of truth.

export async function reorderDownloads(ids: string[]): Promise<void> {
  await api.reorderDownloads(ids);
}

export async function setDownloadPriority(
  id: string,
  priority: number,
): Promise<void> {
  await api.setDownloadPriority(id, priority);
}

// ── Event handling (Tauri live updates) ────────────────────────────
let unlistenFn: (() => void) | null = null;

export async function startEventListener(): Promise<void> {
  try {
    const { listen } = await import("@tauri-apps/api/event");
    unlistenFn = await listen<FrontendEvent>("download-event", (ev) => {
      const e = ev.payload;
      if (e.kind === "captured") {
        // Browser-extension capture: push into the capture queue
        // so the `CaptureDialog` component can show the
        // confirmation prompt. The dialog calls `addDownload()`
        // when the user clicks "Download", which is what actually
        // queues the task in the engine.
        enqueueCapture(e.download);
        return;
      }
      if (e.kind === "removed") {
        const removedId = e.download.id;
        downloads.update((list) => list.filter((d) => d.id !== removedId));
        // Lazy-import to avoid a circular dependency: `ui.ts` is
        // also imported by the download store, so reaching back
        // into it from a top-level `import` would be a cycle.
        import("./ui").then((ui) => ui.pruneSelection(new Set(get(downloads).map((d) => d.id))));
        return;
      }
      const d = e.download;
      downloads.update((list) => {
        const i = list.findIndex((x) => x.id === d.id);
        if (i >= 0) {
          const next = list.slice();
          next[i] = d;
          return next;
        }
        return [d, ...list];
      });
    });
  } catch {
    // Tauri not available (preview mode)
  }
}

export function stopEventListener(): void {
  if (unlistenFn) unlistenFn();
  unlistenFn = null;
}

// ── Derived stores ──────────────────────────────────────────────────
export const counts = derived(downloads, ($downloads) => ({
  all: $downloads.length,
  active: $downloads.filter(
    (d) => !["completed", "error", "canceled"].includes(d.status),
  ).length,
  queued: $downloads.filter(
    (d) => d.status === "queued" || d.status === "scheduled",
  ).length,
  paused: $downloads.filter((d) => d.status === "paused").length,
  completed: $downloads.filter((d) => d.status === "completed").length,
  errored: $downloads.filter((d) => d.status === "error").length,
  canceled: $downloads.filter((d) => d.status === "canceled").length,
  scheduled: $downloads.filter((d) => d.status === "scheduled").length,
}));

export const totalDownloaded = derived(downloads, ($downloads) =>
  $downloads.reduce((s, d) => s + (d.downloaded || 0), 0),
);

export const hasActive = derived(downloads, ($downloads) =>
  $downloads.some(
    (d) => d.status === "downloading" || d.status === "connecting",
  ),
);

// ── Smart folders (file type groupings) ────────────────────────────
export const smartFolders = derived(downloads, ($downloads) => {
  const buckets: Record<
    string,
    { count: number; downloaded: number; total: number }
  > = {
    video: { count: 0, downloaded: 0, total: 0 },
    audio: { count: 0, downloaded: 0, total: 0 },
    archive: { count: 0, downloaded: 0, total: 0 },
    document: { count: 0, downloaded: 0, total: 0 },
    image: { count: 0, downloaded: 0, total: 0 },
    binary: { count: 0, downloaded: 0, total: 0 },
    other: { count: 0, downloaded: 0, total: 0 },
  };

  const extMap: Record<string, string> = {
    mp4: "video",
    mkv: "video",
    webm: "video",
    mov: "video",
    avi: "video",
    flv: "video",
    m3u8: "video",
    mp3: "audio",
    wav: "audio",
    flac: "audio",
    ogg: "audio",
    aac: "audio",
    m4a: "audio",
    zip: "archive",
    rar: "archive",
    "7z": "archive",
    gz: "archive",
    tar: "archive",
    tgz: "archive",
    bz2: "archive",
    pdf: "document",
    doc: "document",
    docx: "document",
    xls: "document",
    xlsx: "document",
    ppt: "document",
    pptx: "document",
    txt: "document",
    md: "document",
    rtf: "document",
    jpg: "image",
    jpeg: "image",
    png: "image",
    gif: "image",
    webp: "image",
    svg: "image",
    bmp: "image",
    exe: "binary",
    msi: "binary",
    appimage: "binary",
    deb: "binary",
    rpm: "binary",
    apk: "binary",
    iso: "binary",
    img: "binary",
    dmg: "binary",
  };

  for (const d of $downloads) {
    const ext = (d.filename || d.url).split(".").pop()?.toLowerCase() || "";
    const bucket = extMap[ext] || "other";
    buckets[bucket].count++;
    buckets[bucket].downloaded += d.downloaded || 0;
    buckets[bucket].total += d.totalSize || 0;
  }

  return buckets;
});
