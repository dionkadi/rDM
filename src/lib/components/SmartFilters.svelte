<script lang="ts">
  import { activeFilter, searchQuery, type FilterKey } from "../stores/ui";
  import { scopedCounts, smartFolders } from "../stores/downloads";

  interface Group {
    key: FilterKey;
    label: string;
    count: number;
  }

  // Text-only chips. The old 12 px icons repeated the label and turned
  // the strip into a row of tiny glyphs; density comes from the counts,
  // not from decoration.
  const FILTERS: Group[] = [
    { key: "all", label: "All", count: 0 },
    { key: "active", label: "Active", count: 0 },
    { key: "queued", label: "Queued", count: 0 },
    { key: "paused", label: "Paused", count: 0 },
    { key: "completed", label: "Done", count: 0 },
    { key: "error", label: "Errors", count: 0 },
  ];

  const SMART: Group[] = [
    { key: "video", label: "Video", count: 0 },
    { key: "audio", label: "Audio", count: 0 },
    { key: "archive", label: "Archives", count: 0 },
    { key: "document", label: "Documents", count: 0 },
    { key: "image", label: "Images", count: 0 },
    { key: "binary", label: "Software", count: 0 },
  ];

  // Chips count the current *search scope*, not the whole library —
  // otherwise a search leaving one row visible still reads "All 6".
  // The status bar keeps using the whole-library `counts` store.
  $: countsByKey = {
    all: $scopedCounts.all,
    active: $scopedCounts.active,
    queued: $scopedCounts.queued,
    paused: $scopedCounts.paused,
    completed: $scopedCounts.completed,
    error: $scopedCounts.errored,
  } as Record<string, number>;

  $: scopeLabel = $searchQuery.trim() ? " in search results" : "";

  $: filters = FILTERS.map((f) => ({ ...f, count: countsByKey[f.key] ?? 0 }));

  $: smartGroups = SMART.map((g) => ({
    ...g,
    count: $smartFolders[g.key]?.count || 0,
  })).filter((g) => g.count > 0);

  function select(k: FilterKey) {
    activeFilter.set(k);
  }
</script>

<!-- Horizontal filter strip — replaces the old sidebar nav. Status
     filters first, smart folders after a separator (only when they
     have counts). The strip scrolls when it overflows. -->
<nav class="strip" aria-label="Filters">
  {#each filters as f}
    <button
      class="chip"
      class:active={$activeFilter === f.key}
      on:click={() => select(f.key)}
      aria-pressed={$activeFilter === f.key}
      title={f.count > 0 ? `${f.label} — ${f.count}${scopeLabel}` : f.label}
    >
      <span class="lbl">{f.label}</span>
      {#if f.count > 0}<span class="count">{f.count}</span>{/if}
    </button>
  {/each}
  {#if smartGroups.length > 0}
    <span class="sep" aria-hidden="true"></span>
    {#each smartGroups as g}
      <button
        class="chip"
        class:active={$activeFilter === g.key}
        on:click={() => select(g.key)}
        aria-pressed={$activeFilter === g.key}
        title={`${g.label} — ${g.count}${scopeLabel}`}
      >
        <span class="lbl">{g.label}</span>
        <span class="count">{g.count}</span>
      </button>
    {/each}
  {/if}
</nav>

<style>
  .strip {
    display: flex;
    align-items: center;
    gap: 2px;
    height: var(--size-filterbar);
    padding: 0 20px;
    overflow-x: auto;
    scrollbar-width: none;
    position: sticky;
    top: var(--size-topbar);
    z-index: 190;
    background: var(--topbar-bg);
    border-bottom: 1px solid var(--color-border);
  }
  .strip::-webkit-scrollbar {
    display: none;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 9px;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-muted);
    font-family: inherit;
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
    cursor: pointer;
    transition:
      background var(--duration-fast) var(--easing-out),
      color var(--duration-fast) var(--easing-out);
  }
  .chip:hover {
    background: var(--color-surface-hover);
    color: var(--color-text);
  }
  .chip.active {
    background: var(--color-primary-dim);
    color: var(--color-primary);
  }
  .count {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    color: var(--color-faint);
  }
  .chip.active .count {
    color: var(--color-primary);
  }
  .sep {
    width: 1px;
    height: 16px;
    margin: 0 6px;
    background: var(--color-border);
    flex: none;
  }
</style>
