// Keyboard shortcut hook — ⌘K palette, navigation, actions
import { onMount } from "svelte";

export interface Shortcut {
  key: string;            // "k", "Escape", "ArrowUp"
  ctrl?: boolean;          // Cmd on Mac, Ctrl on Linux/Win
  shift?: boolean;
  alt?: boolean;
  meta?: boolean;          // explicit Cmd
  description: string;
  category?: string;
  handler: (e: KeyboardEvent) => void;
  enabled?: boolean;
}

let shortcuts: Shortcut[] = [];
let isMac = false;

export function registerShortcuts(list: Shortcut[]): () => void {
  shortcuts = list;
  return () => {
    shortcuts = shortcuts.filter((s) => !list.includes(s));
  };
}

export function getShortcuts(): readonly Shortcut[] {
  return shortcuts;
}

export function initKeyboardShortcuts(): () => void {
  if (typeof navigator !== "undefined") {
    isMac = /Mac|iPhone|iPad/i.test(navigator.platform);
  }
  const handler = (e: KeyboardEvent) => {
    // Don't intercept when typing in inputs (except Escape and global combos)
    const target = e.target as HTMLElement;
    const isInput = ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
    const isContentEditable = target.isContentEditable;

    for (const s of shortcuts) {
      if (s.enabled === false) continue;
      if (!matchKey(e, s, isMac)) continue;
      if ((isInput || isContentEditable) && s.key !== "Escape" && !(s.ctrl || s.meta)) continue;
      e.preventDefault();
      s.handler(e);
      return;
    }
  };
  window.addEventListener("keydown", handler);
  return () => window.removeEventListener("keydown", handler);
}

function matchKey(e: KeyboardEvent, s: Shortcut, isMac: boolean): boolean {
  if (e.key.toLowerCase() !== s.key.toLowerCase() && e.key !== s.key) return false;
  if (s.ctrl || s.meta) {
    const mod = isMac ? e.metaKey : e.ctrlKey;
    if (!mod) return false;
  } else {
    if (e.ctrlKey || e.metaKey) return false;
  }
  if (s.shift !== undefined) {
    if (Boolean(e.shiftKey) !== s.shift) return false;
  }
  if (s.alt !== undefined) {
    if (Boolean(e.altKey) !== s.alt) return false;
  }
  return true;
}

export function useKeyboardShortcuts(list: Shortcut[]): void {
  onMount(() => {
    const unregister = registerShortcuts(list);
    return unregister;
  });
}
