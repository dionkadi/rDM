<script lang="ts">
  import { showCommandPalette } from "../stores/ui";
  import { onMount } from "svelte";

  let isMac = false;
  onMount(() => {
    isMac = /Mac|iPhone|iPad/i.test(navigator.platform);
  });

  function open() {
    showCommandPalette.set(true);
  }
</script>

<!-- Compact command-palette trigger for the top bar. -->
<button
  class="search-trigger"
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
    gap: 7px;
    width: 240px;
    height: 28px;
    padding: 0 8px;
    background: var(--color-surface-hover);
    border: 1px solid var(--color-border-2);
    border-radius: var(--radius-sm);
    color: var(--color-faint);
    font-family: inherit;
    font-size: 12px;
    cursor: pointer;
    transition:
      border-color 0.12s var(--easing-out),
      color 0.12s var(--easing-out);
    overflow: hidden;
  }
  .search-trigger:hover {
    border-color: var(--color-border-strong);
    color: var(--color-muted);
  }
  .ico {
    width: 13px;
    height: 13px;
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
    font-size: 11px;
    color: var(--color-faint);
    flex: none;
  }
</style>
