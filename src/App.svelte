<script lang="ts">
  import { onMount } from "svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { readText } from "@tauri-apps/plugin-clipboard-manager";
  import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
  import * as api from "./lib/api";
  import SpeedGraph from "./lib/SpeedGraph.svelte";
  import type { Download, FrontendEvent, Settings } from "./lib/types";

  let pingResult = "";
  let url = "";
  let newCategory = "";
  let globalLimit = "";
  let downloads: Download[] = [];
  let error = "";

  let settings: Settings | null = null;
  let clipboardUrl = "";
  let clipTimer: ReturnType<typeof setInterval> | null = null;
  let showSettings = false;

  // Launch-at-login (autostart plugin) and optional checksum entry on add.
  let autostart = false;
  let checksumAlgo = "sha256";
  let checksumExpected = "";

  // Aggregate live speed (bytes/sec), sampled once per second from the
  // in-flight downloads and fed to the SpeedGraph for a rolling chart.
  let aggregateSpeed = 0;
  let speedTimer: ReturnType<typeof setInterval> | null = null;
  let prevActiveTotal = 0;
  let speedSeeded = false;
  function startSpeedMonitor() {
    stopSpeedMonitor();
    speedTimer = setInterval(() => {
      const total = downloads
        .filter((d) => d.status === "downloading")
        .reduce((s, d) => s + (d.downloaded || 0), 0);
      if (!speedSeeded) {
        speedSeeded = true;
        prevActiveTotal = total;
        return;
      }
      aggregateSpeed = Math.max(0, total - prevActiveTotal);
      prevActiveTotal = total;
    }, 1000);
  }
  function stopSpeedMonitor() {
    if (speedTimer) clearInterval(speedTimer);
    speedTimer = null;
  }

  async function doPing() {
    try {
      pingResult = await api.ping();
    } catch (e) {
      pingResult = `ping failed: ${String(e)}`;
    }
  }

  async function refresh() {
    try {
      downloads = await api.listDownloads();
    } catch (e) {
      error = String(e);
    }
  }

  async function add(
    u: string,
    cat?: string | null,
    checksum?: { algorithm: string; expected: string } | null,
  ) {
    if (!u) return;
    try {
      await api.addDownload({ url: u, category: cat ?? null, checksum: checksum ?? null });
      await refresh();
      checksumExpected = "";
    } catch (e) {
      error = String(e);
    }
  }

  // Build a ChecksumSpec from the optional checksum inputs (manual add only).
  function currentChecksum(): { algorithm: string; expected: string } | null {
    const hex = checksumExpected.trim();
    return hex === "" ? null : { algorithm: checksumAlgo, expected: hex };
  }

  async function applyGlobalLimit() {
    const v = globalLimit.trim() === "" ? null : Number(globalLimit);
    if (Number.isNaN(v as number)) return;
    await api.setGlobalSpeedLimit(v);
    globalLimit = "";
  }

  async function pause(d: Download) {
    await api.pauseDownload(d.id);
    await refresh();
  }
  async function resume(d: Download) {
    await api.resumeDownload(d.id);
    await refresh();
  }
  async function cancel(d: Download) {
    await api.cancelDownload(d.id);
    await refresh();
  }
  async function remove(d: Download) {
    await api.removeDownload(d.id);
    await refresh();
  }

  // ---- Settings ----
  async function loadSettings() {
    settings = await api.getSettings();
  }
  function pad(n: number): string {
    return n.toString().padStart(2, "0");
  }
  function fmtTime(t: [number, number]): string {
    return `${pad(t[0])}:${pad(t[1])}`;
  }
  function parseTime(s: string): [number, number] {
    const [h, m] = s.split(":").map((x) => Number(x) || 0);
    return [h, m];
  }
  async function saveSettings(patch: Partial<Settings>) {
    if (!settings) settings = await api.getSettings();
    const next = { ...settings, ...patch };
    await api.updateSettings(next);
    settings = next;
  }
  function onScheduleToggle(e: Event) {
    saveSettings({ scheduleEnabled: (e.target as HTMLInputElement).checked });
  }
  function onStartChange(e: Event) {
    saveSettings({ scheduleStart: parseTime((e.target as HTMLInputElement).value) });
  }
  function onEndChange(e: Event) {
    saveSettings({ scheduleEnd: parseTime((e.target as HTMLInputElement).value) });
  }
  function onClipboardToggle(e: Event) {
    const on = (e.target as HTMLInputElement).checked;
    saveSettings({ clipboardMonitor: on });
    if (on) startClipboardMonitor();
    else stopClipboardMonitor();
  }
  function onProxyChange(e: Event) {
    const v = (e.target as HTMLInputElement).value.trim();
    saveSettings({ proxy: v === "" ? null : v });
  }

  // ---- Autostart (launch at login) ----
  async function loadAutostart() {
    try {
      autostart = await isEnabled();
    } catch {
      /* plugin unavailable outside Tauri — leave toggle off */
    }
  }
  async function onAutostartToggle(e: Event) {
    const on = (e.target as HTMLInputElement).checked;
    try {
      if (on) await enable();
      else await disable();
      autostart = on;
    } catch (err) {
      error = String(err);
      autostart = !on;
    }
  }

  // ---- Category management (categories live inside Settings) ----
  async function addCategory() {
    if (!settings) return;
    const cats = settings.categories.slice();
    cats.push({
      id: "cat-" + Date.now().toString(36),
      name: "New category",
      extensions: [],
      directory: "",
    });
    await saveSettings({ categories: cats });
  }
  // Read an input value from a DOM event without an inline TS cast in markup.
  function targetValue(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }
  async function setCategoryField(
    id: string,
    field: "name" | "extensions" | "directory",
    value: string,
  ) {
    if (!settings) return;
    const cats = settings.categories.map((c) => {
      if (c.id !== id) return c;
      const next = { ...c };
      if (field === "name") next.name = value;
      else if (field === "directory") next.directory = value;
      else next.extensions = value.split(",").map((s) => s.trim()).filter(Boolean);
      return next;
    });
    await saveSettings({ categories: cats });
  }
  async function removeCategory(id: string) {
    if (!settings) return;
    const cats = settings.categories.filter((c) => c.id !== id);
    await saveSettings({ categories: cats });
  }

  // ---- Clipboard monitor (M4) ----
  function isUrl(s: string): boolean {
    return /^https?:\/\/\S+$/i.test(s.trim());
  }
  async function checkClipboard() {
    try {
      const t = await readText();
      if (t && isUrl(t)) clipboardUrl = t.trim();
    } catch {
      /* clipboard unavailable (non-Tauri context) — ignore */
    }
  }
  function startClipboardMonitor() {
    stopClipboardMonitor();
    clipTimer = setInterval(checkClipboard, 2000);
  }
  function stopClipboardMonitor() {
    if (clipTimer) clearInterval(clipTimer);
    clipTimer = null;
  }
  async function addClipboard() {
    if (!clipboardUrl) return;
    await add(clipboardUrl, newCategory || null);
    clipboardUrl = "";
  }

  // ---- Drag & drop (M4) ----
  async function onDrop(e: DragEvent) {
    e.preventDefault();
    const raw =
      e.dataTransfer?.getData("text/uri-list") ||
      e.dataTransfer?.getData("text/plain") ||
      "";
    const urls = raw
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter((l) => l && !l.startsWith("#"))
      .filter(isUrl);
    for (const u of urls) await add(u, newCategory || null);
    if (urls.length) await refresh();
  }
  function onDragOver(e: DragEvent) {
    e.preventDefault();
  }

  onMount(() => {
    refresh();
    loadSettings().then(() => {
      if (settings?.clipboardMonitor) startClipboardMonitor();
    });
    loadAutostart();
    startSpeedMonitor();
    const unlisten: Promise<UnlistenFn> = listen<FrontendEvent>(
      "download-event",
      (ev) => {
        const e = ev.payload;
        if (e.kind === "removed") {
          downloads = downloads.filter((d) => d.id !== e.download.id);
          return;
        }
        const d = e.download;
        const i = downloads.findIndex((x) => x.id === d.id);
        if (i >= 0) {
          downloads[i] = d;
          downloads = downloads.slice();
        } else {
          downloads = [d, ...downloads];
        }
      },
    );
    return () => {
      unlisten?.then((f) => f()).catch(() => {});
      stopClipboardMonitor();
      stopSpeedMonitor();
    };
  });

  function pct(d: Download): string {
    if (!d.totalSize) return d.canResume ? "?" : "0%";
    return `${(d.downloaded / d.totalSize * 100).toFixed(1)}%`;
  }
  function fmt(n: number | null): string {
    if (n == null) return "—";
    if (n >= 1e9) return `${(n / 1e9).toFixed(2)} GB`;
    if (n >= 1e6) return `${(n / 1e6).toFixed(2)} MB`;
    if (n >= 1e3) return `${(n / 1e3).toFixed(1)} KB`;
    return n + " B";
  }
</script>

<main on:dragover={onDragOver} on:drop={onDrop}>
  <h1>DM — Download Manager</h1>
  <div class="sub">
    Segmented downloads · pause/resume · speed limits · drag &amp; drop a URL to add
  </div>

  <div class="card">
    <div class="row">
      <button class="ghost" on:click={doPing}>Ping backend</button>
      <span class="pill">{pingResult || "—"}</span>
      <span style="flex:1"></span>
      <button class="ghost" on:click={() => (showSettings = !showSettings)}>
        {showSettings ? "Hide settings" : "Settings"}
      </button>
    </div>
  </div>

  {#if showSettings && settings}
    <div class="card">
      <div class="row" style="margin-bottom:8px">
        <strong>Scheduler</strong>
        <label class="pill">
          <input
            type="checkbox"
            checked={settings.scheduleEnabled}
            on:change={onScheduleToggle}
          />
          enabled
        </label>
        <span>from</span>
        <input
          type="time"
          style="flex:0 1 120px"
          value={fmtTime(settings.scheduleStart)}
          on:change={onStartChange}
        />
        <span>to</span>
        <input
          type="time"
          style="flex:0 1 120px"
          value={fmtTime(settings.scheduleEnd)}
          on:change={onEndChange}
        />
      </div>
      <div class="row" style="margin-bottom:8px">
        <strong>Clipboard monitor</strong>
        <label class="pill">
          <input
            type="checkbox"
            checked={settings.clipboardMonitor}
            on:change={onClipboardToggle}
          />
          auto-detect URLs
        </label>
      </div>
      <div class="row">
        <strong>Proxy</strong>
        <input
          style="flex:1"
          placeholder="http://user:pass@host:port or socks5://host:port"
          value={settings.proxy ?? ""}
          on:change={onProxyChange}
        />
      </div>
      <div class="row" style="margin-bottom:8px">
        <strong>Launch at login</strong>
        <label class="pill">
          <input type="checkbox" checked={autostart} on:change={onAutostartToggle} />
          start DM on system login
        </label>
      </div>
      <div class="row" style="margin-bottom:8px">
        <strong>Categories</strong>
        <button class="ghost" on:click={addCategory}>Add category</button>
      </div>
      {#each settings.categories as c (c.id)}
        <div class="row" style="margin-bottom:6px; align-items:flex-start">
          <input
            style="flex:1"
            value={c.name}
            placeholder="name"
            on:change={(e) => setCategoryField(c.id, "name", targetValue(e))}
          />
          <input
            style="flex:1.4"
            value={c.extensions.join(", ")}
            placeholder="extensions, comma-separated"
            on:change={(e) => setCategoryField(c.id, "extensions", targetValue(e))}
          />
          <input
            style="flex:1.4"
            value={c.directory}
            placeholder="save directory"
            on:change={(e) => setCategoryField(c.id, "directory", targetValue(e))}
          />
          <button class="danger" on:click={() => removeCategory(c.id)}>Del</button>
        </div>
      {/each}
    </div>
  {/if}

  <div class="card">
    <div class="row">
      <input
        placeholder="https://example.com/big-file.iso"
        bind:value={url}
        on:keydown={(e) => e.key === "Enter" && add(url, newCategory || null, currentChecksum())}
      />
      <input
        style="flex: 0 1 160px"
        placeholder="category (optional)"
        bind:value={newCategory}
      />
      <button on:click={() => add(url, newCategory || null, currentChecksum())}>Add URL</button>
    </div>
    <div class="row" style="margin-top:10px">
      <select bind:value={checksumAlgo} style="flex:0 1 140px">
        <option value="sha256">sha256</option>
      </select>
      <input
        style="flex:1"
        placeholder="expected checksum (optional, hex)"
        bind:value={checksumExpected}
      />
    </div>
    <div class="row" style="margin-top:10px">
      <input
        style="flex: 0 1 200px"
        type="number"
        min="0"
        placeholder="global speed cap (bytes/s)"
        bind:value={globalLimit}
      />
      <button class="ghost" on:click={applyGlobalLimit}>Set global limit</button>
    </div>
    {#if clipboardUrl}
      <div class="row" style="margin-top:10px">
        <span class="pill">Clipboard: {clipboardUrl}</span>
        <button class="ghost" on:click={addClipboard}>Add clipboard URL</button>
      </div>
    {/if}
    {#if error}
      <div class="err">{error}</div>
    {/if}
  </div>

  {#each downloads as d (d.id)}
    <div class="card">
      <div class="top">
        <div>
          <div class="name">{d.filename || d.url}</div>
          <div class="url">{d.url}</div>
        </div>
        <div class="status {d.status}">{d.status}</div>
      </div>

      <div class="bar">
        <div class="fill" style="width:{pct(d)}"></div>
      </div>
      <div class="meta">
        <span>{fmt(d.downloaded)}{d.totalSize ? " / " + fmt(d.totalSize) : ""}</span>
        <span>{pct(d)}</span>
      </div>
      {#if d.error}
        <div class="err">{d.error}</div>
      {/if}

      <div class="actions">
        {#if d.status === "downloading" || d.status === "queued" || d.status === "connecting"}
          <button class="ghost" on:click={() => pause(d)}>Pause</button>
          <button class="danger" on:click={() => cancel(d)}>Cancel</button>
        {:else if d.status === "paused"}
          <button on:click={() => resume(d)}>Resume</button>
        {:else if d.status === "scheduled"}
          <span class="pill">waiting for schedule window</span>
        {/if}
        <button class="ghost" on:click={() => remove(d)}>Remove</button>
      </div>
    </div>
  {/each}

  <div class="card">
    <SpeedGraph speed={aggregateSpeed} />
  </div>

  {#if downloads.length === 0}
    <div class="sub">No downloads yet — paste a URL above or drop one in.</div>
  {/if}
</main>
