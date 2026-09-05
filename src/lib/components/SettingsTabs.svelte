<script lang="ts">
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import type { ProxyMode, Settings, Category } from "../types";
  import { settings, updateSettings, setProxyMode } from "../stores/settings";
  import { showToast, theme, setTheme, extensionStatus } from "../stores/ui";
  import * as api from "../api";

  // Poll the native-messaging host status while the Settings panel is open.
  let probeTimer: ReturnType<typeof setInterval> | null = null;

  // Static build / runtime info for the About panel. Loaded once
  // on mount — the values are baked into the binary at compile
  // time so there's no point re-fetching them. We initialise to
  // empty strings and replace them on the first successful
  // `api.appInfo()` call. The "loading" state is implicit (empty
  // strings); once the user opens the About tab we re-render
  // with the real values.
  let appVersion = "";
  let engineVersion = "";
  let tauriVersion = "";
  let appInfoError: string | null = null;
  async function refreshAppInfo() {
    try {
      const info = await api.appInfo();
      appVersion = info.appVersion;
      engineVersion = info.engineVersion;
      tauriVersion = info.tauriVersion;
      appInfoError = null;
    } catch (e) {
      // Don't overwrite the version strings on a transient
      // failure — if the very first call fails, the user will
      // see "—" placeholders, which is honest about the state
      // without hiding the real error.
      appInfoError = String(e);
    }
  }

  async function refreshExtStatus() {
    if (typeof window === "undefined") return;
    try {
      const probe = await api.probeNativeHost();
      extensionStatus.update((s) => ({
        ...s,
        kind: probe.bound ? "ok" : "offline",
        label: probe.bound ? "Ready" : "Not listening",
        port: probe.port,
        lastActivity:
          probe.lastEventUnix > 0
            ? new Date(probe.lastEventUnix * 1000).toLocaleString()
            : "Waiting for first capture…",
        lastError: null,
      }));
    } catch (e) {
      extensionStatus.update((s) => ({
        ...s,
        kind: "error",
        label: "Error",
        lastError: String(e),
      }));
    }
  }

  interface ProxyOption {
    key: ProxyMode;
    label: string;
    desc: string;
    icon: string;
  }
  const PROXY_MODES: ProxyOption[] = [
    {
      key: "none",
      label: "No proxy",
      desc: "Always connect directly.",
      icon: "M18 6L6 18M6 6l12 12",
    },
    {
      key: "system",
      label: "System proxy",
      desc: "Read HTTP_PROXY / HTTPS_PROXY env vars.",
      icon: "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zM2 12h20M12 2a16 16 0 0 1 0 20M12 2a16 16 0 0 0 0 20",
    },
    {
      key: "manual",
      label: "Manual",
      desc: "Specify a single proxy URL.",
      icon: "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8l-6-6zM14 2v6h6M9 13l2 2 4-4",
    },
  ];

  // Local mirror of the URL field so typing isn't laggy; the Tauri command
  // is only invoked on commit.
  let proxyUrl = "";
  $: if ($settings) {
    // Sync from store when not actively editing.
    proxyUrl = $settings?.proxy ?? "";
  }
  $: proxyMode = ($settings?.proxyMode ?? "system") as ProxyMode;

  async function onProxyUrlChange(value: string) {
    const cleaned = value.trim();
    await setProxyMode("manual", cleaned || null);
  }

  let proxyUrlInput: HTMLInputElement | null = null;

  // When the user runs "Proxy: manual…" from the command palette, switch to
  // the Network tab and focus the URL field. The window event is dispatched
  // by App.svelte (dynamic import → cleaner than threading a prop).
  import { onMount, onDestroy } from "svelte";
  function focusProxyUrlFromPalette() {
    activeTab = "network";
    // Wait for the tab to render before focusing.
    requestAnimationFrame(() => proxyUrlInput?.focus());
  }
  onMount(() => {
    if (typeof window !== "undefined") {
      window.addEventListener("dm:focus-proxy-url", focusProxyUrlFromPalette as EventListener);
    }
    refreshAppInfo();
    refreshExtStatus();
    probeTimer = setInterval(refreshExtStatus, 3000);
  });
  onDestroy(() => {
    if (typeof window !== "undefined") {
      window.removeEventListener("dm:focus-proxy-url", focusProxyUrlFromPalette as EventListener);
    }
    if (probeTimer) clearInterval(probeTimer);
  });

  export let onClose: () => void = () => {};

  type Tab = "general" | "network" | "scheduler" | "appearance" | "categories" | "extensions" | "advanced" | "about";
  let activeTab: Tab = "general";

  const TABS: Array<{ key: Tab; label: string; icon: string }> = [
    { key: "general", label: "General", icon: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-2.82 1.17V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 8 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 3.6 15H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6h.09A1.65 1.65 0 0 0 11 3.09V3a2 2 0 1 1 4 0v.09A1.65 1.65 0 0 0 16 4.6a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 20.4 9v.09A1.65 1.65 0 0 0 22 11h0a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" },
    { key: "network", label: "Network", icon: "M1 1l22 22M16.72 11.06A10.94 10.94 0 0 1 19 12.55M5 12.55a10.94 10.94 0 0 1 5.17-2.39M10.71 5.05A16 16 0 0 1 22.58 9M1.42 9a15.91 15.91 0 0 1 4.7-2.88M8.53 16.11a6 6 0 0 1 6.95 0M12 20h.01" },
    { key: "scheduler", label: "Scheduler", icon: "M8 2v4M16 2v4M3 10h18M5 4h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z" },
    { key: "appearance", label: "Appearance", icon: "M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" },
    { key: "categories", label: "Categories", icon: "M3 7l4-4h4l2 2h8v13H3V7z" },
    { key: "extensions", label: "Extensions", icon: "M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" },
    { key: "advanced", label: "Advanced", icon: "M12 1l3 6 6 1-4.5 4.5 1 6L12 16l-5.5 2.5 1-6L3 8l6-1 3-6z" },
    { key: "about", label: "About", icon: "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zM12 16v-4M12 8h.01" },
  ];

  function pad2(n: number): string { return n.toString().padStart(2, "0"); }
  function fmtTime(t: [number, number]): string { return `${pad2(t[0])}:${pad2(t[1])}`; }
  function parseTime(s: string): [number, number] {
    const [h, m] = s.split(":").map((x) => Number(x) || 0);
    return [h, m];
  }

  async function save(patch: Partial<Settings>) {
    try {
      await updateSettings(patch);
      showToast({ kind: "success", title: "Settings saved", duration: 1500 });
    } catch (e) {
      showToast({ kind: "error", title: "Save failed", message: String(e) });
    }
  }

  function toggleCloseToTray() {
    if (!$settings) return;
    save({ closeToTray: !$settings.closeToTray });
  }
  function toggleScheduler() {
    if (!$settings) return;
    save({ scheduleEnabled: !$settings.scheduleEnabled });
  }

  async function addCategory() {
    if (!$settings) return;
    const cats = $settings.categories.slice();
    cats.push({
      id: "cat-" + Date.now().toString(36),
      name: "New category",
      extensions: [],
      directory: "",
    });
    await save({ categories: cats });
  }

  async function updateCategory(id: string, field: keyof Category, value: any) {
    if (!$settings) return;
    const cats = $settings.categories.map((c) => {
      if (c.id !== id) return c;
      const next = { ...c };
      if (field === "extensions") next.extensions = value.split(",").map((s: string) => s.trim()).filter(Boolean);
      else (next as any)[field] = value;
      return next;
    });
    await save({ categories: cats });
  }

  async function removeCategory(id: string) {
    if (!$settings) return;
    await save({ categories: $settings.categories.filter((c) => c.id !== id) });
    showToast({ kind: "info", title: "Category removed" });
  }
</script>

<div class="drawer" transition:fly={{ x: 420, duration: 320, easing: cubicOut }}>
  <div class="drawer-head">
    <h2>Settings</h2>
    <button class="close-btn" on:click={onClose} aria-label="Close">
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2"><path d="M6 6l12 12M18 6L6 18"/></svg>
    </button>
  </div>

  <div class="drawer-body">
    <nav class="tabs" aria-label="Settings tabs">
      {#each TABS as tab}
        <button
          class="tab"
          class:active={activeTab === tab.key}
          on:click={() => (activeTab = tab.key)}
          role="tab"
          aria-selected={activeTab === tab.key}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d={tab.icon}/>
          </svg>
          <span>{tab.label}</span>
        </button>
      {/each}
    </nav>

    <div class="tab-content">
      {#if !$settings}
        <div class="empty">Settings unavailable in preview mode.</div>
      {:else if activeTab === "general"}
        <section class="set-section">
          <h3>Defaults</h3>
          <div class="field">
            <label for="default-dir">Default save location</label>
            <input id="default-dir" type="text" value={$settings.defaultDirectory} on:change={(e) => save({ defaultDirectory: e.currentTarget.value })} />
          </div>
          <div class="field-row">
            <div class="field">
              <label for="max-concurrent">Max concurrent downloads</label>
              <input id="max-concurrent" type="number" min="1" max="32" value={$settings.maxConcurrentDownloads} on:change={(e) => save({ maxConcurrentDownloads: Number(e.currentTarget.value) || 1 })} />
              <span class="hint">How many files transfer in parallel</span>
            </div>
            <div class="field">
              <label for="connections">Connections per file</label>
              <input id="connections" type="number" min="1" max="32" value={$settings.connectionsPerDownload} on:change={(e) => save({ connectionsPerDownload: Number(e.currentTarget.value) || 1 })} />
              <span class="hint">Range segments for parallel download</span>
            </div>
          </div>
        </section>

        <section class="set-section">
          <h3>Behavior</h3>
          <div class="set-line">
            <div>
              <div class="lbl">Close to system tray</div>
              <div class="hint">Keep running in background when window closes</div>
            </div>
            <button
              class="switch"
              class:on={$settings.closeToTray}
              aria-pressed={$settings.closeToTray}
              on:click={toggleCloseToTray}
            >
              <span class="knob"></span>
            </button>
          </div>
        </section>

      {:else if activeTab === "network"}
        <section class="set-section">
          <h3>Proxy</h3>
          <p class="hint">Choose how DM reaches the internet. Per-download overrides still win over the global setting.</p>
          <div class="proxy-grid">
            {#each PROXY_MODES as m}
              <button
                class="proxy-card"
                class:active={proxyMode === m.key}
                on:click={() => setProxyMode(m.key)}
                aria-pressed={proxyMode === m.key}
              >
                <svg class="proxy-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                  <path d={m.icon} />
                </svg>
                <span class="proxy-name">{m.label}</span>
                <span class="proxy-desc">{m.desc}</span>
              </button>
            {/each}
          </div>

          <div class="field" style="margin-top: 16px">
            <label for="proxy-url">Manual proxy URL</label>
            <input
              id="proxy-url"
              type="text"
              placeholder="http://user:pass@host:port · socks5://host:1080"
              value={proxyUrl}
              on:change={(e) => onProxyUrlChange(e.currentTarget.value)}
              disabled={proxyMode !== "manual"}
              bind:this={proxyUrlInput}
            />
            <span class="hint">
              {#if proxyMode === "manual"}
                Used for every download that does not set its own proxy.
              {:else}
                Switch to <em>Manual</em> to edit.
              {/if}
            </span>
          </div>
        </section>

      {:else if activeTab === "scheduler"}
        <section class="set-section">
          <h3>Timed downloads</h3>
          <div class="set-line">
            <div>
              <div class="lbl">Enable scheduler</div>
              <div class="hint">Only transfer during this window</div>
            </div>
            <button
              class="switch"
              class:on={$settings.scheduleEnabled}
              aria-pressed={$settings.scheduleEnabled}
              on:click={toggleScheduler}
            >
              <span class="knob"></span>
            </button>
          </div>
          <div class="field-row">
            <div class="field">
              <label for="sched-start">From</label>
              <input id="sched-start" type="time" value={fmtTime($settings.scheduleStart)} on:change={(e) => save({ scheduleStart: parseTime(e.currentTarget.value) })} />
            </div>
            <div class="field">
              <label for="sched-end">To</label>
              <input id="sched-end" type="time" value={fmtTime($settings.scheduleEnd)} on:change={(e) => save({ scheduleEnd: parseTime(e.currentTarget.value) })} />
            </div>
          </div>
        </section>

      {:else if activeTab === "categories"}
        <section class="set-section">
          <h3>Categories ({$settings.categories.length})</h3>
          <p class="hint">Auto-route downloads to folders by extension.</p>
          <div class="cat-list">
            {#each $settings.categories as c (c.id)}
              <div class="cat-row">
                <input value={c.name} placeholder="Name" on:change={(e) => updateCategory(c.id, "name", e.currentTarget.value)} />
                <input value={c.extensions.join(", ")} placeholder="ext, comma" on:change={(e) => updateCategory(c.id, "extensions", e.currentTarget.value)} />
                <input value={c.directory} placeholder="Save directory" on:change={(e) => updateCategory(c.id, "directory", e.currentTarget.value)} />
                <button class="del-btn" on:click={() => removeCategory(c.id)} aria-label="Delete">
                  <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/></svg>
                </button>
              </div>
            {/each}
          </div>
          <button class="add-btn" on:click={addCategory}>+ Add category</button>
        </section>

      {:else if activeTab === "appearance"}
        <section class="set-section">
          <h3>Appearance</h3>
          <p class="hint">Choose how DM looks on this device.</p>
          <div class="theme-grid">
            <button
              class="theme-card"
              class:active={$theme === "dark"}
              on:click={() => setTheme("dark")}
              aria-pressed={$theme === "dark"}
            >
              <div class="preview preview-dark">
                <span class="dot"></span>
                <span class="bar"></span>
                <span class="bar"></span>
              </div>
              <span class="t-label">Dark</span>
            </button>
            <button
              class="theme-card"
              class:active={$theme === "light"}
              on:click={() => setTheme("light")}
              aria-pressed={$theme === "light"}
            >
              <div class="preview preview-light">
                <span class="dot"></span>
                <span class="bar"></span>
                <span class="bar"></span>
              </div>
              <span class="t-label">Light</span>
            </button>
            <button
              class="theme-card"
              class:active={$theme === "system"}
              on:click={() => setTheme("system")}
              aria-pressed={$theme === "system"}
            >
              <div class="preview preview-split">
                <span class="dot"></span>
                <span class="bar"></span>
                <span class="bar"></span>
              </div>
              <span class="t-label">System</span>
            </button>
          </div>
        </section>

      {:else if activeTab === "extensions"}
        <section class="set-section">
          <h3>Browser extension</h3>
          <p class="hint">
            The MV3 extension captures downloads from your browser and
            forwards them to DM. Chromium-based browsers (Chrome, Edge,
            Brave, Arc, Vivaldi, Opera) talk to DM directly over WebSocket
            on <code>ws://127.0.0.1:9157/</code> — no host binary, no
            manifest copy, no registry edits. Firefox still needs a small
            <code>dm-native-host</code> shim because Firefox MV3 cannot
            reliably open <code>ws://127.0.0.1</code> connections (Firefox
            upgrades insecure <code>ws://</code> to <code>wss://</code> and
            the connection fails silently).
          </p>

          <div class="ext-status">
            <div class="ext-row">
              <div class="ext-name">Listener</div>
              <div class="ext-state">
                <span class="status-pill {$extensionStatus.kind}">
                  <span class="dot"></span>
                  {$extensionStatus.label}
                </span>
              </div>
            </div>
            <div class="ext-row">
              <div class="ext-name">Listening port</div>
              <div class="ext-state mono">127.0.0.1:{$extensionStatus.port}</div>
            </div>
            <div class="ext-row">
              <div class="ext-name">Last activity</div>
              <div class="ext-state">{$extensionStatus.lastActivity}</div>
            </div>
            {#if $extensionStatus.lastError}
              <div class="ext-row">
                <div class="ext-name">Error</div>
                <div class="ext-state danger">{$extensionStatus.lastError}</div>
              </div>
            {/if}
          </div>

          <div class="set-group">
            <h4>Install on Chrome / Edge / Brave / Arc / Vivaldi / Opera</h4>
            <p class="hint">Three steps, no native binary, no manifest copy.</p>
            <ol class="steps">
              <li>
                Download <code>dm-grabber-&lt;version&gt;.zip</code> from the
                latest GitHub release and unzip it anywhere.
              </li>
              <li>
                Open <code>chrome://extensions</code> (or
                <code>brave://extensions</code>, <code>edge://extensions</code>,
                <code>arc://extensions</code>, <code>vivaldi://extensions</code>,
                <code>opera://extensions</code>).
              </li>
              <li>
                Toggle <b>Developer mode</b> (top right) → click
                <b>Load unpacked</b> → pick the unzipped
                <code>browser-extension/</code> folder.
              </li>
            </ol>
            <p class="hint">
              The extension auto-reconnects to the running DM app. There
              is no extension ID to copy and no native host to install.
            </p>
          </div>

          <div class="set-group">
            <h4>Install on Firefox 109+</h4>
            <p class="hint">
              Firefox needs the small <code>dm-native-host</code> shim
              binary because it can't open <code>ws://127.0.0.1</code>
              connections.
            </p>
            <ol class="steps">
              <li>
                Build the native host (or download it from the release):
                <code>cargo build -p dm-native-host --release</code> →
                <code>target/release/dm-native-host</code>.
              </li>
              <li>
                Place the Firefox host manifest on disk:
                <pre class="mono-block">mkdir -p ~/.mozilla/native-messaging-hosts
cp browser-extension/com.app.dm.native.firefox.json \
   ~/.mozilla/native-messaging-hosts/com.app.dm.native.json</pre>
                and edit the <code>path</code> to point at the binary
                from step 1.
              </li>
              <li>
                Open <code>about:debugging#/runtime/this-firefox</code>
                → <b>Load Temporary Add-on…</b> → select
                <code>browser-extension/manifest.json</code>. The
                manifest's <code>browser_specific_settings.gecko</code>
                block fixes the extension ID to
                <code>dm-grabber@dm-project</code>, so no ID copy-paste
                is needed.
              </li>
            </ol>
            <p class="hint">
              Temporary add-ons are erased on browser restart. For a
              permanent install, package the extension as
              <code>.xpi</code> (<code>./scripts/package.sh &lt;version&gt;</code>
              in the repo) and install via <code>about:addons</code>.
            </p>
          </div>
        </section>

      {:else if activeTab === "advanced"}
        <section class="set-section">
          <h3>Data</h3>
          <div class="set-line">
            <div>
              <div class="lbl">Export settings</div>
              <div class="hint">Save to a JSON file</div>
            </div>
            <button class="btn btn-ghost btn-sm">Export</button>
          </div>
          <div class="set-line">
            <div>
              <div class="lbl">Import settings</div>
              <div class="hint">Load from a JSON file</div>
            </div>
            <button class="btn btn-ghost btn-sm">Import</button>
          </div>
          <div class="set-line">
            <div>
              <div class="lbl">Reset to defaults</div>
              <div class="hint">This cannot be undone</div>
            </div>
            <button class="btn btn-danger btn-sm">Reset</button>
          </div>
        </section>

      {:else if activeTab === "about"}
        <section class="set-section">
          <h3>DM Download Manager</h3>
          <p class="hint">A modern, graceful, cross-platform download manager — Tauri 2 + Svelte.</p>
          <div class="about-grid">
            <div>
              <span class="k">Version</span>
              <span class="v">{appVersion || "—"}</span>
            </div>
            <div>
              <span class="k">Engine</span>
              <span class="v">dm-engine {engineVersion || "—"}</span>
            </div>
            <div>
              <span class="k">Tauri runtime</span>
              <span class="v">{tauriVersion || "—"}</span>
            </div>
            <div>
              <span class="k">Frontend</span>
              <span class="v">Svelte 4 + Vite</span>
            </div>
          </div>
          {#if appInfoError}
            <p class="hint danger">
              Could not load build info: {appInfoError}
            </p>
          {/if}
        </section>
      {/if}
    </div>
  </div>
</div>

<style>
  .drawer {
    position: fixed;
    top: 0;
    right: 0;
    z-index: var(--z-modal);
    height: 100vh;
    width: min(720px, 96vw);
    background: var(--color-bg-elevated);
    border-left: 1px solid var(--color-border);
    box-shadow: -30px 0 80px -30px var(--shadow-modal);
    color: var(--color-text);
    display: flex;
    overflow: hidden;
  }
  .drawer-head {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 18px 22px;
    border-bottom: 1px solid var(--color-border);
    background: var(--color-bg-elevated);
    backdrop-filter: blur(12px);
    z-index: 2;
  }
  .drawer-head h2 {
    font-family: var(--font-display);
    font-weight: 500;
    font-size: 22px;
    margin: 0;
  }
  .close-btn {
    background: transparent;
    border: 1px solid var(--hair);
    border-radius: 8px;
    color: var(--muted);
    cursor: pointer;
    padding: 7px;
    display: grid;
    place-items: center;
    transition: all 0.15s;
  }
  .close-btn:hover {
    color: var(--text);
    border-color: var(--border-strong);
    background: rgba(255, 255, 255, 0.04);
  }

  .drawer-body {
    display: grid;
    grid-template-columns: 200px 1fr;
    width: 100%;
    margin-top: 60px;
    overflow: hidden;
  }

  .tabs {
    display: flex;
    flex-direction: column;
    padding: 16px 10px;
    border-right: 1px solid var(--color-border);
    background: var(--color-surface);
    overflow-y: auto;
    /* When the user reaches the top or bottom of the left-nav scroll
       region, the browser forwards the wheel event to the next
       scrollable ancestor — which, without this, is the page behind
       the drawer. `overscroll-behavior: contain` makes the browser
       consume the event at the drawer's boundary instead, so the
       home view stays put. */
    overscroll-behavior: contain;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 12px;
    background: none;
    border: none;
    border-radius: 8px;
    color: var(--muted);
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    text-align: left;
    transition: all 0.15s;
  }
  .tab:hover {
    color: var(--text);
    background: rgba(255, 255, 255, 0.04);
  }
  .tab.active {
    color: var(--accent);
    background: var(--color-primary-dim);
  }
  .tab svg {
    width: 15px;
    height: 15px;
    flex: none;
  }

  .tab-content {
    padding: 22px 26px;
    overflow-y: auto;
    /* Same reasoning as `.tabs` — keep wheel momentum inside the
       drawer instead of letting it leak through to the home view
       underneath. */
    overscroll-behavior: contain;
  }

  .set-section {
    margin-bottom: 24px;
  }
  .set-section h3 {
    font-size: 14px;
    font-weight: 600;
    margin: 0 0 12px;
    color: var(--color-text);
  }

  .proxy-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 10px;
    margin-top: 12px;
  }
  .proxy-card {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: 14px 12px 12px;
    cursor: pointer;
    color: var(--color-text);
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    transition: all 0.15s var(--easing-out);
    text-align: left;
    font-family: inherit;
  }
  .proxy-card:hover {
    border-color: var(--color-border-strong);
    background: var(--color-surface-hover);
    transform: translateY(-1px);
  }
  .proxy-card.active {
    border-color: var(--color-primary);
    background: var(--color-primary-dim);
    box-shadow: 0 0 0 1px var(--color-primary-dim) inset;
  }
  .proxy-ico {
    width: 18px;
    height: 18px;
    color: var(--color-muted);
    margin-bottom: 2px;
  }
  .proxy-card.active .proxy-ico {
    color: var(--color-primary);
  }
  .proxy-name {
    font-size: 13px;
    font-weight: 600;
  }
  .proxy-desc {
    font-size: 11.5px;
    color: var(--color-faint);
    line-height: 1.4;
  }
  .proxy-card.active .proxy-desc {
    color: var(--color-muted);
  }

  .theme-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 10px;
    margin-top: 12px;
  }
  .theme-card {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    padding: 10px 8px 8px;
    cursor: pointer;
    color: var(--color-text);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    transition: all 0.15s;
    font-family: inherit;
  }
  .theme-card:hover {
    border-color: var(--color-border-strong);
    background: var(--color-surface-hover);
  }
  .theme-card.active {
    border-color: var(--color-primary);
    background: var(--color-primary-dim);
  }
  .preview {
    width: 100%;
    height: 60px;
    border-radius: 6px;
    border: 1px solid var(--color-border-2);
    display: flex;
    flex-direction: column;
    align-items: stretch;
    padding: 6px;
    gap: 4px;
    overflow: hidden;
  }
  .preview .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    align-self: flex-start;
  }
  .preview .bar {
    height: 4px;
    border-radius: 2px;
    align-self: stretch;
  }
  .preview-dark { background: #0b0d12; }
  .preview-dark .dot { background: #7af0c8; }
  .preview-dark .bar:first-of-type { background: #2a313e; width: 60%; }
  .preview-dark .bar:last-of-type { background: #1b2230; width: 80%; }
  .preview-light { background: #f6f7f9; }
  .preview-light .dot { background: #0f8a63; }
  .preview-light .bar:first-of-type { background: #cbd1da; width: 60%; }
  .preview-light .bar:last-of-type { background: #dee3eb; width: 80%; }
  .preview-split {
    background: linear-gradient(135deg, #f6f7f9 50%, #0b0d12 50%);
  }
  .preview-split .dot { background: #2d5fda; }
  .preview-split .bar:first-of-type { background: linear-gradient(90deg, #cbd1da 50%, #2a313e 50%); width: 60%; }
  .preview-split .bar:last-of-type { background: linear-gradient(90deg, #dee3eb 50%, #1b2230 50%); width: 80%; }
  .t-label {
    font-size: 12px;
    font-weight: 600;
  }

  .set-line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    margin-bottom: 12px;
  }
  .lbl { font-size: 13.5px; color: var(--text); font-weight: 500; }
  .hint { font-size: 11.5px; color: var(--text-faint); margin-top: 2px; line-height: 1.4; }

  .field { margin-bottom: 14px; }
  .field label {
    display: block;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin-bottom: 6px;
  }
  .field input {
    width: 100%;
    height: 36px;
  }
  .field-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    /* Each cell uses an identical 4-row template (label / input / hint /
       spacer) so a 2-line label on the left doesn't make the whole
       left card taller than the right card. */
    align-items: stretch;
  }
  /* Reserve space for the wrapped label so both columns share the
     same vertical rhythm. `min-height: 2lh` keeps two line-heights of
     headroom for "Max concurrent downloads" while the right column's
     "Connections per file" simply takes the same space. */
  .field-row .field label {
    min-height: 2lh;
  }

  .switch {
    position: relative;
    width: 40px;
    height: 22px;
    border-radius: 99px;
    background: rgba(255, 255, 255, 0.12);
    border: 1px solid var(--hair);
    cursor: pointer;
    transition: all 0.2s;
    flex: none;
  }
  .switch.on {
    background: linear-gradient(150deg, var(--accent), #4fcf9f);
    border-color: transparent;
  }
  .knob {
    position: absolute;
    top: 1px;
    left: 1px;
    width: 18px;
    height: 18px;
    border-radius: 99px;
    background: #fff;
    transition: all 0.2s;
    box-shadow: 0 2px 5px rgba(0, 0, 0, 0.4);
  }
  .switch.on .knob { left: 19px; }

  .cat-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .cat-row {
    display: grid;
    grid-template-columns: 1.1fr 1.3fr 1.5fr auto;
    gap: 6px;
    align-items: center;
  }
  .cat-row input {
    padding: 7px 9px;
    font-size: 12.5px;
  }
  .del-btn {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    background: var(--color-danger-dim);
    border: 1px solid var(--color-danger-strong);
    border-radius: 7px;
    color: var(--color-danger);
    cursor: pointer;
    transition: all 0.15s;
  }
  .del-btn:hover {
    background: color-mix(in srgb, var(--color-danger) 20%, transparent);
  }
  .add-btn {
    margin-top: 10px;
    padding: 7px 14px;
    background: transparent;
    border: 1px dashed var(--border-strong);
    border-radius: 8px;
    color: var(--muted);
    font-size: 12.5px;
    cursor: pointer;
    font-family: inherit;
    transition: all 0.15s;
  }
  .add-btn:hover {
    color: var(--accent);
    border-color: var(--color-primary-strong);
    background: var(--color-primary-dim);
  }

  .steps {
    margin: 12px 0;
    padding-left: 20px;
    color: var(--muted);
    font-size: 12.5px;
    line-height: 1.7;
  }
  .steps code {
    font-family: var(--font-mono);
    font-size: 11.5px;
    background: rgba(255, 255, 255, 0.05);
    padding: 1px 5px;
    border-radius: 4px;
    color: var(--accent);
  }
  .mono-block {
    font-family: var(--font-mono);
    font-size: 11.5px;
    background: rgba(0, 0, 0, 0.25);
    color: var(--text);
    padding: 10px 12px;
    border-radius: 6px;
    margin: 6px 0 4px;
    overflow-x: auto;
    white-space: pre;
  }
  .status-pill {
    display: inline-block;
    padding: 4px 12px;
    border-radius: 99px;
    font-size: 11.5px;
    font-weight: 600;
    background: rgba(255, 255, 255, 0.05);
    color: var(--muted);
  }
  .status-pill.ok {
    background: var(--color-success-dim);
    color: var(--color-success);
  }

  .ext-status {
    display: grid;
    gap: 1px;
    background: var(--color-border-2);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    overflow: hidden;
    margin: 12px 0 18px;
  }
  .ext-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 14px;
    background: var(--color-surface);
  }
  .ext-name {
    font-size: 12.5px;
    color: var(--color-muted);
    font-weight: 500;
  }
  .ext-state {
    font-size: 12.5px;
    color: var(--color-text);
    text-align: right;
    max-width: 65%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ext-state.mono {
    font-family: var(--font-mono);
    font-size: 11.5px;
  }
  .ext-state.danger {
    color: var(--color-danger);
  }
  .status-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .status-pill .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: currentColor;
    box-shadow: 0 0 6px currentColor;
  }
  .status-pill.offline {
    background: var(--color-warning-dim);
    color: var(--color-warning);
  }
  .status-pill.error {
    background: var(--color-danger-dim);
    color: var(--color-danger);
  }
  .status-pill.starting {
    background: var(--color-info-dim);
    color: var(--color-info);
  }
  .set-group h4 {
    margin: 16px 0 8px;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--color-text);
  }

  .about-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px 16px;
    margin-top: 8px;
  }
  .about-grid > div {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .about-grid .k {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-faint);
    font-weight: 600;
  }
  .about-grid .v {
    font-size: 13px;
    color: var(--text);
  }

  .empty {
    text-align: center;
    padding: 40px 20px;
    color: var(--text-faint);
  }
</style>