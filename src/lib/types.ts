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
  proxy: string | null;
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
