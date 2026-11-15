<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { fade } from "svelte/transition";
  import {
    selectedIds,
    hasSelection,
    clearSelection,
  } from "../stores/ui";
  import { downloads } from "../stores/downloads";
  import { fmtBytes } from "../utils/formatters";
  import type { ToolInfo } from "../types";

  /** Optional list of *visible* ids — the bar uses this to render
   *  the "in selection / visible total" line. Falls back to the
   *  full `downloads` set when the parent doesn't supply it. */
  export let visibleIds: string[] = [];
  let bulkLimit = "";
  let showLimitInput = false;

  const dispatch = createEventDispatcher();

  $: count = $selectedIds.size;
  // Aggregate stats for the selected rows (used in the summary line)
  $: stats = computeStats($downloads, $selectedIds, visibleIds);

  /**
   * Merging is only meaningful for exactly two *finished* rows — the
   * shape you get from a site that serves video as two independent DASH
   * streams (video + audio). Anything else and the button would invite
   * the user to create a broken file.
   *
   * ffmpeg is not bundled, so `ffmpegState` (probed once by the parent)
   * lets us disable the button with a reason instead of failing after
   * the click. `null` = not probed yet, which is treated as "allow"
   * so a slow probe never blocks an otherwise valid action; the
   * command itself reports a missing ffmpeg clearly.
   */
  export let ffmpegState: ToolInfo | null = null;

  $: selectedRows = $downloads.filter((d) => $selectedIds.has(d.id));
  $: allFinished =
    selectedRows.length === 2 &&
    selectedRows.every((d) => d.status === "completed");
  $: mergeDisabled = !allFinished || ffmpegState?.path === null;
  $: mergeTitle = !allFinished
    ? "Select exactly two finished downloads to merge (a DASH video + audio pair)"
    : ffmpegState?.path === null
      ? "Merging needs ffmpeg, which was not found on PATH — install it and restart DM"
      : "Merge these two into one playable file (ffmpeg stream copy, no re-encode)";

  function computeStats(
    list: typeof $downloads,
    sel: Set<string>,
    visIds: readonly string[],
  ): { downloaded: number; total: number; active: number; visible: number } {
    let downloaded = 0;
    let total = 0;
    let active = 0;
    const visibleSet = visIds.length > 0 ? new Set(visIds) : null;
    for (const d of list) {
      if (!sel.has(d.id)) continue;
      downloaded += d.downloaded || 0;
      total += d.totalSize || 0;
      if (d.status === "downloading" || d.status === "connecting") active++;
    }
    return { downloaded, total, active, visible: visibleSet?.size ?? list.length };
  }

  function fire(type: string, extra: Record<string, unknown> = {}) {
    const ids = Array.from($selectedIds);
    dispatch("action", { type, ids, ...extra });
  }

  function applyBulkLimit() {
    const v = bulkLimit.trim();
    if (v === "") {
      fire("set-limit", { limit: null });
    } else {
      const mb = Number(v);
      if (isNaN(mb) || mb < 0) return;
      fire("set-limit", { limit: Math.round(mb * 1024 * 1024) });
    }
    showLimitInput = false;
    bulkLimit = "";
  }
</script>

<!-- A contextual floating strip, not a second toolbar: it sits just
     above the status bar so selecting a row never pushes the ledger
     down or fights the sticky column header. -->
{#if $hasSelection}
  <div
    class="bulk-bar"
    role="toolbar"
    aria-label="Bulk actions for {count} selected downloads"
    transition:fade={{ duration: 120 }}
  >
    <div class="left">
      <button
        class="x"
        on:click={clearSelection}
        aria-label="Clear selection"
        title="Clear selection (Esc)"
      >
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round">
          <path d="M6 6l12 12M18 6L6 18"/>
        </svg>
      </button>
      <span class="count"><b>{count}</b> selected</span>
      <span class="sep"></span>
      <span class="stat" title="Active transfers in selection">
        <span class="dot" class:active={stats.active > 0}></span>
        {stats.active} active
      </span>
      <span class="stat mono" title="Downloaded / total size">
        {fmtBytes(stats.downloaded)}{stats.total > 0 ? ` / ${fmtBytes(stats.total)}` : ""}
      </span>
      {#if stats.visible !== $downloads.length}
        <span class="stat faint">of {stats.visible} visible</span>
      {/if}
    </div>

    <div class="right">
      <button class="action" on:click={() => fire("pause")} title="Pause selected (P)">
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M8 5v14M16 5v14"/></svg>
        Pause
      </button>
      <button class="action primary" on:click={() => fire("resume")} title="Resume selected (R)">
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 4l14 8-14 8V4z"/></svg>
        Resume
      </button>

      <div class="limit-wrap">
        {#if showLimitInput}
          <input
            class="limit-input"
            type="number"
            min="0"
            placeholder="MB/s (blank = clear)"
            bind:value={bulkLimit}
            on:keydown={(e) => {
              if (e.key === "Enter") applyBulkLimit();
              if (e.key === "Escape") { showLimitInput = false; bulkLimit = ""; }
            }}
            on:blur={applyBulkLimit}
            aria-label="Speed limit in MB per second"
          />
        {:else}
          <button class="action" on:click={() => (showLimitInput = true)} title="Set speed limit for selected">
            <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 14l4-4M4 18l8-8M14 4l6 6M10 10l-6 6"/></svg>
            Limit
          </button>
        {/if}
      </div>

      <button
        class="action"
        on:click={() => fire("merge")}
        disabled={mergeDisabled}
        title={mergeTitle}
      >
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M7 4v6a3 3 0 0 0 3 3h4M7 20v-6a3 3 0 0 1 3-3h4"/><path d="M14 7h4v4"/></svg>
        Merge
      </button>

      <button class="action" on:click={() => fire("trash")} title="Move selected to Trash (recoverable)">
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6M10 11v6M14 11v6"/></svg>
        Trash
      </button>
      <button class="action danger" on:click={() => fire("remove")} title="Remove selected (Del)">
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/></svg>
        Remove
      </button>
    </div>
  </div>
{/if}

<style>
  .bulk-bar {
    position: fixed;
    left: 50%;
    bottom: calc(var(--size-statusbar) + 10px);
    transform: translateX(-50%);
    z-index: var(--z-overlay);
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    max-width: calc(100vw - 24px);
    padding: 6px 8px 6px 10px;
    background: var(--color-bg-elevated);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-floating);
  }
  .left {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
    flex: 1;
  }
  .right {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .x {
    width: 24px;
    height: 24px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    color: var(--color-muted);
    cursor: pointer;
    display: grid;
    place-items: center;
    transition: background 0.12s var(--easing-out), color 0.12s var(--easing-out);
  }
  .x:hover {
    color: var(--color-text);
    background: var(--color-surface-hover);
  }
  .count {
    color: var(--color-text);
    font-size: 12px;
    white-space: nowrap;
  }
  .count b {
    font-weight: 600;
  }
  .sep {
    width: 1px;
    height: 16px;
    background: var(--color-border);
  }
  .stat {
    font-size: 11px;
    color: var(--color-muted);
    display: inline-flex;
    align-items: center;
    gap: 5px;
    white-space: nowrap;
  }
  .stat.mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
  .stat.faint { color: var(--color-faint); }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--color-faint);
  }
  .dot.active {
    background: var(--color-primary);
  }

  /* Ghost actions: the bar itself is the surface. Six filled buttons
     inside a floating card read as a second toolbar. */
  .action {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 9px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    color: var(--color-muted);
    font-family: var(--font-ui);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition:
      background 0.12s var(--easing-out),
      color 0.12s var(--easing-out);
  }
  .action:hover {
    background: var(--color-surface-hover);
    color: var(--color-text);
  }
  .action:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .action:disabled:hover {
    background: transparent;
    color: var(--color-muted);
  }
  .action.primary {
    color: var(--color-primary);
  }
  .action.primary:hover {
    background: var(--color-primary-dim);
    color: var(--color-primary);
  }
  .action.danger {
    color: var(--color-danger);
  }
  .action.danger:hover {
    background: var(--color-danger-dim);
    color: var(--color-danger);
  }

  .limit-wrap {
    position: relative;
  }
  .limit-input {
    width: 160px;
    height: 28px;
    padding: 0 9px;
    background: var(--color-surface);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    color: var(--color-text);
    font-size: 12px;
    font-family: var(--font-mono);
    outline: none;
  }
  .limit-input:focus {
    border-color: var(--color-primary);
    box-shadow: var(--shadow-glow);
  }

  @media (max-width: 640px) {
    .bulk-bar {
      left: 12px;
      right: 12px;
      transform: none;
      max-width: none;
    }
    .left { flex-basis: 100%; }
  }
</style>
