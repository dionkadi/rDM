// Thin wrappers around the Rust Tauri commands. Argument names are camelCase
// (Tauri v2 converts them to the Rust snake_case parameter names); command
// names are used verbatim.

import { invoke } from "@tauri-apps/api/core";
import type {
 BrowserKind,
 CookieImportResult,
 Download,
 ProxyMode,
 Settings,
} from "./types";

export function ping(): Promise<string> {
 return invoke("ping");
}

export function addDownload(opts: {
 url: string;
 category?: string | null;
 filename?: string | null;
 speedLimit?: number | null;
 checksum?: { algorithm: string; expected: string } | null;
 proxy?: string | null;
 /**
  * Per-download HTTP headers to attach (e.g. Referer,
  * User-Agent, Cookie). In-memory only — a restart clears
  * them. The engine filters out restricted keys (Host,
  * Content-Length, Accept-Encoding) and warns on the rest.
  */
 headers?: Record<string, string> | null;
 /**
  * Per-download auth. In-memory only. The frontend's
  * AuthDialog is the canonical way to set this; the
  * CaptureDialog only sets it for browser-captured URLs
  * when the user has confirmed cookies / Referer.
  */
 auth?: HeadersAuth | null;
}): Promise<Download> {
 return invoke("add_download", {
  url: opts.url,
  category: opts.category ?? null,
  filename: opts.filename ?? null,
  speedLimit: opts.speedLimit ?? null,
  checksum: opts.checksum ?? null,
  proxy: opts.proxy ?? null,
  headers: opts.headers ?? null,
  auth: opts.auth ?? null,
 });
}

export function listDownloads(): Promise<Download[]> {
 return invoke("list_downloads");
}

export function getDownload(id: string): Promise<Download | null> {
 return invoke("get_download", { id });
}

export function pauseDownload(id: string): Promise<void> {
 return invoke("pause_download", { id });
}

export function resumeDownload(id: string): Promise<void> {
 return invoke("resume_download", { id });
}

export function cancelDownload(id: string): Promise<void> {
 return invoke("cancel_download", { id });
}

export function removeDownload(id: string): Promise<void> {
 return invoke("remove_download", { id });
}

export function setSpeedLimit(id: string, limit: number | null): Promise<void> {
 return invoke("set_speed_limit", { id, limit });
}

export function setGlobalSpeedLimit(limit: number | null): Promise<void> {
 return invoke("set_global_speed_limit", { limit });
}

/**
 * Persist a new queue order for the given download ids. The
 * engine assigns fresh `sort_key` values spaced by 1000 so a
 * future interleaved reorder still has integer room to land.
 * Per-row `StatusChanged` events are emitted; the frontend
 * store merges them without a full refresh.
 */
export function reorderDownloads(ids: string[]): Promise<void> {
 return invoke("reorder_downloads", { ids });
}

/**
 * Set the per-download priority. `0` = low, `1` = normal
 * (default), `2` = high. Out-of-range values are clamped by
 * the engine to `[0, 2]`.
 */
export function setDownloadPriority(
 id: string,
 priority: number,
): Promise<void> {
 return invoke("set_download_priority", { id, priority });
}

export function getSettings(): Promise<Settings> {
 return invoke("get_settings");
}

export function updateSettings(settings: Settings): Promise<void> {
 return invoke("update_settings", { settings });
}

/** Switch the global proxy policy. `url` is only used when `mode === "manual"`. */
export function setProxy(
 mode: ProxyMode,
 url?: string | null,
): Promise<Settings> {
 return invoke("set_proxy", { mode, url: url ?? null });
}

export function saveDirFor(category?: string | null): Promise<string> {
 return invoke("save_dir_for", { category: category ?? null });
}

export interface NativeHostProbe {
 bound: boolean;
 port: number;
 lastEventUnix: number;
}

export function probeNativeHost(): Promise<NativeHostProbe> {
 return invoke("probe_native_host");
}

/**
 * Open `path` in the OS file manager.
 *
 * - If `path` is a directory, it is opened directly.
 * - If `path` is an existing file and `selectFile` is `true`, the OS
 *   is asked to highlight it (Finder reveal / Explorer /select,
 *   Nautilus best-effort).
 * - Otherwise, the file's parent directory is opened (the typical
 *   "Open folder" action for a download whose file may not exist
 *   yet).
 *
 * Returns the underlying opener error as a string so the caller can
 * show it in a toast.
 */
export function openFolder(
 path: string,
 selectFile: boolean = false,
): Promise<void> {
 return invoke("open_folder", { path, selectFile });
}

/**
 * Open `path` (a file, not a directory) in the OS's default
 * handler. Distinct from `openFolder` — this launches the
 * associated application (the .pdf reader, the .mp4 player, the
 * default text editor, etc.) rather than the file manager.
 * Errors (e.g. the file is still being downloaded and does not
 * exist yet) are returned as a string for the caller to toast.
 */
export function openFile(path: string): Promise<void> {
 return invoke("open_file", { path });
}

/**
 * Copy `text` to the OS clipboard. Routed through the Tauri
 * command surface (rather than `navigator.clipboard.writeText`)
 * so it works in the Tauri webview where some configurations
 * refuse clipboard writes outside a user gesture.
 */
export function copyText(text: string): Promise<void> {
 return invoke("copy_text", { text });
}

/**
 * Move a download's on-disk file (and its `.part` sibling) to
 * the OS trash, then drop the in-memory entry and the SQLite
 * row. This is the recoverable counterpart to `removeDownload`:
 * the user can undelete via Finder / Explorer / Files if they
 * hit it by accident. The SQLite row is the irreversible half.
 */
export function trashDownload(id: string): Promise<void> {
 return invoke("trash_download", { id });
}

/**
 * Set per-download HTTP headers + auth. Routes through the
 * existing `set_download_auth` Tauri command. Headers and auth
 * are in-memory only — they are NOT persisted to SQLite, so a
 * restart clears them. The Rust side filters restricted headers
 * (`Host`, `Content-Length`, `Accept-Encoding`) and surfaces
 * any rejected entries as warnings in the log file.
 */
export type HeadersAuth =
 | { kind: "none" }
 | { kind: "basic"; username: string; password: string }
 | { kind: "bearer"; token: string }
 | { kind: "digest"; username: string; password: string };

export function setDownloadAuth(
 id: string,
 headers: Record<string, string>,
 auth: HeadersAuth,
): Promise<void> {
 return invoke("set_download_auth", { id, headers, auth });
}

/**
 * Read cookies from a browser's local SQLite database. Used by
 * the per-download auth dialog to populate a `Cookie:` header
 * from the user's existing browser session.
 *
 * `kind` is `"firefox"` or `"chromium"`. The engine picks the
 * default on-disk path for the current OS; `pathOverride` lets
 * the user pick a custom file (e.g. a snap install or a
 * portable browser). `host` filters by suffix match — pass the
 * URL's host (e.g. `"example.com"`) to get only the cookies
 * that apply. Pass an empty string for the full list.
 *
 * Errors are returned as a rejected promise so the frontend can
 * show the exact reason in a toast (e.g. "Chromium cookies on
 * macOS are encrypted — see the docs").
 */
export function importBrowserCookies(
 kind: BrowserKind,
 host?: string,
 pathOverride?: string,
): Promise<CookieImportResult> {
 return invoke("import_browser_cookies", {
  kind,
  host: host ?? "",
  pathOverride: pathOverride ?? null,
 });
}
