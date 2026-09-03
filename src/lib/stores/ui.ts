// UI store — sidebar state, search, toasts, command palette, modals
import { writable, derived } from "svelte/store";
import { browser } from "../utils/env";

// ── Theme ───────────────────────────────────────────────────────────
export type ThemeKey = "dark" | "light" | "system";
const THEME_KEY = "dm-theme";
const mediaQuery =
  typeof window === "undefined"
    ? null
    : window.matchMedia("(prefers-color-scheme: light)");

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
  const meta = document.querySelector<HTMLMetaElement>(
    'meta[name="color-scheme"]',
  );
  if (meta) meta.content = theme;
}

export const theme = writable<ThemeKey>(readStoredTheme());

// React to theme changes
if (browser) {
  theme.subscribe((stored) => {
    if (typeof localStorage !== "undefined")
      localStorage.setItem(THEME_KEY, stored);
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

// ── Selection (bulk operations) ────────────────────────────────
//
// `selectedIds` is a Svelte store holding the *visible* selection —
// the ids the user has explicitly picked in the current view. It is
// intentionally a `Set` so toggle/add/remove are O(1) and so the
// "select all" / "clear" operations don't iterate to dedupe.
//
// The set is *ephemeral* (not persisted to localStorage): a
// selection across an app restart would be a UX bug, not a feature
// (the user expects to start fresh).
export const selectedIds = writable<Set<string>>(new Set());
/** The id that was most recently clicked. Used as the "anchor" of
 *  the next shift-click range. Cleared when the selection is cleared. */
export const lastSelectedId = writable<string | null>(null);

/** True when the user has at least one row selected. Derived so
 *  components can `class:active={$hasSelection}` without re-deriving
 *  from a Set in every template. */
export const hasSelection = derived(selectedIds, ($s) => $s.size > 0);

/** Add a single id to the selection (no-op if already in the set). */
export function selectOne(id: string): void {
  selectedIds.update((s) => {
    if (s.has(id)) return s;
    const next = new Set(s);
    next.add(id);
    return next;
  });
  lastSelectedId.set(id);
}

/** Remove a single id. */
export function deselectOne(id: string): void {
  selectedIds.update((s) => {
    if (!s.has(id)) return s;
    const next = new Set(s);
    next.delete(id);
    return next;
  });
  lastSelectedId.update((last) => (last === id ? null : last));
}

/** Toggle a single id, returning the new membership state. */
export function toggleSelect(id: string): boolean {
  let nowSelected = false;
  selectedIds.update((s) => {
    const next = new Set(s);
    if (next.has(id)) {
      next.delete(id);
      nowSelected = false;
    } else {
      next.add(id);
      nowSelected = true;
    }
    return next;
  });
  lastSelectedId.set(id);
  return nowSelected;
}

/**
 * Range-select: select every id between `lastId` and `currentId` in
 * `orderedIds`, taking the same on/off state as the action that
 * triggered this call (`adding === true` mimics shift-click-down,
 * `false` mimics shift-click-up of a previously-selected row).
 *
 * If `lastId` is missing (e.g. the user shift-clicks on a fresh
 * selection), this falls back to selecting just `currentId`.
 */
export function selectRange(
  orderedIds: readonly string[],
  lastId: string | null,
  currentId: string,
  adding: boolean,
): void {
  if (!orderedIds.includes(currentId)) return;
  if (!lastId || !orderedIds.includes(lastId) || lastId === currentId) {
    if (adding) selectOne(currentId);
    else deselectOne(currentId);
    return;
  }
  const a = orderedIds.indexOf(lastId);
  const b = orderedIds.indexOf(currentId);
  const [lo, hi] = a < b ? [a, b] : [b, a];
  const range = orderedIds.slice(lo, hi + 1);
  selectedIds.update((s) => {
    const next = new Set(s);
    for (const id of range) {
      if (adding) next.add(id);
      else next.delete(id);
    }
    return next;
  });
  lastSelectedId.set(currentId);
}

/** Replace the selection with the entire provided id list. */
export function selectAll(orderedIds: readonly string[]): void {
  selectedIds.set(new Set(orderedIds));
  if (orderedIds.length > 0)
    lastSelectedId.set(orderedIds[orderedIds.length - 1]);
}

/** Clear the selection entirely. */
export function clearSelection(): void {
  selectedIds.set(new Set());
  lastSelectedId.set(null);
}

/** Mark the selection as no-longer-valid because a row left the
 *  store (e.g. removed). Called from the event listener. */
export function pruneSelection(remainingIds: ReadonlySet<string>): void {
  selectedIds.update((s) => {
    let changed = false;
    const next = new Set<string>();
    for (const id of s) {
      if (remainingIds.has(id)) next.add(id);
      else changed = true;
    }
    return changed ? next : s;
  });
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
