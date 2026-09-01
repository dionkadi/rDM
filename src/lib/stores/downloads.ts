// Downloads store — reactive state for the download list
import { writable, derived, get } from "svelte/store";
import type { Download, FrontendEvent } from "../types";
import * as api from "../api";

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

export async function setDownloadSpeedLimit(id: string, limit: number | null): Promise<void> {
  await api.setSpeedLimit(id, limit);
  await refreshDownloads();
}

// ── Event handling (Tauri live updates) ────────────────────────────
let unlistenFn: (() => void) | null = null;

export async function startEventListener(): Promise<void> {
  try {
    const { listen } = await import("@tauri-apps/api/event");
    unlistenFn = await listen<FrontendEvent>("download-event", (ev) => {
      const e = ev.payload;
      if (e.kind === "removed") {
        downloads.update((list) => list.filter((d) => d.id !== e.download.id));
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
  active: $downloads.filter((d) => !["completed", "error", "canceled"].includes(d.status)).length,
  queued: $downloads.filter((d) => d.status === "queued" || d.status === "scheduled").length,
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
  $downloads.some((d) => d.status === "downloading" || d.status === "connecting"),
);

// ── Smart folders (file type groupings) ────────────────────────────
export const smartFolders = derived(downloads, ($downloads) => {
  const buckets: Record<string, { count: number; downloaded: number; total: number }> = {
    video: { count: 0, downloaded: 0, total: 0 },
    audio: { count: 0, downloaded: 0, total: 0 },
    archive: { count: 0, downloaded: 0, total: 0 },
    document: { count: 0, downloaded: 0, total: 0 },
    image: { count: 0, downloaded: 0, total: 0 },
    binary: { count: 0, downloaded: 0, total: 0 },
    other: { count: 0, downloaded: 0, total: 0 },
  };

  const extMap: Record<string, string> = {
    mp4: "video", mkv: "video", webm: "video", mov: "video", avi: "video", flv: "video", "m3u8": "video",
    mp3: "audio", wav: "audio", flac: "audio", ogg: "audio", aac: "audio", m4a: "audio",
    zip: "archive", rar: "archive", "7z": "archive", gz: "archive", tar: "archive", tgz: "archive", bz2: "archive",
    pdf: "document", doc: "document", docx: "document", xls: "document", xlsx: "document",
    ppt: "document", pptx: "document", txt: "document", md: "document", rtf: "document",
    jpg: "image", jpeg: "image", png: "image", gif: "image", webp: "image", svg: "image", bmp: "image",
    exe: "binary", msi: "binary", appimage: "binary", deb: "binary", rpm: "binary", apk: "binary",
    iso: "binary", img: "binary", dmg: "binary",
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
