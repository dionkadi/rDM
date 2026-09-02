<script lang="ts">
  import { createEventDispatcher, tick } from "svelte";
  import { slide, fade } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import type { Download } from "../types";
  import { fmtBytes, fmtRate, fmtDuration, fmtPercent, truncate } from "../utils/formatters";
  import FileIcon from "./FileIcon.svelte";
  import StatusBadge from "./StatusBadge.svelte";
  import ProgressBar from "./ProgressBar.svelte";
  import DropdownMenu, { type MenuItem } from "./DropdownMenu.svelte";
  import InlineSpeedLimit from "./InlineSpeedLimit.svelte";
  import { speedMap } from "../stores/downloads";

  export let download: Download;
  export let compact: boolean = false;
  export let selectable: boolean = false;
  export let selected: boolean = false;

  const dispatch = createEventDispatcher();

  let expanded = false;
  let menuOpen = false;
  let menuTrigger: HTMLElement;

  $: speed = $speedMap[download.id] || 0;
  $: pct = fmtPercent(download.downloaded, download.totalSize);
  $: pctNum = download.totalSize ? (download.downloaded / download.totalSize) * 100 : 0;
  $: eta = computeEta(download, speed);
  $: menuItems = buildMenu(download, () => (menuOpen = false));

  function computeEta(d: Download, s: number): string | null {
    if (!d.totalSize || s <= 0) return null;
    const rem = d.totalSize - d.downloaded;
    if (rem <= 0) return "done";
    return fmtDuration(Math.round(rem / s));
  }

  function buildMenu(d: Download, close: () => void): MenuItem[] {
    const items: MenuItem[] = [];
    if (d.status === "downloading" || d.status === "connecting" || d.status === "queued" || d.status === "scheduled") {
      items.push({ label: "Pause", icon: "M8 5v14M16 5v14", shortcut: "P", action: () => { close(); dispatch("action", { type: "pause", id: d.id }); } });
    }
    if (d.status === "paused" || d.status === "error" || d.status === "canceled") {
      items.push({ label: "Resume", icon: "M5 4l14 8-14 8V4z", shortcut: "R", action: () => { close(); dispatch("action", { type: "resume", id: d.id }); } });
    }
    if (d.status !== "completed" && d.status !== "canceled") {
      items.push({ label: "Cancel", icon: "M6 6l12 12M18 6L6 18", shortcut: "X", danger: true, action: () => { close(); dispatch("action", { type: "cancel", id: d.id }); } });
    }
    items.push({ separator: true, label: "" });
    items.push({ label: "Copy link", icon: "M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71", shortcut: "⌘C", action: () => { close(); navigator.clipboard?.writeText(d.url); } });
    items.push({ label: "Open in browser", icon: "M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6M15 3h6v6M10 14L21 3", action: () => { close(); window.open(d.url, "_blank"); } });
    if (d.savePath) {
      items.push({ label: "Open folder", icon: "M3 7l4-4h4l2 2h8v13H3V7z", action: () => { close(); dispatch("action", { type: "open-folder", id: d.id }); } });
    }
    items.push({ separator: true, label: "" });
    items.push({ label: "Remove", icon: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13", danger: true, action: () => { close(); dispatch("action", { type: "remove", id: d.id }); } });
    return items;
  }

  async function toggleExpand() {
    expanded = !expanded;
    if (expanded) await tick();
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      toggleExpand();
    }
  }
</script>

<div
  class="drow status-{download.status}"
  class:expanded
  class:compact
  class:selected
  role="button"
  aria-label="Download: {download.filename || download.url}, press Enter to {expanded ? 'collapse' : 'expand'} details"
  tabindex="0"
  on:keydown={onKeydown}
>
  <div class="lead">
    {#if selectable}
      <input type="checkbox" bind:checked={selected} aria-label="Select download" />
    {/if}
    <FileIcon
      filename={download.filename}
      url={download.url}
      progress={pctNum}
      status={download.status}
      size={compact ? 40 : 52}
    />
  </div>

  <div class="main">
    <div class="title-row">
      <div class="title">
        <span class="name" title={download.filename || download.url}>
          {truncate(download.filename || download.url, 60)}
        </span>
        <StatusBadge status={download.status} />
      </div>
      <div class="actions">
        <InlineSpeedLimit
          value={download.speedLimit}
          on:change={(e) => dispatch("action", { type: "set-limit", id: download.id, limit: e.detail })}
        />
        <button class="action-btn" on:click|stopPropagation={() => dispatch("action", { type: "pause", id: download.id })} title="Pause" aria-label="Pause">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M8 5v14M16 5v14"/></svg>
        </button>
        <button class="action-btn primary" on:click|stopPropagation={() => dispatch("action", { type: "resume", id: download.id })} title="Resume" aria-label="Resume">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M5 4l14 8-14 8V4z"/></svg>
        </button>
        <button
          class="action-btn menu-btn"
          bind:this={menuTrigger}
          on:click|stopPropagation={() => (menuOpen = !menuOpen)}
          title="More actions"
          aria-label="More actions"
          aria-haspopup="true"
          aria-expanded={menuOpen}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="5" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="12" cy="19" r="1.5"/></svg>
        </button>
        <DropdownMenu
          items={menuItems}
          trigger={menuTrigger}
          open={menuOpen}
          on:close={() => (menuOpen = false)}
        />
      </div>
    </div>

    <ProgressBar
      progress={pctNum}
      status={download.status}
      chunks={download.chunks}
      height={compact ? 4 : 6}
    />

    <div class="meta">
      <span class="seg"><b>{fmtBytes(download.downloaded)}</b>{download.totalSize ? " / " + fmtBytes(download.totalSize) : ""}</span>
      <span class="seg pct">{pct}</span>
      {#if (download.status === "downloading" || download.status === "connecting") && speed > 0}
        <span class="seg speed">{fmtRate(speed)}</span>
        {#if eta}<span class="seg">eta {eta}</span>{/if}
      {/if}
      {#if download.chunks.length > 1}
        <span class="seg streams">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 12h4M17 12h4M7 8v8M17 8v8M11 6v12"/></svg>
          {download.chunks.length} streams
        </span>
      {/if}
      {#if download.category}
        <span class="seg cat">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7l4-4h4l2 2h8v13H3V7z"/></svg>
          {download.category}
        </span>
      {/if}
      {#if download.proxy}
        <span class="seg proxy" title="Per-download proxy: {download.proxy}">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zM2 12h20M12 2a16 16 0 0 1 0 20M12 2a16 16 0 0 0 0 20"/></svg>
          proxy
        </span>
      {/if}
      {#if download.error && download.status === "error"}
        <span class="seg error" title={download.error}>⚠ {truncate(download.error, 40)}</span>
      {/if}
    </div>

    {#if expanded}
      <div class="expanded" transition:slide={{ duration: 220, easing: cubicOut }}>
        <div class="ex-section">
          <div class="ex-label">Chunks ({download.chunks.length})</div>
          {#if download.chunks.length > 0}
            <div class="chunks">
              {#each download.chunks as chunk}
                {@const isOpenEnded = !Number.isFinite(chunk.end) || chunk.end > Number.MAX_SAFE_INTEGER}
                {@const size = isOpenEnded ? 0 : chunk.end - chunk.start + 1}
                {@const chunkPct = isOpenEnded ? 0 : (chunk.downloaded / size) * 100}
                <div class="chunk">
                  <div class="chunk-head">
                    <span>#{chunk.index}</span>
                    <span>
                      {fmtBytes(chunk.downloaded)}{#if isOpenEnded}
                        <span class="open-ended-hint">· unknown total</span>
                      {:else}
                        &nbsp;/&nbsp;{fmtBytes(size)}
                      {/if}
                    </span>
                  </div>
                  <div class="chunk-bar">
                    <div class="chunk-fill" style="width: {chunkPct}%"></div>
                  </div>
                </div>
              {/each}
            </div>
          {:else}
            <div class="hint">Single-connection transfer (no chunks)</div>
          {/if}
        </div>

        <div class="ex-grid">
          <div class="ex-stat">
            <span class="k">URL</span>
            <span class="v mono">{truncate(download.url, 70)}</span>
          </div>
          <div class="ex-stat">
            <span class="k">Started</span>
            <span class="v">{new Date(download.createdAt).toLocaleString()}</span>
          </div>
          {#if download.finishedAt}
            <div class="ex-stat">
              <span class="k">Finished</span>
              <span class="v">{new Date(download.finishedAt).toLocaleString()}</span>
            </div>
          {/if}
          {#if download.contentType}
            <div class="ex-stat">
              <span class="k">Type</span>
              <span class="v mono">{download.contentType}</span>
            </div>
          {/if}
          {#if download.checksum}
            <div class="ex-stat">
              <span class="k">Checksum</span>
              <span class="v mono">{download.checksum.algorithm}: {truncate(download.checksum.expected, 20)}…</span>
            </div>
          {/if}
          {#if download.proxy}
            <div class="ex-stat">
              <span class="k">Proxy</span>
              <span class="v mono">{download.proxy}</span>
            </div>
          {/if}
          {#if download.savePath}
            <div class="ex-stat">
              <span class="k">Path</span>
              <span class="v mono">{truncate(download.savePath, 70)}</span>
            </div>
          {/if}
        </div>

        {#if download.error && download.status === "error"}
          <div class="ex-error" transition:fade={{ duration: 150 }}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 8v4m0 4h.01M4.93 19.07h14.14V4.93H4.93z"/></svg>
            {download.error}
          </div>
        {/if}
      </div>
    {/if}
  </div>

  <button
    class="expand"
    on:click|stopPropagation={toggleExpand}
    aria-label={expanded ? "Collapse details" : "Expand details"}
    aria-expanded={expanded}
  >
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class:rot={expanded}>
      <path d="M6 9l6 6 6-6"/>
    </svg>
  </button>
</div>

<style>
  .drow {
    display: grid;
    grid-template-columns: auto 1fr auto;
    gap: 14px;
    align-items: center;
    padding: 14px 16px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 14px;
    margin-bottom: 8px;
    box-shadow: var(--shadow-raised);
    transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
    cursor: pointer;
  }
  .drow:hover {
    border-color: var(--color-border-strong);
    background: var(--color-surface-hover);
    /* No `transform: translateX(2px)` here — it makes the chevron move
       by 2 px on hover, which (via the DropdownMenu's scroll handler)
       re-anchors the menu and produces a visible two-position jump. */
  }
  .drow.compact {
    padding: 10px 14px;
  }
  .drow.expanded {
    background: var(--surface-active);
    border-color: var(--color-primary-strong);
  }
  .drow.selected {
    border-color: var(--color-info);
    background: color-mix(in srgb, var(--color-info) 8%, var(--surface));
  }
  .drow.status-completed {
    border-color: var(--color-success-strong);
  }
  .drow.status-error, .drow.status-canceled {
    border-color: var(--color-danger-strong);
  }

  .lead {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .lead input[type="checkbox"] {
    width: 16px;
    height: 16px;
    accent-color: var(--accent);
  }

  .main { min-width: 0; }

  .title-row {
    display: flex;
    align-items: center;
    gap: 10px;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    flex: 1;
  }
  .name {
    font-weight: 600;
    font-size: 14px;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 4px;
    opacity: 0;
    /* No `transform: translateX(4px)` — the chevron sits inside `.actions`,
       and any transform here shifts the trigger by 4 px on hover, which
       causes the open DropdownMenu to re-anchor and produce a visible
       two-position jump. We keep the opacity fade for a clean reveal. */
    transition: opacity 0.18s;
    position: relative;
  }
  .drow:hover .actions, .drow:focus-within .actions, .drow.expanded .actions {
    opacity: 1;
  }
  .action-btn {
    width: 28px;
    height: 28px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 7px;
    color: var(--muted);
    cursor: pointer;
    display: grid;
    place-items: center;
    transition: all 0.15s;
  }
  .action-btn:hover {
    background: rgba(255, 255, 255, 0.06);
    color: var(--text);
    border-color: var(--border);
  }
  .action-btn.primary {
    color: var(--accent);
    background: var(--color-primary-dim);
  }
  .action-btn.primary:hover {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .action-btn svg {
    width: 14px;
    height: 14px;
  }

  .meta {
    display: flex;
    gap: 14px;
    flex-wrap: wrap;
    font-family: var(--font-mono);
    font-size: 11.5px;
    color: var(--muted);
    margin-top: 6px;
  }
  .seg {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .seg b {
    color: var(--text);
    font-weight: 500;
  }
  .pct { color: var(--accent); font-weight: 600; }
  .speed { color: var(--color-success); }
  .streams { color: var(--text-faint); }
  .streams svg, .cat svg, .proxy svg { width: 11px; height: 11px; }
  .cat { color: var(--text-faint); }
  .proxy {
    color: var(--color-info);
    background: var(--color-info-dim);
    border-radius: 99px;
    padding: 1px 7px;
  }
  .error { color: var(--color-danger); }

  .expand {
    width: 26px;
    height: 26px;
    background: transparent;
    border: 1px solid var(--hair);
    border-radius: 7px;
    color: var(--muted);
    cursor: pointer;
    display: grid;
    place-items: center;
    transition: all 0.15s;
  }
  .expand:hover {
    color: var(--text);
    border-color: var(--border-strong);
  }
  .expand svg {
    width: 14px;
    height: 14px;
    transition: transform 0.2s;
  }
  .expand svg.rot { transform: rotate(180deg); }

  .expanded {
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--hair-2);
  }

  .ex-section { margin-bottom: 14px; }
  .ex-label {
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.1em;
    color: var(--text-faint);
    margin-bottom: 8px;
  }
  .chunks {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 8px;
  }
  .chunk {
    background: rgba(255, 255, 255, 0.02);
    border: 1px solid var(--hair-2);
    border-radius: 8px;
    padding: 8px 10px;
  }
  .chunk-head {
    display: flex;
    justify-content: space-between;
    font-family: var(--font-mono);
    font-size: 10.5px;
    color: var(--muted);
    margin-bottom: 5px;
  }
  /* "unknown total" hint shown next to the running byte count for
     open-ended chunks (where the proxy didn't relay Content-Length
     and the engine uses a single chunk with end = u64::MAX). The
     size column is hidden in that case so the user never sees
     "879 B / 16777216.0 TB" or similar sentinel-leak. */
  .open-ended-hint {
    color: var(--text-faint);
    font-style: italic;
    margin-left: 4px;
  }
  .chunk-bar {
    height: 4px;
    background: rgba(255, 255, 255, 0.06);
    border-radius: 99px;
    overflow: hidden;
  }
  .chunk-fill {
    height: 100%;
    background: linear-gradient(90deg, var(--accent), var(--accent-2));
    border-radius: 99px;
    transition: width 0.3s;
  }

  .ex-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 8px 16px;
    margin-top: 8px;
  }
  .ex-stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .ex-stat .k {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-faint);
  }
  .ex-stat .v {
    font-size: 12.5px;
    color: var(--text);
  }
  .ex-stat .v.mono {
    font-family: var(--font-mono);
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ex-error {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    padding: 10px 12px;
    background: var(--color-danger-dim);
    border: 1px solid var(--color-danger-strong);
    border-radius: 9px;
    color: var(--color-danger);
    font-size: 12.5px;
  }

  .hint {
    color: var(--text-faint);
    font-size: 12px;
  }

  @media (prefers-reduced-motion: reduce) {
    .drow:hover { transform: none; }
  }
</style>