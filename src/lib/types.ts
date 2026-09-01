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

// Discriminated union emitted on the "download-event" channel.
export type FrontendEvent =
  | { kind: "added"; download: Download }
  | { kind: "progress"; download: Download }
  | { kind: "statusChanged"; download: Download }
  | { kind: "completed"; download: Download }
  | { kind: "error"; download: Download }
  | { kind: "removed"; download: { id: string } };

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
