<script lang="ts">
  import { createEventDispatcher, tick } from "svelte";
  import { slide, fade } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import type { Download } from "../types";
  import { fmtBytes, fmtRate, fmtDuration, fmtPercent, truncate } from "../utils/formatters";
  import FileIcon from "./FileIcon.svelte";
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
  // Rows are deliberately NOT draggable. The window is the drop
  // target: dragging a link (or anything with a URL) from the
  // browser *into* the app adds it as a download, and the row
  // must neither hijack nor swallow those drops — the row's own
  // drag handlers used to `preventDefault()` every dragover and
  // read only their private payload type, which blocked the
  // app-level drop handler and made the task card itself drag
  // around like a ghost. Reordering stays available through the
  // keyboard (Alt+↑ / Alt+↓), which dispatches `reorder-keyboard`.

  const STATUS_LABELS: Record<string, string> = {
    downloading: "Downloading",
    connecting: "Connecting",
    paused: "Paused",
    completed: "Done",
    error: "Error",
    canceled: "Canceled",
    queued: "Queued",
    scheduled: "Scheduled",
  };

  let expanded = false;
  let menuOpen = false;
  let menuTrigger: HTMLElement;

  $: speed = $speedMap[download.id] || 0;
  $: pct = fmtPercent(download.downloaded, download.totalSize);
  $: pctNum = download.totalSize ? (download.downloaded / download.totalSize) * 100 : 0;
  $: eta = computeEta(download, speed);
  $: menuItems = buildMenu(download, () => (menuOpen = false));
  // Chunked downloads report their chunk count; HLS reports live fetches.
  // Both mean "how many connections are working on this right now".
  $: liveStreams = download.segmentProgress
    ? download.segmentProgress.active
    : download.chunks.length;
  $: segmentCells = buildSegmentCells(download);

  // ── Segment map (HLS) ────────────────────────────────────
  // A playlist can list thousands of segments. One cell per segment
  // would bloat the DOM and be unreadable in a 400 px panel, so large
  // playlists are bucketed: each cell covers `total / cells` segments
  // and still shows the fraction of that bucket which is on disk.
  const MAX_SEGMENT_CELLS = 64;

  interface SegmentCell {
    /** Fill of this cell, 0–100. */
    pct: number;
    /** A fetch for this bucket is in flight right now. */
    active: boolean;
  }

  function buildSegmentCells(d: Download): SegmentCell[] {
    const sp = d.segmentProgress;
    if (!sp || sp.total <= 0) return [];
    const cells = Math.min(sp.total, MAX_SEGMENT_CELLS);
    const per = sp.total / cells;
    const activeEnd = sp.done + sp.active;
    const out: SegmentCell[] = [];
    for (let i = 0; i < cells; i++) {
      const from = Math.floor(i * per);
      const to = Math.max(from + 1, Math.floor((i + 1) * per));
      const count = to - from;
      const done = Math.min(count, Math.max(0, sp.done - from));
      out.push({
        pct: (done / count) * 100,
        // Only buckets that start at/after the written prefix are
        // "in flight"; the boundary bucket is already partly on disk
        // and must not fade its fill.
        active: sp.active > 0 && from >= sp.done && from < activeEnd,
      });
    }
    return out;
  }

  function computeEta(d: Download, s: number): string | null {
    if (!d.totalSize || s <= 0) return null;
    const rem = d.totalSize - d.downloaded;
    if (rem <= 0) return "done";
    return fmtDuration(Math.round(rem / s));
  }

  function sizeLabel(d: Download): string {
    // A finished transfer only needs its size once — "219.3 / 219.3 MB"
    // is noise. In-flight rows show the running / total split.
    if (d.status === "completed") return fmtBytes(d.totalSize || d.downloaded);
    if (!d.totalSize) return fmtBytes(d.downloaded);
    // Share the unit when both sides have the same one, so the
    // column reads "219.3 / 219.3 MB" instead of truncating.
    const a = fmtBytes(d.downloaded).split(" ");
    const b = fmtBytes(d.totalSize).split(" ");
    return a[1] === b[1] ? `${a[0]} / ${b.join(" ")}` : `${a.join(" ")} / ${b.join(" ")}`;
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
    // Ignore keys that originated on the checkbox / action buttons —
    // otherwise Enter on "Pause" both clicks the button and expands
    // the row, which looks like a double action.
    if (e.target !== e.currentTarget) return;
    if (e.key === "Enter" || e.key === " " || e.key === "Spacebar") {
      // `role="button"` promises both Enter and Space; without the
      // Space branch the row only scrolls the page on a keypress.
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

<!-- Ledger row: one line of record with fixed numeric columns;
     click/Enter expands the detail panel. Grid columns are shared
     with the column header in App (var(--row-grid) in app.css). -->
<div
  class="drow status-{download.status}"
  class:expanded
  class:compact
  class:selected
  role="button"
  aria-label="Download: {download.filename || download.url}, press Enter to {expanded ? 'collapse' : 'expand'} details"
  tabindex="0"
  on:keydown={onKeydown}
  on:click={onRowClick}
>
  {#if selectable}
    <!-- One-way `checked` (never `bind`): the checkbox must toggle the
         *store* via a select event, not a local prop — a local mutation
         is silently reverted on the next parent render, which is how
         checkboxes appeared to check and then uncheck themselves. -->
    <input
      class="cb"
      type="checkbox"
      checked={selected}
      aria-label="Select download"
      on:click|stopPropagation={() =>
        dispatch("select", { id: download.id, shift: false, ctrlOrMeta: true })}
    />
  {:else}
    <span class="cb-slot"></span>
  {/if}

  <FileIcon
    filename={download.filename}
    url={download.url}
    size={compact ? 24 : 28}
  />

  <div class="name" title={download.filename || download.url}>
    {truncate(download.filename || download.url, 90)}
  </div>

  <div class="c-progress">
    <ProgressBar
      progress={pctNum}
      status={download.status}
      chunks={download.chunks}
      segmentProgress={download.segmentProgress ?? null}
      height={download.status === "downloading" || download.status === "connecting" ? 5 : 3}
    />
    <span class="pct">{pct}</span>
  </div>

  <div class="col c-size">{sizeLabel(download)}</div>

  <div class="col c-speed">
    {#if (download.status === "downloading" || download.status === "connecting") && speed > 0}
      {fmtRate(speed)}{#if liveStreams > 1}<span class="streams" title="{liveStreams} connections working on this download">×{liveStreams}</span>{/if}
    {:else}
      <span class="dim">—</span>
    {/if}
  </div>

  <div class="col c-eta">
    {#if (download.status === "downloading" || download.status === "connecting") && eta && eta !== "done"}
      {eta}
    {:else}
      <span class="dim">—</span>
    {/if}
  </div>

  <div class="col c-status">
    <span class="dot" class:live={download.status === "downloading"}></span>
    <span class="st-label">{STATUS_LABELS[download.status] ?? download.status}</span>
  </div>

  <div class="c-actions">
    {#if download.status === "downloading" || download.status === "connecting" || download.status === "queued" || download.status === "scheduled"}
      <button class="action-btn" on:click|stopPropagation={() => dispatch("action", { type: "pause", id: download.id })} title="Pause" aria-label="Pause">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M8 5v14M16 5v14"/></svg>
      </button>
    {:else}
      <button class="action-btn primary" on:click|stopPropagation={() => dispatch("action", { type: "resume", id: download.id })} title="Resume" aria-label="Resume">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M5 4l14 8-14 8V4z"/></svg>
      </button>
    {/if}
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
    <button
      class="action-btn expand"
      on:click|stopPropagation={toggleExpand}
      aria-label={expanded ? "Collapse details" : "Expand details"}
      aria-expanded={expanded}
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class:rot={expanded}>
        <path d="M6 9l6 6 6-6"/>
      </svg>
    </button>
  </div>

  {#if expanded}
    <div class="expanded-panel" transition:slide={{ duration: 180, easing: cubicOut }}>
      <div class="ex-controls">
        <select
          class="priority"
          class:hi={download.priority === 2}
          class:lo={download.priority === 0}
          value={download.priority ?? 1}
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
        {#if download.error && download.status === "error"}
          <span class="ex-error-inline" title={download.error}>⚠ {truncate(download.error, 80)}</span>
        {/if}
      </div>

      {#if download.segmentProgress}
        {@const sp = download.segmentProgress}
        <div class="ex-section">
          <div class="ex-label">
            Segments
            <span class="ex-count">{sp.done.toLocaleString()} / {sp.total.toLocaleString()}</span>
            {#if sp.active > 0}
              <span class="ex-note">{sp.active} in flight</span>
            {:else if sp.done >= sp.total}
              <span class="ex-note">all written</span>
            {/if}
          </div>
          <div class="seg-map" title="One cell per segment (bucketed for long playlists): filled means the bytes are in the output file">
            {#each segmentCells as cell}
              <span class="seg-cell" class:active={cell.active}>
                <i style="width: {cell.pct}%"></i>
              </span>
            {/each}
          </div>
          <div class="hint seg-legend">
            Fetched concurrently, written in playlist order; tinted cells are in flight.
          </div>
        </div>
      {:else}
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
      {/if}

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

<style>
  /* Ledger row — one line of record. Grid columns come from the
     shared --row-grid token (app.css) so the column header in
     App.svelte stays aligned. Status meaning lives in the status
     column + meter; no per-row cards, shadows or colored borders. */
  .drow {
    display: grid;
    grid-template-columns: var(--row-grid);
    align-items: center;
    column-gap: 10px;
    padding: 0 12px;
    min-height: var(--size-row);
    background: transparent;
    border-bottom: 1px solid var(--color-border-2);
    transition: background 0.12s var(--easing-out);
    cursor: pointer;
  }
  .drow:hover {
    background: var(--color-surface-hover);
    /* No `transform` — it makes the menu trigger move on hover, which
       (via the DropdownMenu's scroll handler) re-anchors the menu and
       produces a visible two-position jump. */
  }
  .drow.compact {
    min-height: var(--size-row-compact);
  }
  .drow.expanded {
    background: var(--color-surface);
  }
  .drow.selected {
    background: var(--color-primary-dim);
    box-shadow: inset 2px 0 0 var(--color-primary);
  }

  /* Checkbox look comes from the shared `input.cb` block in app.css
     (14px + SVG check) — only the layout slot lives here. */
  .cb {
    justify-self: center;
  }

  .name {
    font-size: 13px;
    font-weight: 500;
    color: var(--color-text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  /* ── Numeric columns — right-aligned tabular readouts ─── */
  .col {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    color: var(--color-muted);
    text-align: right;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dim {
    color: var(--color-faint);
  }
  .streams {
    color: var(--color-faint);
    margin-left: 3px;
    font-size: 11px;
  }

  .c-progress {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .c-progress :global(.bar) {
    flex: 1;
    min-width: 0;
  }
  .pct {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    color: var(--color-muted);
    width: 44px;
    text-align: right;
    flex: none;
  }
  /* The meter and the status label already carry the accent on a live
     row. A third accent number turns the row into a highlighter; the
     percentage steps up to primary text instead. */
  .drow.status-downloading .pct,
  .drow.status-connecting .pct {
    color: var(--color-text);
  }

  /* ── Status column ───────────────────────────────────── */
  .c-status {
    display: flex;
    align-items: center;
    gap: 6px;
    justify-content: flex-start;
    min-width: 0;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex: none;
    background: var(--color-faint);
  }
  .dot.live {
    background: var(--color-primary);
    animation: pulse 2s ease-in-out infinite;
  }
  .status-downloading .dot { background: var(--color-primary); }
  .status-connecting .dot { background: var(--color-info); }
  .status-completed .dot { background: var(--color-success); }
  .status-error .dot { background: var(--color-danger); }
  .status-queued .dot { background: var(--color-warning); }
  .status-scheduled .dot { background: var(--color-info); }
  .status-paused .dot, .status-canceled .dot { background: var(--color-faint); }

  .st-label {
    font-size: 11px;
    color: var(--color-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status-downloading .st-label { color: var(--color-primary); }
  .status-error .st-label { color: var(--color-danger); }
  /* Finished rows recede: the green dot carries the meaning, the
     label stays quiet so live rows are the only thing that pops. */
  .status-completed .st-label { color: var(--color-faint); }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.45; }
  }

  /* ── Hover actions ───────────────────────────────────── */
  .c-actions {
    display: flex;
    align-items: center;
    gap: 1px;
    justify-content: flex-end;
    opacity: 0;
    /* No `transform` — same DropdownMenu re-anchor reason as above. */
    transition: opacity 0.15s;
    position: relative;
  }
  .drow:hover .c-actions, .drow:focus-within .c-actions, .drow.expanded .c-actions {
    opacity: 1;
  }
  .action-btn {
    width: 24px;
    height: 24px;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--color-muted);
    cursor: pointer;
    display: grid;
    place-items: center;
    transition:
      background 0.12s var(--easing-out),
      color 0.12s var(--easing-out);
  }
  .action-btn:hover {
    background: var(--color-surface-active);
    color: var(--color-text);
  }
  .action-btn.primary {
    color: var(--color-primary);
  }
  .action-btn.primary:hover {
    background: var(--color-primary-dim);
  }
  .action-btn svg {
    width: 13px;
    height: 13px;
  }
  .action-btn.expand svg {
    transition: transform 0.15s var(--easing-out);
  }
  .action-btn.expand svg.rot {
    transform: rotate(180deg);
  }

  /* ── Expanded detail panel (spans all columns) ───────── */
  .expanded-panel {
    grid-column: 1 / -1;
    /* Align the panel's left edge with the filename column:
       12 (row pad) + 18 (checkbox) + 10 + 28 (tile) + 10. */
    padding: 4px 0 14px 78px;
    border-top: 1px solid var(--color-border-2);
    margin-top: -1px;
    cursor: default;
  }

  .ex-controls {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    padding: 8px 0 4px;
    max-width: 720px;
  }
  /* Keep the inline speed-limit input readout-sized instead of
     stretching to fill the panel. */
  .ex-controls :global(input) {
    width: 120px;
    flex: none;
  }
  .priority {
    height: 24px;
    width: 74px;
    padding: 0 18px 0 7px;
    background-color: transparent;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-muted);
    font-family: var(--font-mono);
    font-size: 11px;
    cursor: pointer;
    outline: none;
    appearance: none;
    -webkit-appearance: none;
    background-image: var(--select-chevron);
    background-repeat: no-repeat;
    background-position: right 3px center;
    background-size: 9px 9px;
  }
  .priority:focus {
    border-color: var(--color-border-focus);
    box-shadow: var(--shadow-glow);
  }
  .priority.hi {
    border-color: var(--color-warning-strong);
    color: var(--color-warning);
  }
  .priority.lo {
    color: var(--color-faint);
  }
  .priority option {
    background: var(--color-surface);
    color: var(--color-text);
  }
  .ex-error-inline {
    color: var(--color-danger);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 1;
  }

  .ex-section { margin: 10px 0 12px; }
  .ex-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--color-muted);
    margin-bottom: 6px;
  }
  .ex-count {
    margin-left: 6px;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-weight: 500;
    color: var(--color-text);
  }
  .ex-note {
    margin-left: 6px;
    font-size: 11px;
    font-weight: 500;
    color: var(--color-primary);
  }
  .seg-map {
    display: flex;
    gap: 1px;
    height: 14px;
    max-width: 720px;
  }
  .seg-cell {
    flex: 1 1 0;
    min-width: 0;
    background: var(--color-bg-active);
    border-radius: 1px;
    overflow: hidden;
  }
  .seg-cell i {
    display: block;
    height: 100%;
    background: var(--color-primary);
    transition: width 0.25s var(--easing-out);
  }
  .seg-cell.active {
    /* Tint the bucket, keep its fill readable. The old rule painted
       the whole cell accent at 45% opacity, which faded the fill it
       was supposed to highlight. */
    background: var(--color-primary-dim);
  }
  .seg-legend { margin-top: 6px; }
  .chunks {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 6px;
    max-width: 860px;
  }
  .chunk {
    background: var(--color-bg-active);
    border: 1px solid var(--color-border-2);
    border-radius: var(--radius-sm);
    padding: 6px 8px;
  }
  .chunk-head {
    display: flex;
    justify-content: space-between;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    color: var(--color-muted);
    margin-bottom: 4px;
  }
  /* "unknown total" hint shown next to the running byte count for
     open-ended chunks (where the proxy didn't relay Content-Length
     and the engine uses a single chunk with end = u64::MAX). The
     size column is hidden in that case so the user never sees
     "879 B / 16777216.0 TB" or similar sentinel-leak. */
  .open-ended-hint {
    color: var(--color-faint);
    font-style: italic;
    margin-left: 4px;
  }
  .chunk-bar {
    height: 3px;
    background: var(--color-track);
    border-radius: 1px;
    overflow: hidden;
  }
  .chunk-fill {
    height: 100%;
    background: var(--color-primary);
    transition: width 0.25s var(--easing-out);
  }

  .ex-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 6px 14px;
    margin-top: 6px;
    max-width: 860px;
  }
  .ex-stat {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .ex-stat .k {
    font-size: 11px;
    font-weight: 500;
    color: var(--color-faint);
  }
  .ex-stat .v {
    font-size: 12px;
    color: var(--color-text);
  }
  .ex-stat .v.mono {
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ex-error {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 10px;
    padding: 8px 10px;
    max-width: 860px;
    background: var(--color-danger-dim);
    border: 1px solid var(--color-danger-strong);
    border-radius: var(--radius-sm);
    color: var(--color-danger);
    font-size: 12px;
  }

  .hint {
    color: var(--color-faint);
    font-size: 12px;
  }

  /* Drop the least critical columns as the window narrows. */
  @media (max-width: 1180px) {
    .c-eta { display: none; }
    .drow { grid-template-columns: var(--row-grid-no-eta); }
  }
  @media (max-width: 980px) {
    .c-speed, .streams { display: none; }
    .drow { grid-template-columns: var(--row-grid-no-eta-no-speed); }
  }

  @media (max-width: 640px) {
    .expanded-panel { padding-left: 42px; }
  }

  @media (prefers-reduced-motion: reduce) {
    .dot.live { animation: none; }
  }
</style>
