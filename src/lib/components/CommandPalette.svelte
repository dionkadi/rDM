<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { fade } from "svelte/transition";
  import { showCommandPalette } from "../stores/ui";
  import { downloads } from "../stores/downloads";
  import { fuzzySearch } from "../utils/fuzzySearch";
  import { fmtBytes } from "../utils/formatters";
  import { focusTrap } from "../utils/focusTrap";

  const dispatch = createEventDispatcher();

  interface Command {
    id: string;
    title: string;
    hint?: string;
    category: string;
    icon?: string;
    action: () => void;
  }

  let query = "";
  let selectedIndex = 0;
  let mode: "command" | "download" = "command";

  const ICONS = {
    command: "M8 9l3 3-3 3M16 9l-3 3 3 3",
    download: "M12 4v11m0 0l-4-4m4 4l4-4M5 20h14",
    play: "M5 4l14 8-14 8V4z",
    pause: "M8 5v14M16 5v14",
    trash: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13",
    settings: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
  };

  $: commands = getCommands();
  $: downloadResults = mode === "download" ? fuzzySearch($downloads, query, ["filename", "url"], 8) : [];
  $: results = mode === "command" ? fuzzySearch(commands.map((c) => ({ ...c, searchText: c.title + " " + c.category })), query, ["searchText"], 12) : [];

  function getCommands(): Command[] {
    return [
      { id: "add", title: "Add download", category: "Actions", icon: ICONS.download, action: () => dispatch("action", { type: "add" }) },
      { id: "pause-all", title: "Pause all", category: "Actions", icon: ICONS.pause, action: () => dispatch("action", { type: "pause-all" }) },
      { id: "resume-all", title: "Resume all", category: "Actions", icon: ICONS.play, action: () => dispatch("action", { type: "resume-all" }) },
      { id: "clear-completed", title: "Remove all completed", category: "Cleanup", icon: ICONS.trash, action: () => dispatch("action", { type: "clear-completed" }) },
      { id: "settings", title: "Open settings", category: "Navigation", icon: ICONS.settings, action: () => dispatch("action", { type: "settings" }) },
      { id: "proxy-none", title: "Proxy: disable", category: "Network", icon: "M18 6L6 18M6 6l12 12", action: () => dispatch("action", { type: "set-proxy", mode: "none" }) },
      { id: "proxy-system", title: "Proxy: use system", category: "Network", icon: "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zM2 12h20", action: () => dispatch("action", { type: "set-proxy", mode: "system" }) },
      { id: "proxy-manual", title: "Proxy: manual…", category: "Network", icon: "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8l-6-6z", action: () => dispatch("action", { type: "set-proxy", mode: "manual" }) },
      { id: "search-downloads", title: "Search downloads…", category: "Navigation", icon: "M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35", action: () => setMode("download") },
    ];
  }

  /** Switching modes changes the result set; a stale index would leave
   *  no highlighted row and make Enter a no-op. Reset it with the query. */
  function setMode(next: "command" | "download") {
    mode = next;
    query = "";
    selectedIndex = 0;
  }

  function close() {
    showCommandPalette.set(false);
    query = "";
    mode = "command";
    selectedIndex = 0;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (mode === "download") { setMode("command"); e.preventDefault(); }
      else close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      selectedIndex = Math.min(selectedIndex + 1, (mode === "command" ? results.length : downloadResults.length) - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      selectedIndex = Math.max(selectedIndex - 1, 0);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (mode === "command" && results[selectedIndex]) {
        results[selectedIndex].item.action();
        close();
      } else if (mode === "download" && downloadResults[selectedIndex]) {
        dispatch("select-download", { id: downloadResults[selectedIndex].item.id });
        close();
      }
    }
    // Tab is intentionally left alone: the explicit Commands / Downloads
    // buttons switch modes, and `use:focusTrap` keeps Tab inside the
    // dialog instead of silently repurposing it.
  }
</script>

{#if $showCommandPalette}
  <div class="backdrop" transition:fade={{ duration: 150 }} on:click={close} role="presentation"></div>
  <div class="palette" use:focusTrap transition:fade={{ duration: 120 }} role="dialog" aria-modal="true" aria-label="Command palette">
    <div class="header">
      <svg class="search-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35"/></svg>
      <input
        bind:value={query}
        on:keydown={handleKeydown}
        aria-label={mode === "command" ? "Type a command" : "Search downloads"}
        placeholder={mode === "command" ? "Type a command or search…" : "Search downloads…"}
        autocomplete="off"
        spellcheck="false"
      />
      <div class="modes" role="group" aria-label="Palette mode">
        <button
          class="mode"
          class:active={mode === "command"}
          on:click={() => setMode("command")}
          aria-pressed={mode === "command"}
        >Commands</button>
        <button
          class="mode"
          class:active={mode === "download"}
          on:click={() => setMode("download")}
          aria-pressed={mode === "download"}
        >Downloads</button>
      </div>
      <kbd class="esc">esc</kbd>
    </div>

    <div class="results">
      {#if mode === "command"}
        {#each results as r, i (r.item.id)}
          <button
            class="result"
            class:selected={i === selectedIndex}
            on:click={() => { r.item.action(); close(); }}
            on:mouseenter={() => (selectedIndex = i)}
          >
            {#if r.item.icon}
              <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d={r.item.icon}/></svg>
            {/if}
            <span class="title">{r.item.title}</span>
            <span class="cat">{r.item.category}</span>
          </button>
        {/each}
        {#if results.length === 0}
          <div class="empty">No commands match "{query}"</div>
        {/if}
      {:else}
        {#each downloadResults as r, i (r.item.id)}
          <button
            class="result"
            class:selected={i === selectedIndex}
            on:click={() => { dispatch("select-download", { id: r.item.id }); close(); }}
            on:mouseenter={() => (selectedIndex = i)}
          >
            <div class="dl-info">
              <div class="dl-name">{r.item.filename || r.item.url}</div>
              <div class="dl-url">{r.item.url}</div>
            </div>
            <div class="dl-meta">
              <span class="status status-{r.item.status}">{r.item.status}</span>
              <span class="size">{fmtBytes(r.item.totalSize)}</span>
            </div>
          </button>
        {/each}
        {#if downloadResults.length === 0}
          <div class="empty">No downloads match "{query}"</div>
        {/if}
      {/if}
    </div>

    <div class="footer">
      <span><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
      <span><kbd>↵</kbd> select</span>
      <span><kbd>tab</kbd> switch mode</span>
      <span><kbd>esc</kbd> close</span>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-modal);
    background: var(--color-backdrop);
    backdrop-filter: blur(4px);
  }
  .palette {
    position: fixed;
    top: 18%;
    left: 50%;
    transform: translateX(-50%);
    z-index: calc(var(--z-modal) + 1);
    width: min(640px, 92vw);
    background: var(--color-bg-elevated);
    color: var(--color-text);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-modal);
    overflow: hidden;
  }
  .header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--color-border);
  }
  .search-ico {
    width: 18px;
    height: 18px;
    color: var(--color-faint);
    flex: none;
  }
  .header input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: var(--color-text);
    font-size: 14px;
    font-family: inherit;
    padding: 0;
  }
  .header input::placeholder { color: var(--color-faint); }
  .modes {
    display: flex;
    gap: 2px;
    padding: 2px;
    background: var(--color-surface-active);
    border: 1px solid var(--color-border-2);
    border-radius: var(--radius-sm);
  }
  .mode {
    background: none;
    border: none;
    color: var(--color-muted);
    padding: 3px 8px;
    border-radius: 4px;
    cursor: pointer;
    font-family: inherit;
    font-size: 11px;
    font-weight: 500;
    transition: background 0.12s, color 0.12s;
  }
  .mode:hover { color: var(--color-text); }
  .mode.active {
    background: var(--color-bg-elevated);
    color: var(--color-text);
  }
  .esc {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-faint);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    padding: 2px 6px;
  }
  .results {
    max-height: 50vh;
    overflow-y: auto;
    /* Keep wheel momentum inside the palette — when the user reaches
       the top or bottom of the results, the wheel should not fall
       through to the home view behind. */
    overscroll-behavior: contain;
    padding: 6px;
  }
  .result {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 8px 12px;
    background: none;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--color-text);
    text-align: left;
    cursor: pointer;
    font-family: inherit;
    transition: background 0.12s;
  }
  .result.selected {
    background: var(--color-primary-dim);
  }
  .result:hover {
    background: var(--color-surface-hover);
  }
  .ico {
    width: 16px;
    height: 16px;
    color: var(--color-muted);
    flex: none;
  }
  .title {
    flex: 1;
    font-size: 13px;
  }
  .cat {
    font-size: 11px;
    color: var(--color-faint);
    letter-spacing: 0;
  }
  .dl-info { flex: 1; min-width: 0; }
  .dl-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dl-url {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dl-meta {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .status {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0;
    padding: 2px 7px;
    border-radius: 4px;
    background: var(--color-surface-active);
    color: var(--color-muted);
  }
  .status-downloading { color: var(--color-primary); background: var(--color-primary-dim); }
  .status-completed { color: var(--color-success); background: var(--color-success-dim); }
  .status-error { color: var(--color-danger); background: var(--color-danger-dim); }
  .status-paused { color: var(--color-muted); }
  .size {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-muted);
  }
  .empty {
    padding: 30px 20px;
    text-align: center;
    color: var(--color-faint);
    font-size: 13px;
  }
  .footer {
    display: flex;
    gap: 14px;
    padding: 8px 14px;
    border-top: 1px solid var(--color-border);
    background: var(--color-bg);
    font-size: 11px;
    color: var(--color-faint);
  }
  .footer kbd {
    font-family: var(--font-mono);
    font-size: 11px;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 3px;
    padding: 1px 5px;
    margin-right: 3px;
  }
</style>