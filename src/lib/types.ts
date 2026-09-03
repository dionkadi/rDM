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
