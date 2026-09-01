// UI store — sidebar state, search, toasts, command palette, modals
import { writable, derived } from "svelte/store";
import { browser } from "../utils/env";

// ── Theme ───────────────────────────────────────────────────────────
export type ThemeKey = "dark" | "light" | "system";
const THEME_KEY = "dm-theme";
const mediaQuery = typeof window !== "undefined"
  ? window.matchMedia("(prefers-color-scheme: light)")
  : null;

function readStoredTheme(): ThemeKey {
  if (typeof localStorage === "undefined") return "dark";
  const v = localStorage.getItem(THEME_KEY);
  return v === "light" || v === "dark" || v === "system" ? v : "dark";
}

function effectiveTheme(stored: ThemeKey): "dark" | "light" {
  if (stored === "system") {
    return mediaQuery?.matches ? "light" : "dark";
  }
  return stored;
}

function applyTheme(theme: "dark" | "light"): void {
  if (typeof document === "undefined") return;
  document.documentElement.setAttribute("data-theme", theme);
  // Also nudge native form controls
  const meta = document.querySelector<HTMLMetaElement>('meta[name="color-scheme"]');
  if (meta) meta.content = theme;
}

export const theme = writable<ThemeKey>(readStoredTheme());

// React to theme changes
if (browser) {
  theme.subscribe((stored) => {
    if (typeof localStorage !== "undefined") localStorage.setItem(THEME_KEY, stored);
    const eff = effectiveTheme(stored);
    applyTheme(eff);
  });
  if (mediaQuery && typeof mediaQuery.addEventListener === "function") {
    mediaQuery.addEventListener("change", () => {
      const current = readStoredTheme();
      if (current === "system") applyTheme(effectiveTheme("system"));
    });
  }
  // Apply initial value
  applyTheme(effectiveTheme(readStoredTheme()));
}

export const effectiveThemeStore = derived(theme, ($t) => effectiveTheme($t));

export function setTheme(next: ThemeKey): void {
  theme.set(next);
}

// ── Sidebar ─────────────────────────────────────────────────────────
export const sidebarCollapsed = writable(false);
export const sidebarWidth = derived(sidebarCollapsed, ($c) => ($c ? 64 : 248));

// ── Filters / view ──────────────────────────────────────────────────
export type FilterKey =
  | "all"
  | "active"
  | "queued"
  | "paused"
  | "completed"
  | "error"
  | "scheduled"
  | "video"
  | "audio"
  | "archive"
  | "document"
  | "image"
  | "binary";

export const activeFilter = writable<FilterKey>("all");
export const searchQuery = writable("");
export const sortBy = writable<"recent" | "name" | "size" | "speed">("recent");
export const viewMode = writable<"comfortable" | "compact">("comfortable");

// ── Modals ──────────────────────────────────────────────────────────
export const showSettings = writable(false);
export const showAddDialog = writable(false);
export const showCommandPalette = writable(false);
export const showOnboarding = writable(false);

// ── Toasts ──────────────────────────────────────────────────────────
export interface Toast {
  id: string;
  kind: "success" | "error" | "info" | "warning";
  title: string;
  message?: string;
  duration?: number;
  action?: { label: string; handler: () => void };
}

export const toasts = writable<Toast[]>([]);

let toastId = 0;
export function showToast(toast: Omit<Toast, "id">): string {
  const id = `toast-${++toastId}`;
  const t: Toast = { id, duration: 4000, ...toast };
  toasts.update((list) => [...list, t]);
  if (t.duration && t.duration > 0) {
    setTimeout(() => dismissToast(id), t.duration);
  }
  return id;
}

export function dismissToast(id: string): void {
  toasts.update((list) => list.filter((t) => t.id !== id));
}

// ── Clipboard monitoring ───────────────────────────────────────────
export const clipboardUrl = writable("");
export const clipboardMonitorEnabled = writable(false);

// ── Drag & drop ────────────────────────────────────────────────────
export const dragActive = writable(false);

// ── Status bar metrics ────────────────────────────────────────────
export const globalSpeedLimit = writable<number | null>(null);
export const globalSpeedLimitLabel = writable<string>("Unlimited");

// ── Demo mode ─────────────────────────────────────────────────────
export const demoMode = writable(false);

// ── Browser extension / native host status ────────────────────────
export interface ExtensionStatus {
  /** Whether the native-host TCP listener is bound. */
  kind: "ok" | "starting" | "offline" | "error";
  label: string;
  port: number;
  /** Human-readable "last activity" line. */
  lastActivity: string;
  /** Set when `kind === "error"`. */
  lastError: string | null;
}

export const extensionStatus = writable<ExtensionStatus>({
  kind: "starting",
  label: "Checking…",
  port: 9157,
  lastActivity: "—",
  lastError: null,
});

/** Mark a successful capture (URL pushed by the native host). */
export function recordNativeHostEvent(): void {
  extensionStatus.update((s) => ({
    ...s,
    kind: "ok",
    label: "Ready",
    lastActivity: new Date().toLocaleTimeString(),
    lastError: null,
  }));
}

/** Mark the listener bind failed (e.g. port in use). */
export function recordNativeHostError(msg: string): void {
  extensionStatus.update((s) => ({
    ...s,
    kind: "error",
    label: "Error",
    lastActivity: new Date().toLocaleTimeString(),
    lastError: msg,
  }));
}

