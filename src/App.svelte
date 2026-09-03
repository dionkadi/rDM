<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { get } from "svelte/store";
  import { fly, fade } from "svelte/transition";
  import { flip } from "svelte/animate";
  import { cubicOut } from "svelte/easing";
  import { readText } from "@tauri-apps/plugin-clipboard-manager";
  import {
    DownloadRow,
    SpeedGraph,
    SmartFilters,
    GlobalSearch,
    CommandPalette,
    SettingsTabs,
    StatusBar,
    Toast,
    CaptureDialog,
    BulkActionBar,
  } from "./lib/components";
  import * as api from "./lib/api";
  import { setDownloadPriority as setPriority } from "./lib/api";
  import {
    downloads,
    aggregateSpeed,
    counts,
    totalDownloaded,
    refreshDownloads,
    addDownload as addDownloadStore,
    pauseDownload,
    resumeDownload,
    cancelDownload,
    removeDownload,
    setDownloadSpeedLimit,
    bulkPause,
    bulkResume,
    bulkRemove,
    bulkSetLimit,
    reorderDownloads,
    startSpeedMonitor,
    stopSpeedMonitor,
    startEventListener,
    stopEventListener,
  } from "./lib/stores/downloads";
  import {
    activeFilter,
    searchQuery,
    showSettings,
    showCommandPalette,
    sidebarCollapsed,
    globalSpeedLimitLabel,
    clipboardUrl,
    clipboardMonitorEnabled,
    dragActive,
    demoMode,
    showToast,
    selectedIds,
    hasSelection,
    selectAll,
    clearSelection,
    lastSelectedId,
    selectOne,
    toggleSelect,
    selectRange,
    type FilterKey,
  } from "./lib/stores/ui";
  import { settings, loadSettings } from "./lib/stores/settings";
  import { initKeyboardShortcuts, registerShortcuts } from "./lib/hooks/useKeyboardShortcuts";
  import { fmtBytes, fmtRate, filenameFromUrl, isUrl } from "./lib/utils/formatters";
  import type { Download } from "./lib/types";

  // ── Form state ────────────────────────────────────────────────
  let url = "";
  let newCategory = "";
  let checksumAlgo = "sha256";
  let checksumExpected = "";
  let showChecksum = false;
  let globalLimitMB = "";
  let error = "";
  let pingResult = "";

  let clipTimer: ReturnType<typeof setInterval> | null = null;
  let dragDepth = 0;

  // ── Demo data ─────────────────────────────────────────────────
  function mkDownload(o: Partial<Download> & { id: string; filename: string; url: string; totalSize: number; downloaded: number; status: Download["status"] }): Download {
    return {
      savePath: "",
      contentType: null,
      category: null,
      chunks: [],
      speedLimit: null,
      proxy: null,
      checksum: null,
      error: null,
      createdAt: new Date().toISOString(),
      finishedAt: null,
      canResume: true,
      ...o,
    } as Download;
  }
  function makeDemoDownload(u: string, cat: string | null): Download {
    const fn = filenameFromUrl(u);
    const total = Math.round((0.2 + Math.random() * 3.8) * 1e9);
    return mkDownload({
      id: "demo-" + Date.now().toString(36) + Math.random().toString(36).slice(2, 5),
      filename: fn,
      url: u,
      totalSize: total,
      downloaded: 0,
      status: "downloading",
      category: cat,
    });
  }
  function seedDemo(): Download[] {
    const base = "https://cdn.example.com/";
    return [
      mkDownload({ id: "d1", filename: "ubuntu-24.04-desktop-amd64.iso", url: base + "ubuntu-24.04-desktop-amd64.iso", totalSize: 4.2e9, downloaded: 2.6e9, status: "downloading", chunks: [
        { index: 0, start: 0, end: 1048575, downloaded: 1048575 },
        { index: 1, start: 1048576, end: 2097151, downloaded: 2097151 },
        { index: 2, start: 2097152, end: 3145727, downloaded: 1820000 },
        { index: 3, start: 3145728, end: 4194303, downloaded: 600000 },
      ] }),
      mkDownload({ id: "d2", filename: "product-demo-final_cut.mp4", url: base + "product-demo-final_cut.mp4", totalSize: 1.4e9, downloaded: 9.1e8, status: "downloading", chunks: [
        { index: 0, start: 0, end: 524287, downloaded: 524287 },
        { index: 1, start: 524288, end: 1048575, downloaded: 1048575 },
        { index: 2, start: 1048576, end: 1399463, downloaded: 800000 },
      ] }),
      mkDownload({ id: "d3", filename: "node-v20.11.0-linux-x64.tar.gz", url: base + "node-v20.11.0-linux-x64.tar.gz", totalSize: 2.3e8, downloaded: 2.3e8, status: "completed", finishedAt: new Date().toISOString() }),
      mkDownload({ id: "d4", filename: "quarterly-report-Q4.pdf", url: base + "quarterly-report-Q4.pdf", totalSize: 8.2e6, downloaded: 8.2e6, status: "completed", finishedAt: new Date().toISOString() }),
      mkDownload({ id: "d5", filename: "album-flac-pack.zip", url: base + "album-flac-pack.zip", totalSize: 6.1e8, downloaded: 1.2e8, status: "paused" }),
      mkDownload({ id: "d6", filename: "driver-installer.exe", url: base + "driver-installer.exe", totalSize: 5.4e8, downloaded: 0, status: "queued" }),
    ];
  }
  let demoTimer: ReturnType<typeof setInterval> | null = null;
  function startDemoSim() {
    stopDemoSim();
    demoTimer = setInterval(() => {
      downloads.update((list) => list.map((d) => {
        if (d.status !== "downloading" && d.status !== "connecting") return d;
        const sp = (1.5 + Math.random() * 6) * 1024 * 1024;
        let next = d.downloaded + sp * 0.7;
        let status: Download["status"] = d.status;
        if (d.totalSize && next >= d.totalSize) {
          next = d.totalSize;
          status = "completed";
        }
        return { ...d, downloaded: next, status };
      }));
    }, 700);
  }
  function stopDemoSim() {
    if (demoTimer) clearInterval(demoTimer);
    demoTimer = null;
  }

  /**
   * Mirror of the engine's `save_dir_for(category)` — used to
   * render the save-path preview next to the category dropdown
   * on the home view. `catId === ""` (no category) falls back to
   * the global default save dir; otherwise we look up the
   * category's `directory` field. The engine is the source of
   * truth at the moment the user actually clicks "Add" — this is
   * just a preview to help the user pick a category.
   */
  function resolveDirFor(catId: string): string {
    const s = $settings;
    if (!s) return "";
    if (!catId) {
      return typeof s.defaultDirectory === "string"
        ? s.defaultDirectory
        : String(s.defaultDirectory ?? "");
    }
    const cat = s.categories.find((c) => c.id === catId);
    if (!cat) {
      return typeof s.defaultDirectory === "string"
        ? s.defaultDirectory
        : String(s.defaultDirectory ?? "");
    }
    return cat.directory || String(s.defaultDirectory ?? "");
  }

  // ── Derived ───────────────────────────────────────────────────
  $: filter = $activeFilter;
  $: visible = filterDownloads($downloads, filter, $searchQuery);
  $: headerTitle = filter === "all" ? "Downloads" : filter[0].toUpperCase() + filter.slice(1);

  function filterDownloads(list: Download[], f: FilterKey, q: string): Download[] {
    let out = list;
    if (f !== "all") {
      out = out.filter((d) => matchFilter(d, f));
    }
    if (q.trim()) {
      const qLower = q.toLowerCase();
      out = out.filter((d) => (d.filename || d.url).toLowerCase().includes(qLower));
    }
    return out;
  }
  function matchFilter(d: Download, f: FilterKey): boolean {
    switch (f) {
      case "active": return !["completed", "error", "canceled"].includes(d.status);
      case "queued": return d.status === "queued" || d.status === "scheduled";
      case "paused": return d.status === "paused";
      case "completed": return d.status === "completed";
      case "error": return d.status === "error";
      case "scheduled": return d.status === "scheduled";
      case "video": case "audio": case "archive": case "document": case "image": case "binary":
        return isFileType(d, f);
      default: return true;
    }
  }
  function isFileType(d: Download, kind: string): boolean {
    const ext = (d.filename || d.url).split(".").pop()?.toLowerCase() || "";
    const map: Record<string, string[]> = {
      video: ["mp4","mkv","webm","mov","avi","flv","m3u8"],
      audio: ["mp3","wav","flac","ogg","aac","m4a"],
      archive: ["zip","rar","7z","gz","tar","tgz","bz2"],
      document: ["pdf","doc","docx","xls","xlsx","ppt","pptx","txt","md","rtf"],
      image: ["jpg","jpeg","png","gif","webp","svg","bmp"],
      binary: ["exe","msi","appimage","deb","rpm","apk","iso","img","dmg"],
    };
    return (map[kind] || []).includes(ext);
  }

  // ── Actions ───────────────────────────────────────────────────
  async function add(u: string, cat?: string | null, checksum?: { algorithm: string; expected: string } | null) {
    if (!u) return;
    url = "";
    checksumExpected = "";
    showChecksum = false;
    if ($demoMode) {
      downloads.update((list) => [makeDemoDownload(u, cat ?? null), ...list]);
      showToast({ kind: "success", title: "Download added", message: filenameFromUrl(u) });
      return;
    }
    try {
      await addDownloadStore({ url: u, category: cat ?? null, checksum: checksum ?? null });
      showToast({ kind: "success", title: "Download added", message: filenameFromUrl(u) });
    } catch (e) {
      error = String(e);
      showToast({ kind: "error", title: "Failed to add", message: String(e) });
    }
  }

  function currentChecksum() {
    const hex = checksumExpected.trim();
    return hex === "" ? null : { algorithm: checksumAlgo, expected: hex };
  }

  function applyLimit(bytes: number | null, label: string) {
    globalSpeedLimitLabel.set(label);
    if ($demoMode) return;
    api.setGlobalSpeedLimit(bytes).catch((e) => (error = String(e)));
  }
  function applyLimitMB() {
    const v = globalLimitMB.trim();
    if (v === "") {
      applyLimit(null, "Unlimited");
      return;
    }
    const mb = Number(v);
    if (isNaN(mb) || mb < 0) return;
    applyLimit(Math.round(mb * 1024 * 1024), `${mb} MB/s`);
  }
  function presetLimit(mb: number | null) {
    if (mb == null) {
      globalLimitMB = "";
      applyLimit(null, "Unlimited");
    } else {
      globalLimitMB = String(mb);
      applyLimit(Math.round(mb * 1024 * 1024), `${mb} MB/s`);
    }
  }

  async function onAction(e: CustomEvent) {
    const { type, id, limit } = e.detail;
    if ($demoMode) {
      downloads.update((list) => list.map((d) => {
        if (d.id !== id) return d;
        if (type === "pause") return { ...d, status: "paused" as const };
        if (type === "resume") return { ...d, status: "downloading" as const };
        if (type === "cancel") return { ...d, status: "canceled" as const };
        if (type === "set-limit") return { ...d, speedLimit: limit };
        return d;
      }).filter((d) => type !== "remove" || d.id !== id));
      return;
    }
    try {
      if (type === "pause") await pauseDownload(id);
      else if (type === "resume") await resumeDownload(id);
      else if (type === "cancel") await cancelDownload(id);
      else if (type === "remove") await removeDownload(id);
      else if (type === "set-limit") await setDownloadSpeedLimit(id, limit);
      else if (type === "set-priority") {
        // Per-download priority (0 = low, 1 = normal, 2 = high).
        // The engine clamps to `[0, 2]`. We don't refresh the
        // full list because the engine emits a `StatusChanged`
        // event with the new priority, which the event
        // listener merges into the store.
        await setPriority(id, limit);
      }
      else if (type === "open-folder") {
        // Find the download's save_path and hand it to the Rust opener.
        // The command opens the parent directory (or reveals the file
        // if it exists), so the user always lands somewhere useful —
        // even for transfers that are still in flight.
        const d = $downloads.find((x) => x.id === id);
        const path = d?.savePath;
        if (!path) {
          showToast({
            kind: "error",
            title: "No save path",
            message: "This download has no save path yet.",
          });
          return;
        }
        await api.openFolder(path, false);
      }
    } catch (err) {
      showToast({ kind: "error", title: "Action failed", message: String(err) });
    }
  }

  // ── Bulk action handler ──────────────────────────────────────
  //
  // The `BulkActionBar` dispatches `{ type, ids, limit? }` events
  // with an *array* of ids (vs. the single-id per-row events from
  // `onAction`). We fan them out to the bulk store helpers, which
  // run all the per-row Tauri calls in parallel via `Promise.allSettled`.
  // We *do not* clear the selection on success — the user can see
  // the rows re-render and decide whether to clear or do another
  // action. We *do* clear on remove (the rows are gone anyway).
  async function onBulkAction(e: CustomEvent) {
    const { type, ids, limit } = e.detail as {
      type: string;
      ids: string[];
      limit?: number | null;
    };
    if (!ids || ids.length === 0) return;

    if ($demoMode) {
      // Demo mode: mutate the local store directly, mirroring
      // the single-row branch in `onAction`.
      const idSet = new Set(ids);
      downloads.update((list) => {
        if (type === "remove") return list.filter((d) => !idSet.has(d.id));
        return list.map((d) => {
          if (!idSet.has(d.id)) return d;
          if (type === "pause") return { ...d, status: "paused" as const };
          if (type === "resume") return { ...d, status: "downloading" as const };
          if (type === "set-limit") return { ...d, speedLimit: limit ?? null };
          return d;
        });
      });
      if (type === "remove") clearSelection();
      showToast({ kind: "info", title: `${ids.length} ${verbFor(type)}` });
      return;
    }

    try {
      if (type === "pause") await bulkPause(ids);
      else if (type === "resume") await bulkResume(ids);
      else if (type === "remove") await bulkRemove(ids);
      else if (type === "set-limit") await bulkSetLimit(ids, limit ?? null);
      showToast({ kind: "info", title: `${ids.length} ${verbFor(type)}` });
      if (type === "remove") clearSelection();
    } catch (err) {
      showToast({ kind: "error", title: "Bulk action failed", message: String(err) });
    }
  }

  function verbFor(type: string): string {
    switch (type) {
      case "pause": return "paused";
      case "resume": return "resumed";
      case "remove": return "removed";
      case "set-limit": return "limit set";
      default: return type;
    }
  }

  // ── Row selection wiring ──────────────────────────────────────────
  //
  // `DownloadRow` dispatches `select` with the row's id and the
  // shift/ctrl modifiers we captured in its own click handler.
  // We translate that into the right store action so the rest of
  // the UI (the bulk-action bar, the visible-row index for
  // shift-range) stays decoupled from the row component.
  function onRowSelect(
    e: CustomEvent<{ id: string; shift: boolean; ctrlOrMeta: boolean }>,
    visibleIds: string[],
  ) {
    const { id, shift, ctrlOrMeta } = e.detail;
    if (shift) {
      // Shift-click extends the selection as a contiguous range
      // from the most recently clicked id. We treat it as an
      // "adding" action when the current row is *not* already in
      // the selection, and as a "removing" action when it is —
      // that mirrors Finder / Explorer behaviour and matches
      // what users expect from "shift-click to extend".
      const isInSel = (get(selectedIds) as Set<string>).has(id);
      selectRange(visibleIds, get(lastSelectedId), id, !isInSel);
    } else if (ctrlOrMeta) {
      // Cmd/Ctrl-click toggles the single row's membership; the
      // other selected rows are left alone.
      toggleSelect(id);
    } else {
      // Plain click in selection-mode (= row is the only one
      // selected). Plain click outside selection-mode does
      // nothing — the row still receives a single-click `action`
      // event for things like pause/resume.
      const sel = get(selectedIds) as Set<string>;
      if (sel.size > 0) {
        // Replace the selection with just this row.
        clearSelection();
        selectOne(id);
      }
    }
  }

  // ── Row reorder wiring (drag-and-drop + Alt+↑/↓) ─────────────
  //
  // Both the drag-and-drop payload (`reorder`) and the
  // keyboard alternative (`reorder-keyboard`) bubble up here.
  // We build the new visible-order id list and pass it to
  // `reorderDownloads`, which calls the Rust
  // `reorder_downloads` command. The engine assigns fresh
  // `sort_key` values and emits per-row `StatusChanged`
  // events; the store's event listener merges them and the
  // list re-renders with the new order.
  async function onRowReorder(
    e: CustomEvent<{
      sourceId: string;
      targetId: string;
      position: "before" | "after";
    } | {
      id: string;
      from: number;
      to: number;
    }>,
    visibleIds: string[],
  ) {
    // Compute the new id list for either the drag-and-drop or
    // the keyboard case. Both reduce to a "new order" array.
    let newOrder: string[];
    if ("sourceId" in e.detail) {
      const { sourceId, targetId, position } = e.detail;
      if (sourceId === targetId) return;
      const fromIdx = visibleIds.indexOf(sourceId);
      const toIdx = visibleIds.indexOf(targetId);
      if (fromIdx < 0 || toIdx < 0) return;
      const next = visibleIds.slice();
      next.splice(fromIdx, 1);
      const insertAt = position === "before" ? next.indexOf(targetId) : next.indexOf(targetId) + 1;
      next.splice(insertAt, 0, sourceId);
      newOrder = next;
    } else {
      const { from, to } = e.detail;
      // `from` and `to` are 1-based; convert to 0-based.
      const fromIdx = from - 1;
      const toIdx = to - 1;
      if (fromIdx < 0 || fromIdx >= visibleIds.length) return;
      const next = visibleIds.slice();
      const [moved] = next.splice(fromIdx, 1);
      next.splice(toIdx, 0, moved);
      newOrder = next;
    }
    try {
      await reorderDownloads(newOrder);
    } catch (err) {
      showToast({ kind: "error", title: "Reorder failed", message: String(err) });
    }
  }

  async function doPing() {
    try {
      pingResult = await api.ping();
    } catch {
      pingResult = "offline";
    }
  }

  // ── Autostart (informational; the actual toggle lives in Settings) ───
  // Note: the autostart plugin is queried at app start to verify availability.
  // Toggling happens via the Settings panel.
  async function loadAutostart() {
    try {
      const { isEnabled } = await import("@tauri-apps/plugin-autostart");
      await isEnabled();
    } catch { /* unavailable outside Tauri */ }
  }

  // ── Clipboard ─────────────────────────────────────────────────
  async function checkClipboard() {
    try {
      const t = await readText();
      if (t && isUrl(t)) clipboardUrl.set(t.trim());
    } catch { /* unavailable */ }
  }
  function startClipboardMonitor() {
    stopClipboardMonitor();
    clipboardMonitorEnabled.set(true);
    clipTimer = setInterval(checkClipboard, 2000);
  }
  function stopClipboardMonitor() {
    clipboardMonitorEnabled.set(false);
    if (clipTimer) clearInterval(clipTimer);
    clipTimer = null;
  }
  async function addClipboard() {
    if (!$clipboardUrl) return;
    await add($clipboardUrl, newCategory || null);
    clipboardUrl.set("");
  }

  async function applyPaletteProxy(mode: string) {
    if (mode !== "none" && mode !== "system" && mode !== "manual") return;
    if (mode === "manual") {
      showSettings.set(true);
      if (typeof window !== "undefined") {
        window.dispatchEvent(new CustomEvent("dm:focus-proxy-url"));
      }
    }
    const { setProxyMode } = await import("./lib/stores/settings");
    await setProxyMode(mode, $settings?.proxy ?? null);
  }

  // ── Drag & drop ───────────────────────────────────────────────
  function onDragEnter(e: DragEvent) {
    e.preventDefault();
    dragDepth++;
    dragActive.set(true);
  }
  function onDragLeave(e: DragEvent) {
    e.preventDefault();
    dragDepth--;
    if (dragDepth <= 0) {
      dragDepth = 0;
      dragActive.set(false);
    }
  }
  function onDragOver(e: DragEvent) { e.preventDefault(); }
  async function onDrop(e: DragEvent) {
    e.preventDefault();
    dragDepth = 0;
    dragActive.set(false);
    const raw = e.dataTransfer?.getData("text/uri-list") || e.dataTransfer?.getData("text/plain") || "";
    const urls = raw.split(/\r?\n/).map((l) => l.trim()).filter((l) => l && !l.startsWith("#")).filter(isUrl);
    for (const u of urls) await add(u, newCategory || null);
  }

  // ── Sidebar toggle ────────────────────────────────────────────
  function toggleSidebar() {
    sidebarCollapsed.update((c) => !c);
  }

  // ── Lifecycle ─────────────────────────────────────────────────
  // ── Body scroll lock ─────────────────────────────────────────
  //
  // While any modal-style panel is open (Settings, Command Palette, …)
  // we want the wheel to operate **only** on the panel — never on the
  // page behind it. The browser's default is "pointer-aware": a wheel
  // over the backdrop (which is not scrollable) cascades up to the
  // document and scrolls the home view. We replace that with
  // "modal-aware" behaviour by setting `overflow: hidden` on both
  // `<html>` and `<body>` whenever a modal layer is active.
  //
  // We also stash the previous `overflow` value so we can restore it
  // on close (rather than blindly clearing it — the Tauri webview
  // may have a non-default `overflow` we shouldn't clobber).
  let prevBodyOverflow = "";
  let prevHtmlOverflow = "";
  let lockCount = 0;
  function lockBodyScroll() {
    if (typeof document === "undefined") return;
    if (lockCount === 0) {
      const body = document.body;
      const html = document.documentElement;
      prevBodyOverflow = body.style.overflow;
      prevHtmlOverflow = html.style.overflow;
      // `!important` via `setProperty` so a stray per-element rule
      // (e.g. a body class added by an extension) can't override us.
      body.style.setProperty("overflow", "hidden", "important");
      html.style.setProperty("overflow", "hidden", "important");
    }
    lockCount++;
  }
  function unlockBodyScroll() {
    if (typeof document === "undefined") return;
    if (lockCount === 0) return;
    lockCount--;
    if (lockCount === 0) {
      const body = document.body;
      const html = document.documentElement;
      body.style.removeProperty("overflow");
      html.style.removeProperty("overflow");
      // Fallback in case the rules came from a stylesheet, not
      // inline: re-apply the previous inline value.
      if (prevBodyOverflow) body.style.overflow = prevBodyOverflow;
      if (prevHtmlOverflow) html.style.overflow = prevHtmlOverflow;
    }
  }
  $: if (typeof window !== "undefined") {
    if ($showSettings || $showCommandPalette) {
      lockBodyScroll();
    } else {
      unlockBodyScroll();
    }
  }

  onMount(() => {
    // Load initial data (fire and forget)
    (async () => {
      try {
        await refreshDownloads();
      } catch {
        demoMode.set(true);
        downloads.set(seedDemo());
        startDemoSim();
      }
      await loadSettings();
      if ($settings?.clipboardMonitor) startClipboardMonitor();
      await loadAutostart();
      startSpeedMonitor();
      await startEventListener();
      doPing();
    })();

    // Global keyboard shortcuts
    const unregister = registerShortcuts([
      {
        key: "k", ctrl: true, meta: true, description: "Open command palette", category: "Navigation",
        handler: () => showCommandPalette.update((v) => !v),
      },
      {
        key: ",", ctrl: true, meta: true, description: "Open settings", category: "Navigation",
        handler: () => showSettings.update((v) => !v),
      },
      {
        key: "n", ctrl: true, meta: true, description: "Add new download", category: "Actions",
        handler: () => document.querySelector<HTMLInputElement>(".add-input")?.focus(),
      },
      {
        key: "/", description: "Focus search", category: "Navigation",
        handler: () => showCommandPalette.set(true),
      },
      {
        key: "a", ctrl: true, meta: true, description: "Select all visible downloads", category: "Actions",
        handler: () => {
          // Only act when the download list is in focus (i.e. no
          // modal is open that would hijack Cmd+A). We mirror the
          // input-aware guard from `useKeyboardShortcuts` here too
          // — the engine stops here, so the OS doesn't grab the
          // event for "select all in the focused text field".
          selectAll(visible.map((d) => d.id));
        },
      },
      {
        key: "Escape", description: "Close modal / clear selection", category: "Navigation",
        handler: () => {
          if ($showSettings) { showSettings.set(false); return; }
          if ($showCommandPalette) { showCommandPalette.set(false); return; }
          // No modal open — if the user has a selection, clear it
          // first (so a stray Esc doesn't dismiss anything else).
          if ($hasSelection) clearSelection();
        },
      },
      {
        key: "[", ctrl: true, meta: true, description: "Toggle sidebar", category: "View",
        handler: toggleSidebar,
      },
    ]);
    initKeyboardShortcuts();
    return unregister;
  });

  onDestroy(() => {
    stopClipboardMonitor();
    stopSpeedMonitor();
    stopEventListener();
    stopDemoSim();
    // Belt-and-braces: if a modal is somehow still open at teardown,
    // make sure we don't leave the document with `overflow: hidden`
    // — that would render the next launch as a non-scrolling page
    // until the user happens to toggle a panel.
    while (lockCount > 0) unlockBodyScroll();
  });
</script>

<div
  class="shell"
  class:collapsed={$sidebarCollapsed}
  role="application"
  on:dragenter={onDragEnter}
  on:dragleave={onDragLeave}
  on:dragover={onDragOver}
  on:drop={onDrop}
>
  <!-- ── Sidebar ─────────────────────────────────────────────── -->
  <aside class="sidebar" aria-label="Main navigation">
    <div class="brand">
      <div class="mark" aria-hidden="true">
        <svg viewBox="0 0 24 24" fill="none">
          <path d="M12 4v9m0 0l-3.4-3.4M12 13l3.4-3.4" stroke="currentColor" stroke-width="2.1" stroke-linecap="round" stroke-linejoin="round"/>
          <path d="M5 19h14" stroke="currentColor" stroke-width="2.1" stroke-linecap="round"/>
        </svg>
      </div>
      <div class="brand-text">
        <div class="word">DM</div>
        <div class="sub">Download Manager</div>
      </div>
      <div class="live" class:idle={!$counts.active} title={$counts.active ? "Active transfers" : "Idle"}></div>
    </div>

    <div class="search-wrap">
      <GlobalSearch />
    </div>

    <SmartFilters />

    <div class="sidebar-footer">
      <div class="side-stat" class:collapsed={$sidebarCollapsed}>
        {#if !$sidebarCollapsed}
          <div class="row"><span class="k">Throughput</span><span class="v">{fmtRate($aggregateSpeed)}</span></div>
          <div class="row"><span class="k">Downloaded</span><span class="v">{fmtBytes($totalDownloaded)}</span></div>
          <div class="row"><span class="k">Speed cap</span><span class="v">{$globalSpeedLimitLabel}</span></div>
        {:else}
          <div class="row-icons" title="Throughput: {fmtRate($aggregateSpeed)}">
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 4v11m0 0l-4-4m4 4l4-4"/><path d="M5 20h14"/></svg>
            <span class="v">{fmtRate($aggregateSpeed)}</span>
          </div>
        {/if}
      </div>
      {#if $demoMode}
        <div class="demo-flag" title="Preview mode · demo data">
          {#if !$sidebarCollapsed}Preview · demo data{:else}⚠{/if}
        </div>
      {/if}
    </div>
  </aside>

  <!-- ── Main content ────────────────────────────────────────── -->
  <main class="main">
    <header class="topbar">
      <button class="btn btn-ghost btn-icon" on:click={toggleSidebar} aria-label="Toggle sidebar" title="Toggle sidebar (⌘[)">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M3 12h18M3 18h18"/></svg>
      </button>
      <div>
        <h1>{headerTitle}</h1>
        <div class="lead">Segmented transfers · pause &amp; resume · live throughput</div>
      </div>
      <div class="spacer"></div>
      <button class="btn btn-ghost btn-sm" on:click={doPing} title="Ping engine">Ping</button>
      <span class="chip" style="border-color:transparent;background:transparent;color:var(--muted);font-size:11.5px">{pingResult || "—"}</span>
      <button class="btn btn-ghost btn-icon" on:click={() => showSettings.set(true)} aria-label="Open settings" title="Settings (⌘,)">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-2.82 1.17V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 8 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 3.6 15H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6h.09A1.65 1.65 0 0 0 11 3.09V3a2 2 0 1 1 4 0v.09A1.65 1.65 0 0 0 16 4.6a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 20.4 9v.09A1.65 1.65 0 0 0 22 11h0a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
      </button>
    </header>

    <!-- ── Add hero ──────────────────────────────────────────── -->
    <section class="panel add-hero">
      <div class="add-row">
        <input
          class="add-input"
          placeholder="Paste a download link — https://example.com/big-file.iso"
          bind:value={url}
          on:keydown={(e) => e.key === "Enter" && add(url, newCategory || null, currentChecksum())}
        />
        <button class="btn btn-primary" on:click={() => add(url, newCategory || null, currentChecksum())}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M12 4v11m0 0l-4-4m4 4l4-4"/><path d="M5 20h14"/></svg>
          Add
        </button>
      </div>

      <div class="add-sub">
        <select
          class="cat-select"
          bind:value={newCategory}
          aria-label="Category"
        >
          <option value="">Default (no category)</option>
          {#each $settings?.categories ?? [] as cat}
            <option value={cat.id}>{cat.name}</option>
          {/each}
        </select>
        {#if newCategory}
          <span class="dir-hint" title="Resolved save directory">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>
            </svg>
            <code>{resolveDirFor(newCategory)}</code>
          </span>
        {/if}
        <label class="chk-toggle">
          <input type="checkbox" bind:checked={showChecksum} />
          checksum
        </label>
        {#if showChecksum}
          <input class="grow" style="flex:1 1 160px" placeholder="expected hash (hex)" bind:value={checksumExpected} />
        {/if}
      </div>

      <div class="add-sub">
        <span class="eyebrow" style="align-self:center">Speed cap</span>
        <div class="cap-chips">
          <button class="chip-btn" class:on={globalLimitMB === ""} on:click={() => presetLimit(null)}>∞</button>
          <button class="chip-btn" class:on={globalLimitMB === "5"} on:click={() => presetLimit(5)}>5 MB/s</button>
          <button class="chip-btn" class:on={globalLimitMB === "20"} on:click={() => presetLimit(20)}>20 MB/s</button>
          <button class="chip-btn" class:on={globalLimitMB === "50"} on:click={() => presetLimit(50)}>50 MB/s</button>
        </div>
        <input
          class="grow"
          style="flex:0 1 150px"
          type="number"
          min="0"
          placeholder="custom MB/s"
          bind:value={globalLimitMB}
          on:change={applyLimitMB}
        />
      </div>

      {#if $clipboardUrl}
        <div class="add-sub">
          <span class="clip-pill">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/></svg>
            <b>{$clipboardUrl}</b>
          </span>
          <button class="link-btn" on:click={addClipboard}>add from clipboard</button>
        </div>
      {/if}

      {#if error}
        <div class="err">{error}</div>
      {/if}
    </section>

    <!-- ── Throughput ────────────────────────────────────────── -->
    <section class="panel throughput">
      <div>
        <div class="tp-head">
          <div>
            <div class="eyebrow">Aggregate throughput</div>
            <div class="big-rate">{fmtRate($aggregateSpeed).split(" ")[0]}<span class="unit">{fmtRate($aggregateSpeed).split(" ")[1]}</span></div>
          </div>
          <div class="tp-stats">
            <div><span class="tp-num">{$counts.active}</span><span class="tp-lbl">active</span></div>
            <div><span class="tp-num">{fmtBytes($totalDownloaded)}</span><span class="tp-lbl">downloaded</span></div>
            <div><span class="tp-num">{$globalSpeedLimitLabel}</span><span class="tp-lbl">cap</span></div>
          </div>
        </div>
        <div class="tp-graph">
          <SpeedGraph speed={$aggregateSpeed} />
        </div>
      </div>
    </section>

    <!-- ── List ─────────────────────────────────────────────── -->
    <div class="list-head">
      <span class="h">{$searchQuery ? `Results for "${$searchQuery}"` : headerTitle}</span>
      <span class="n">{visible.length} shown · {$downloads.length} total</span>
    </div>

    {#if visible.length === 0}
      <div class="empty" in:fade={{ duration: 250 }}>
        <div class="glyph">
          <svg width="30" height="30" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6"><path d="M12 4v9m0 0l-3.4-3.4M12 13l3.4-3.4"/><path d="M5 19h14"/></svg>
        </div>
        {#if $searchQuery}
          <h3>No matches</h3>
          <p>No downloads match "{$searchQuery}".</p>
        {:else if $downloads.length === 0}
          <h3>Nothing here yet</h3>
          <p>Paste a URL above, or drop a link anywhere in the window to start a download.</p>
          <div class="hint-row">
            <kbd>⌘K</kbd> open search
            <kbd>⌘N</kbd> new download
            <kbd>drop</kbd> any link
          </div>
        {:else}
          <h3>Empty {filter} list</h3>
          <p>Switch filters or start a new download.</p>
        {/if}
      </div>
    {:else}
      <BulkActionBar
        visibleIds={visible.map((d) => d.id)}
        on:action={onBulkAction}
      />
      {#each visible as d, i (d.id)}
        <div animate:flip={{ duration: 280 }} in:fly={{ y: 8, duration: 280, easing: cubicOut }}>
          <DownloadRow
            download={d}
            selectable
            selected={$selectedIds.has(d.id)}
            index={i + 1}
            total={visible.length}
            on:action={onAction}
            on:select={(e) => onRowSelect(e, visible.map((x) => x.id))}
            on:reorder={(e) => onRowReorder(e, visible.map((x) => x.id))}
            on:reorder-keyboard={(e) => onRowReorder(e, visible.map((x) => x.id))}
          />
        </div>
      {/each}
    {/if}
  </main>
</div>

<!-- ── Settings drawer ───────────────────────────────────────── -->
{#if $showSettings}
  <div class="backdrop" role="button" aria-label="Close settings" tabindex="0" transition:fade={{ duration: 200 }} on:click={() => showSettings.set(false)} on:keydown={(e) => { if (e.key === "Escape") showSettings.set(false); }}></div>
  <SettingsTabs onClose={() => showSettings.set(false)} />
{/if}

<!-- ── Command palette ───────────────────────────────────────── -->
<CommandPalette
  on:action={async (e) => {
    if (e.detail.type === "add") document.querySelector<HTMLInputElement>(".add-input")?.focus();
    else if (e.detail.type === "settings") showSettings.set(true);
    else if (e.detail.type === "set-proxy") {
      await applyPaletteProxy(e.detail.mode);
    }
    else if (e.detail.type === "pause-all") {
      const active = $downloads.filter((d) => d.status === "downloading" || d.status === "connecting");
      active.forEach((d) => onAction(new CustomEvent("a", { detail: { type: "pause", id: d.id } })));
      showToast({ kind: "info", title: `Paused ${active.length} downloads` });
    }
    else if (e.detail.type === "resume-all") {
      const paused = $downloads.filter((d) => d.status === "paused" || d.status === "error");
      paused.forEach((d) => onAction(new CustomEvent("a", { detail: { type: "resume", id: d.id } })));
      showToast({ kind: "info", title: `Resumed ${paused.length} downloads` });
    }
    else if (e.detail.type === "clear-completed") {
      const done = $downloads.filter((d) => d.status === "completed");
      done.forEach((d) => onAction(new CustomEvent("a", { detail: { type: "remove", id: d.id } })));
      showToast({ kind: "info", title: `Removed ${done.length} completed` });
    }
  }}
/>

<!-- ── Toasts ────────────────────────────────────────────────── -->
<Toast />

<!-- ── Capture dialog ────────────────────────────────────────── -->
<!-- The `CaptureDialog` listens to the `captureQueue` store and
     shows an IDM-style confirmation prompt for every URL the
     browser extension forwards. It is the *only* path that
     calls `addDownload` for browser-captured URLs. -->
<CaptureDialog />

<!-- ── Status bar ────────────────────────────────────────────── -->
<StatusBar />

<!-- ── Drop veil ─────────────────────────────────────────────── -->
{#if $dragActive}
  <div class="dropveil" transition:fade={{ duration: 120 }}>
    <div class="box">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6"><path d="M12 4v11m0 0l-4-4m4 4l4-4"/><path d="M5 20h14"/></svg>
      <div class="big">Drop to download</div>
      <div class="small">Release the link anywhere to add it to the queue</div>
    </div>
  </div>
{/if}
