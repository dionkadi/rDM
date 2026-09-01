<script lang="ts">
  import { onMount, onDestroy } from "svelte";
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
  } from "./lib/components";
  import * as api from "./lib/api";
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
        key: "Escape", description: "Close modal", category: "Navigation",
        handler: () => {
          if ($showSettings) showSettings.set(false);
          if ($showCommandPalette) showCommandPalette.set(false);
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
        <input class="grow" placeholder="category (optional)" bind:value={newCategory} />
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
      {#each visible as d (d.id)}
        <div animate:flip={{ duration: 280 }} in:fly={{ y: 8, duration: 280, easing: cubicOut }}>
          <DownloadRow download={d} on:action={onAction} />
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
