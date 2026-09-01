<script lang="ts">
  import { showCommandPalette, sidebarCollapsed } from "../stores/ui";
  import { onMount } from "svelte";

  let isMac = false;
  onMount(() => {
    isMac = /Mac|iPhone|iPad/i.test(navigator.platform);
  });

  function open() {
    showCommandPalette.set(true);
  }
</script>

<button
  class="search-trigger"
  class:collapsed={$sidebarCollapsed}
  on:click={open}
  aria-label="Open command palette"
  title="Search or run a command…"
>
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
    <path d="M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.35-4.35"/>
  </svg>
  <span class="lbl">Search…</span>
  <kbd class="hint">{isMac ? "⌘K" : "Ctrl K"}</kbd>
</button>

<style>
  .search-trigger {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 10px;
    background: var(--surface-hover);
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--faint);
    font-family: inherit;
    font-size: 12.5px;
    cursor: pointer;
    transition: all 0.15s;
    overflow: hidden;
  }
  .search-trigger:hover {
    background: var(--surface-active);
    border-color: var(--border-strong);
    color: var(--muted);
  }
  .ico {
    width: 14px;
    height: 14px;
    flex: none;
  }
  .lbl {
    flex: 1;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    font-family: var(--font-mono);
    font-size: 10px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 1px 5px;
    color: var(--faint);
    flex: none;
  }
  /* Collapsed mode — icon-only pill, square aspect */
  .search-trigger.collapsed {
    width: 38px;
    height: 38px;
    padding: 0;
    justify-content: center;
    margin: 0 auto;
    border-radius: 10px;
  }
  .search-trigger.collapsed .lbl,
  .search-trigger.collapsed .hint {
    display: none;
  }
  .search-trigger.collapsed .ico {
    width: 16px;
    height: 16px;
  }
</style>