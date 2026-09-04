// TypeScript mirrors of the Rust engine model (serialised as camelCase JSON).

export type DownloadStatus =
 | "queued"
 | "connecting"
 | "downloading"
 | "paused"
 | "completed"
 | "error"
 | "canceled"
 | "scheduled";

export type ProxyMode = "none" | "system" | "manual";

export interface ChunkState {
 index: number;
 start: number;
 end: number;
 downloaded: number;
}

export interface ChecksumSpec {
 algorithm: string;
 expected: string;
}

export interface Download {
 id: string;
 url: string;
 filename: string;
 savePath: string;
 totalSize: number | null;
 downloaded: number;
 status: DownloadStatus;
 category: string | null;
 contentType: string | null;
 chunks: ChunkState[];
 speedLimit: number | null;
 proxy: string | null;
 checksum: ChecksumSpec | null;
 error: string | null;
 createdAt: string;
 finishedAt: string | null;
 canResume: boolean;
 /** User-controlled queue position. Smaller values run first. */
 sortKey: number;
 /** 0 = low, 1 = normal (default), 2 = high. */
 priority: number;
}

export interface Category {
 id: string;
 name: string;
 extensions: string[];
 directory: string;
}

export interface Settings {
 maxConcurrentDownloads: number;
 connectionsPerDownload: number;
 defaultDirectory: string;
 speedLimitGlobal: number | null;
 categories: Category[];
 /** Manual proxy URL (used when `proxyMode === "manual"`). */
 proxy: string | null;
 /** Proxy policy. `undefined` is tolerated for backward compat (old rows). */
 proxyMode?: ProxyMode;
 clipboardMonitor: boolean;
 closeToTray: boolean;
 scheduleEnabled: boolean;
 scheduleStart: [number, number];
 scheduleEnd: [number, number];
}

// Payload for the `captured` variant of `FrontendEvent`. Sent
// from the Tauri side when the browser extension's native
// host forwards a URL to DM. The frontend shows an IDM-style
// confirmation dialog before the URL is actually queued.
export interface CapturedUrl {
 /** Where the capture came from. One of `"browser-click"`,
  *  `"browser-save-as"`, `"browser-grab"`, `"native-host"`,
  *  `"unknown"`. Used to label the dialog title. */
 source: string;
 /** The raw URL the browser handed us. */
 url: string;
 /** Filename extracted from the URL path / Content-Disposition
  *  on the Rust side. The user can edit this in the dialog. */
 suggestedFilename: string;
 /** The user's default save directory at the time of capture.
  *  The dialog shows this as the default; the user can pick a
  *  category and the resolved save path is what `add_download`
  *  will actually use. */
 defaultSaveDir: string;
 /** `Referer` header value the browser captured for the
  *  *source* page (i.e. the page that contained the link
  *  the user clicked). This is the high-leverage auth hint
  *  for the Tier-1 "Referer / user-agent per download" item:
  *  the frontend pre-fills the per-download Referer row
  *  with this value, and the user can confirm or edit
  *  before clicking "Download". `null` when the browser
  *  didn't send one (e.g. a copy-paste capture with no
  *  originating page). */
 referer: string | null;
 /** User-Agent the browser was using at the time of
  *  capture. Some servers gate downloads on UA fingerprint
  *  (e.g. mobile-only mirrors) so pre-filling this in the
  *  per-download headers is the path of least surprise.
  *  `null` when the native host didn't relay one. */
 userAgent: string | null;
 /** Monotonic ID for dedupe. If the same URL comes in twice in
  *  quick succession (e.g. double-click) the frontend can drop
  *  the second one. */
 nonce: string;
}

// Discriminated union emitted on the "download-event" channel.
export type FrontendEvent =
 | { kind: "added"; download: Download }
 | { kind: "progress"; download: Download }
 | { kind: "statusChanged"; download: Download }
 | { kind: "completed"; download: Download }
 | { kind: "error"; download: Download }
 | { kind: "removed"; download: { id: string } }
 | { kind: "captured"; download: CapturedUrl };

/** Human-readable label for a proxy mode. */
export const PROXY_MODE_LABEL: Record<ProxyMode, string> = {
 none: "No proxy",
 system: "System proxy",
 manual: "Manual",
};

/** Short description for a proxy mode, used in tooltips and the picker. */
export const PROXY_MODE_DESC: Record<ProxyMode, string> = {
 none: "Always connect directly. Ignore the system environment.",
 system: "Use the HTTP_PROXY / HTTPS_PROXY / NO_PROXY environment variables.",
 manual: "Use a specific proxy URL. Supports http://, https://, socks5://.",
};

// ── Browser cookies (per-download auth) ────────────────────────
//
// Returned by the `import_browser_cookies` Tauri command. The
// frontend shows a checklist of these so the user can pick
// which cookies to send with their next download. The
// `header` field is the pre-formatted `Cookie: a=1; b=2; …`
// value (ready to paste into the per-download headers
// dialog) and is what we actually send to the engine.

export type BrowserKind = "firefox" | "chromium";

export interface BrowserCookie {
 name: string;
 value: string;
 host: string;
 path: string;
 secure: boolean;
 /** Unix seconds; `null` = session cookie. */
 expiresUnix: number | null;
}

export interface CookieImportResult {
 count: number;
 /** Pre-formatted `Cookie:` header value (without the
  *  leading `Cookie:` token). The user can apply this as-is
  *  via the per-download headers dialog. */
 header: string;
 cookies: BrowserCookie[];
}
