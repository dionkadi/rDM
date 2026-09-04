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
  /**
   * Position of this row in the *visible* list (1-based). When
   * provided, Alt+↑ / Alt+↓ keyboard shortcuts move the row in
   * the list by one slot. The parent owns the list ordering and
   * the actual `reorder` call; this row just dispatches the
   * `move-up` / `move-down` event with the right id.
   */
  export let index: number = 0;
  /** Total number of rows in the visible list. */
  export let total: number = 0;

  const dispatch = createEventDispatcher();

  // ── Drag and drop ────────────────────────────────────────
  // We use the native HTML5 drag API rather than a library
  // because the use case is small: drag a row by its handle,
  // drop on another row, the parent reorders the array. The
  // browser handles the visual feedback (the row is
  // automatically half-transparent while dragged). We do
  // *not* use the `dataTransfer` payload for ordering — the
  // parent knows the source id from the `dragstart` event.
  let dragOver: "above" | "below" | null = null;
  function onDragStart(e: DragEvent) {
    if (!e.dataTransfer) return;
    e.dataTransfer.setData("text/x-dm-download-id", download.id);
    // `effectAllowed = "move"` is the default for most browsers
    // but setting it explicitly prevents some browsers from
    // showing the "copy" cursor when the row is over a text
    // field by accident.
    e.dataTransfer.effectAllowed = "move";
  }
  function onDragOver(e: DragEvent) {
    e.preventDefault();
    if (!e.dataTransfer) return;
    e.dataTransfer.dropEffect = "move";
    // The top/bottom half of the row determines whether the
    // drop is "insert above" or "insert below". A 12 px deadband
    // near the row edges keeps the cursor steady when the
    // pointer hovers exactly on the boundary.
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const y = e.clientY - r.top;
    dragOver = y < r.height / 2 ? "above" : "below";
  }
  function onDragLeave() {
    dragOver = null;
  }
  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragOver = null;
    const sourceId = e.dataTransfer?.getData("text/x-dm-download-id");
    if (!sourceId || sourceId === download.id) return;
    dispatch("reorder", {
      sourceId,
      targetId: download.id,
      position: dragOver === "above" ? "before" : "after",
    });
  }

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
    if (d.savePath) {
      // "Copy path" duplicates the absolute save path to the
      // clipboard. Routed through the Tauri command surface
      // (the `copyText` api wrapper) so it works in the
      // embedded webview where `navigator.clipboard.writeText`
      // is sometimes blocked outside a user gesture.
      items.push({ label: "Copy path", icon: "M9 5H7a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2h-2M9 5a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v0a2 2 0 0 1-2 2h-2a2 2 0 0 1-2-2v0zM9 13h6M9 17h6", action: () => { close(); dispatch("action", { type: "copy-path", id: d.id }); } });
    }
    // "Auth…" opens the per-download auth dialog so the
    // user can attach HTTP basic / bearer credentials,
    // custom headers (Referer, User-Agent, Cookie, …) or
    // import cookies from their browser. Available on every
    // status (the user might want to set auth *before* a
    // 401 surfaces, not after).
    items.push({ label: "Auth…", icon: "M12 11V7a4 4 0 1 1 8 0v4M5 11h14a2 2 0 0 1 2 2v8H3v-8a2 2 0 0 1 2-2z", action: () => { close(); dispatch("action", { type: "auth", id: d.id }); } });
    items.push({ label: "Open in browser", icon: "M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6M15 3h6v6M10 14L21 3", action: () => { close(); window.open(d.url, "_blank"); } });
    if (d.savePath) {
      // "Open" launches the OS default handler (PDF reader,
      // video player, etc.). Only offered for completed
      // downloads — for in-flight ones the file may not
      // exist yet, in which case the Rust side returns a
      // clear "still downloading" error and the toast tells
      // the user to wait.
      if (d.status === "completed") {
        items.push({ label: "Open", icon: "M9 11l3 3L22 4M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11", action: () => { close(); dispatch("action", { type: "open-file", id: d.id }); } });
      }
      items.push({ label: "Open folder", icon: "M3 7l4-4h4l2 2h8v13H3V7z", action: () => { close(); dispatch("action", { type: "open-folder", id: d.id }); } });
    }
    items.push({ separator: true, label: "" });
    // Two destructive-but-recoverable actions at the bottom
    // of the menu, separated by a divider from the rest of
    // the actions. "Move to Trash" is the recoverable default
    // — the file goes to the OS trash and the SQLite row is
    // removed. "Remove" is the older "delete the row but
    // leave the file on disk" action, kept for users who
    // want to keep the file outside the engine (e.g. they
    // archived it already) but want it gone from the list.
    items.push({ label: "Move to Trash", icon: "M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6", action: () => { close(); dispatch("action", { type: "trash", id: d.id }); } });
    items.push({ label: "Remove (keep file)", icon: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13", danger: true, action: () => { close(); dispatch("action", { type: "remove", id: d.id }); } });
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
      return;
    }
    // Alt+ArrowUp / Alt+ArrowDown: keyboard-equivalent of the
    // drag-and-drop reorder. The row's `index` is 1-based and
    // `total` is the visible list size; we clamp the move to
    // `[1, total]` so the user can't go past either end.
    if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      e.preventDefault();
      if (e.key === "ArrowUp" && index > 1) {
        dispatch("reorder-keyboard", { id: download.id, from: index, to: index - 1 });
      } else if (e.key === "ArrowDown" && index < total) {
        dispatch("reorder-keyboard", { id: download.id, from: index, to: index + 1 });
      }
    }
  }

  /**
   * Click anywhere on the row (outside the interactive controls)
   * → fire a `select` event with the row's id plus the shift /
   * ctrl-or-meta modifiers. The parent decides what to do with it
   * (toggle / range / replace). The action buttons and the
   * dropdown trigger call `e.stopPropagation()` so they don't
   * double-fire the event.
   */
  function onRowClick(e: MouseEvent) {
    if (!selectable) return;
    dispatch("select", {
      id: download.id,
      shift: e.shiftKey,
      ctrlOrMeta: e.ctrlKey || e.metaKey,
    });
  }

  /**
   * Read the priority value out of the `<select>` change event
   * and dispatch it as an `action` of type `set-priority`. The
   * parent handler (`onAction` in `App.svelte`) dispatches it
   * to the Rust engine and refreshes the list. We extract this
   * to a named function because the Svelte template parser
   * rejects inline TypeScript `as` casts inside arrow
   * functions — the cast lives here in the `<script>` block
   * where TypeScript is allowed.
   */
  function onPriorityChange(e: Event) {
    const target = e.currentTarget as HTMLSelectElement;
    const priority = Number(target.value);
    if (Number.isNaN(priority)) return;
    dispatch("action", { type: "set-priority", id: download.id, priority });
  }
</script>

<div
  class="drow status-{download.status}"
  class:expanded
  class:compact
  class:selected
  class:drag-above={dragOver === "above"}
  class:drag-below={dragOver === "below"}
  draggable="true"
  role="button"
  aria-label="Download: {download.filename || download.url}, press Enter to {expanded ? 'collapse' : 'expand'} details"
  tabindex="0"
  on:keydown={onKeydown}
  on:click={onRowClick}
  on:dragstart={onDragStart}
  on:dragover={onDragOver}
  on:dragleave={onDragLeave}
  on:drop={onDrop}
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
        <select
          class="priority"
          class:hi={download.priority === 2}
          class:lo={download.priority === 0}
          value={download.priority}
          on:change={onPriorityChange}
          on:click|stopPropagation
          title="Priority — higher runs first when slots are full"
          aria-label="Download priority"
        >
          <option value={0}>Low</option>
          <option value={1}>Normal</option>
          <option value={2}>High</option>
        </select>
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
  /* Drag-and-drop reorder affordances. A 2 px accent line on the
     edge where the row will land tells the user exactly where
     the drop will insert. The line fades in within 80 ms so the
     feedback is fast enough that the user can target a slot
     mid-drag without overshooting. We avoid a box-shadow on
     the whole row — it would shift the layout and the user's
     pointer would no longer line up with the slot they aimed at. */
  .drow.drag-above {
    box-shadow: inset 0 2px 0 0 var(--accent);
  }
  .drow.drag-below {
    box-shadow: inset 0 -2px 0 0 var(--accent);
  }
  .drow[draggable="true"] {
    cursor: grab;
  }
  .drow[draggable="true"]:active {
    cursor: grabbing;
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

  /* Priority badge: a small 2-letter chip that shows the current
     priority. We use a `<select>` (not a button) so screen readers
     announce it as a form control and the keyboard picker works
     natively. The chip color shifts with the priority level so
     the user can scan the list and find "high" items at a glance. */
  .priority {
    height: 22px;
    padding: 0 6px 0 8px;
    background: var(--color-primary-dim);
    border: 1px solid color-mix(in srgb, var(--accent) 20%, transparent);
    border-radius: 99px;
    color: var(--accent);
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    cursor: pointer;
    outline: none;
    appearance: none;
    -webkit-appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12' fill='none' stroke='%237af0c8' stroke-width='1.6'%3E%3Cpath d='M3 5l3 3 3-3'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 4px center;
    background-size: 9px 9px;
    padding-right: 18px;
  }
  .priority:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 25%, transparent);
  }
  .priority.hi {
    background-color: var(--color-warning-dim, rgba(255, 180, 0, 0.16));
    border-color: color-mix(in srgb, #ffb400 40%, transparent);
    color: #ffb400;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12' fill='none' stroke='%23ffb400' stroke-width='1.6'%3E%3Cpath d='M3 5l3 3 3-3'/%3E%3C/svg%3E");
  }
  .priority.lo {
    background-color: var(--surface);
    border-color: var(--border);
    color: var(--text-faint);
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12' fill='none' stroke='%23808080' stroke-width='1.6'%3E%3Cpath d='M3 5l3 3 3-3'/%3E%3C/svg%3E");
  }
  .priority option {
    background: var(--surface);
    color: var(--text);
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