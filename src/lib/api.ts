// Thin wrappers around the Rust Tauri commands. Argument names are camelCase
// (Tauri v2 converts them to the Rust snake_case parameter names); command
// names are used verbatim.

import { invoke } from "@tauri-apps/api/core";
import type { Download, Settings } from "./types";

export function ping(): Promise<string> {
  return invoke("ping");
}

export function addDownload(opts: {
  url: string;
  category?: string | null;
  filename?: string | null;
  speedLimit?: number | null;
  checksum?: { algorithm: string; expected: string } | null;
}): Promise<Download> {
  return invoke("add_download", {
    url: opts.url,
    category: opts.category ?? null,
    filename: opts.filename ?? null,
    speedLimit: opts.speedLimit ?? null,
    checksum: opts.checksum ?? null,
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

export function getSettings(): Promise<Settings> {
  return invoke("get_settings");
}

export function updateSettings(settings: Settings): Promise<void> {
  return invoke("update_settings", { settings });
}

export function saveDirFor(category?: string | null): Promise<string> {
  return invoke("save_dir_for", { category: category ?? null });
}
