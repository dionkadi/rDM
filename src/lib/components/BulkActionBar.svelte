<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import {
    selectedIds,
    hasSelection,
    clearSelection,
  } from "../stores/ui";
  import { downloads } from "../stores/downloads";
  import { fmtBytes } from "../utils/formatters";

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

{#if $hasSelection}
  <div
    class="bulk-bar"
    role="toolbar"
    aria-label="Bulk actions for {count} selected downloads"
    transition:slide={{ duration: 200, easing: cubicOut }}
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
      <span class="count">
        <b>{count}</b> selected
      </span>
      <span class="sep"></span>
      <span class="stat" title="Active transfers in selection">
        <span class="dot" class:active={stats.active > 0}></span>
        {stats.active} active
      </span>
      <span class="stat">{fmtBytes(stats.downloaded)}{stats.total > 0 ? ` / ${fmtBytes(stats.total)}` : ""}</span>
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

      <button class="action danger" on:click={() => fire("remove")} title="Remove selected (Del)">
        <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/></svg>
        Remove
      </button>
    </div>
  </div>
{/if}

<style>
  .bulk-bar {
    position: sticky;
    top: 0;
    z-index: var(--z-sticky, 20);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    margin-bottom: 12px;
    background: color-mix(in srgb, var(--color-info) 12%, var(--surface));
    border: 1px solid color-mix(in srgb, var(--color-info) 50%, var(--border));
    border-radius: 12px;
    box-shadow: var(--shadow-raised, 0 1px 2px rgba(0, 0, 0, 0.06));
    backdrop-filter: blur(8px);
  }
  .left {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    flex: 1;
  }
  .right {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .x {
    width: 24px;
    height: 24px;
    background: transparent;
    border: 1px solid var(--hair);
    border-radius: 6px;
    color: var(--muted);
    cursor: pointer;
    display: grid;
    place-items: center;
    transition: all 0.15s;
  }
  .x:hover {
    color: var(--text);
    background: rgba(255, 255, 255, 0.06);
    border-color: var(--border-strong);
  }
  .count {
    color: var(--text);
    font-size: 13px;
  }
  .count b {
    color: var(--color-info);
    font-weight: 700;
  }
  .sep {
    width: 1px;
    height: 16px;
    background: var(--hair-2);
  }
  .stat {
    font-family: var(--font-mono);
    font-size: 11.5px;
    color: var(--muted);
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .stat.faint { color: var(--text-faint); }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-faint);
  }
  .dot.active {
    background: var(--color-success);
    box-shadow: 0 0 6px var(--color-success);
  }

  .action {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 6px 10px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s;
  }
  .action:hover {
    background: rgba(255, 255, 255, 0.06);
    border-color: var(--border-strong);
  }
  .action.primary {
    color: var(--accent);
    background: var(--color-primary-dim);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .action.primary:hover {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .action.danger {
    color: var(--color-danger);
    border-color: color-mix(in srgb, var(--color-danger) 30%, transparent);
  }
  .action.danger:hover {
    background: color-mix(in srgb, var(--color-danger) 14%, var(--surface));
  }

  .limit-wrap {
    position: relative;
  }
  .limit-input {
    width: 160px;
    padding: 6px 10px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text);
    font-size: 12px;
    font-family: var(--font-mono);
    outline: none;
  }
  .limit-input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 20%, transparent);
  }
</style>
