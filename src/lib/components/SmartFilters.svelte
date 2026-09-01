<script lang="ts">
  import { activeFilter, sidebarCollapsed, type FilterKey } from "../stores/ui";
  import { smartFolders } from "../stores/downloads";

  interface Group {
    key: FilterKey;
    label: string;
    icon: string;
    count: number;
  }

  const FILTERS: Group[] = [
    { key: "all", label: "All", icon: "M3 4h18v4H3V4zm0 6h18v4H3v-4zm0 6h18v4H3v-4z", count: 0 },
    { key: "active", label: "Active", icon: "M5 12h14M13 6l6 6-6 6", count: 0 },
    { key: "queued", label: "Queued", icon: "M12 8v4l3 3", count: 0 },
    { key: "paused", label: "Paused", icon: "M8 5v14M16 5v14", count: 0 },
    { key: "completed", label: "Completed", icon: "M20 6L9 17l-5-5", count: 0 },
    { key: "error", label: "Errors", icon: "M12 8v4m0 4h.01M4.93 19.07h14.14V4.93H4.93z", count: 0 },
  ];

  const SMART: Group[] = [
    { key: "video", label: "Video", icon: "M23 7l-7 5 7 5V7zM14 5H3a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2z", count: 0 },
    { key: "audio", label: "Audio", icon: "M9 18V5l12-2v13M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0zm12-2a3 3 0 1 1-6 0 3 3 0 0 1 6 0z", count: 0 },
    { key: "archive", label: "Archives", icon: "M21 8v13H3V8M1 3h22v5H1V3zm9 4h4v3h-4V7z", count: 0 },
    { key: "document", label: "Documents", icon: "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8l-6-6zM14 2v6h6M16 13H8M16 17H8M10 9H8", count: 0 },
    { key: "image", label: "Images", icon: "M3 5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5zm14 8l-3-3-5 5M9 11a2 2 0 1 0 0-4 2 2 0 0 0 0 4z", count: 0 },
    { key: "binary", label: "Software", icon: "M8 9l3 3-3 3M16 9l-3 3 3 3M3 4h18v16H3V4z", count: 0 },
  ];

  $: filters = FILTERS.map((f) => {
    if (f.key === "all") return { ...f, count: $smartFolders ? Object.values($smartFolders).reduce((s, v) => s + v.count, 0) : 0 };
    if (f.key === "active") return { ...f, count: $smartFolders ? Object.values($smartFolders).filter((_, i) => i < 5).reduce((s, v) => s + v.count, 0) : 0 };
    return f;
  });

  $: smartGroups = SMART.map((g) => ({
    ...g,
    count: $smartFolders[g.key]?.count || 0,
  })).filter((g) => g.count > 0);

  function select(k: FilterKey) {
    activeFilter.set(k);
  }
</script>

<nav class="nav-section" aria-label="Filters">
  {#if !$sidebarCollapsed}
    <div class="nav-label">Filters</div>
  {/if}
  <div class="nav-items">
    {#each filters as f}
      <button
        class="nav-item"
        class:collapsed={$sidebarCollapsed}
        class:active={$activeFilter === f.key}
        on:click={() => select(f.key)}
        aria-pressed={$activeFilter === f.key}
        title={$sidebarCollapsed ? f.label : undefined}
        aria-label={f.label}
      >
        <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d={f.icon} />
        </svg>
        {#if !$sidebarCollapsed}<span class="lbl">{f.label}</span>{/if}
        {#if !$sidebarCollapsed}<span class="count">{f.count}</span>{/if}
      </button>
    {/each}
  </div>
</nav>

{#if smartGroups.length > 0}
  <nav class="nav-section" aria-label="Smart folders">
    {#if !$sidebarCollapsed}
      <div class="nav-label">Smart folders</div>
    {/if}
    <div class="nav-items">
      {#each smartGroups as g}
        <button
          class="nav-item"
          class:collapsed={$sidebarCollapsed}
          class:active={$activeFilter === g.key}
          on:click={() => select(g.key)}
          title={$sidebarCollapsed ? g.label : undefined}
          aria-label={g.label}
        >
          <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d={g.icon} />
          </svg>
          {#if !$sidebarCollapsed}<span class="lbl">{g.label}</span>{/if}
          {#if !$sidebarCollapsed}<span class="count">{g.count}</span>{/if}
        </button>
      {/each}
    </div>
  </nav>
{/if}

<style>
  .nav-section { margin-bottom: 18px; }
  .nav-label {
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.12em;
    color: var(--faint);
    padding: 0 10px 8px;
  }
  .nav-items {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: 8px;
    color: var(--muted);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    border: 1px solid transparent;
    background: none;
    transition: all 0.15s;
    width: 100%;
    text-align: left;
    font-family: inherit;
    position: relative;
  }
  .nav-item.collapsed {
    width: 38px;
    height: 38px;
    padding: 0;
    justify-content: center;
    margin: 0 auto;
  }
  .nav-item:hover {
    color: var(--text);
    background: var(--surface-hover);
  }
  .nav-item.active {
    color: var(--text);
    background: linear-gradient(90deg, var(--color-primary-dim), transparent);
    border-color: var(--color-primary-strong);
  }
  .nav-item.collapsed.active {
    background: var(--color-primary-dim);
    border-color: var(--color-primary-strong);
  }
  .ico {
    width: 15px;
    height: 15px;
    flex: none;
    opacity: 0.85;
  }
  .lbl {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--faint);
    background: var(--surface-hover);
    padding: 1px 7px;
    border-radius: 99px;
    min-width: 22px;
    text-align: center;
    flex: none;
  }
  .nav-item.active .count {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  /* When collapsed, surface a small dot indicator for active + nonzero counts */
  .nav-item.collapsed.active::after {
    content: "";
    position: absolute;
    top: 6px;
    right: 6px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
</style>