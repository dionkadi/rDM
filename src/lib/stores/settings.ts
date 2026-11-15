// Settings store — reactive settings with persistence
import { writable, get } from "svelte/store";
import type { ProxyMode, Settings } from "../types";
import * as api from "../api";
import { showToast, globalSpeedLimitLabel } from "./ui";

export const settings = writable<Settings | null>(null);

/** Human label for a bytes/sec cap; mirrors the presets in the UI. */
export function speedLimitLabel(bytes: number | null | undefined): string {
  if (bytes == null || bytes <= 0) return "Unlimited";
  const mb = bytes / (1024 * 1024);
  return `${Number(mb.toFixed(mb < 10 ? 1 : 0))} MB/s`;
}

export async function loadSettings(): Promise<void> {
  try {
    const s = await api.getSettings();
    settings.set(s);
    // The status bar used to say "Unlimited" after a restart even when
    // a cap was persisted, because nothing rehydrated its label.
    globalSpeedLimitLabel.set(speedLimitLabel(s.speedLimitGlobal));
  } catch {
    settings.set(null);
  }
}

export async function updateSettings(patch: Partial<Settings>): Promise<void> {
  const current = get(settings) ?? (await api.getSettings());
  const next = { ...current, ...patch };
  settings.set(next);
  if (patch.speedLimitGlobal !== undefined) {
    globalSpeedLimitLabel.set(speedLimitLabel(next.speedLimitGlobal));
  }
  try {
    await api.updateSettings(next);
  } catch (e) {
    // Revert on error — including the status-bar label, which was set
    // optimistically above. Without this the engine keeps the old cap
    // while the status bar keeps claiming the new one until restart.
    settings.set(current);
    if (patch.speedLimitGlobal !== undefined) {
      globalSpeedLimitLabel.set(speedLimitLabel(current.speedLimitGlobal));
    }
    throw e;
  }
}

/**
 * Switch the global proxy policy. The dedicated Tauri command is used so the
 * engine can rebuild its `RunContext` immediately (cheap path) rather than
 * re-serialising the full `Settings` struct.
 */
export async function setProxyMode(
  mode: ProxyMode,
  url?: string | null,
): Promise<void> {
  const current = get(settings);
  // Optimistic update
  if (current) {
    settings.set({ ...current, proxyMode: mode, proxy: url ?? null });
  }
  try {
    const next = await api.setProxy(mode, url ?? null);
    settings.set(next);
    showToast({
      kind: "success",
      title: proxyModeLabel(mode),
      message: mode === "manual" && url ? url : undefined,
      duration: 1800,
    });
  } catch (e) {
    if (current) settings.set(current);
    showToast({ kind: "error", title: "Proxy change failed", message: String(e) });
    throw e;
  }
}

function proxyModeLabel(mode: ProxyMode): string {
  switch (mode) {
    case "none":
      return "Proxy disabled";
    case "system":
      return "Using system proxy";
    case "manual":
      return "Manual proxy saved";
  }
}
