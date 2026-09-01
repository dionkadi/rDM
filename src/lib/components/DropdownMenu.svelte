<script lang="ts" context="module">
  export interface MenuItem {
    label: string;
    icon?: string; // SVG path
    shortcut?: string;
    danger?: boolean;
    disabled?: boolean;
    separator?: boolean;
    action?: () => void;
  }
</script>

<script lang="ts">
  import { onMount, createEventDispatcher } from "svelte";
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";

  export let items: MenuItem[] = [];
  export let trigger: HTMLElement | null = null;
  export let align: "left" | "right" = "right";
  export let open: boolean = false;

  const dispatch = createEventDispatcher();
  let menuEl: HTMLDivElement;

  // Resolved pixel coords for `position: fixed`. We compute these **once**
  // when the menu opens (and again on resize / scroll while open). To
  // avoid the visible "two-position jump" that happened when the
  // browser micro-jitters the trigger's `getBoundingClientRect()` on
  // every reflow, `measure()` ignores movement below a small deadband.
  let pos = {
    top: 0,
    left: 0,
    placement: "down" as "down" | "up",
    caretX: 0,
  };
  // Last measured trigger rect (in viewport coords). Used by the
  // deadband check so we don't reposition on sub-pixel reflows.
  let lastTrigger: { top: number; left: number; width: number } | null = null;

  // Threshold (px) below which a re-measured trigger position is
  // considered noise. 2 px is enough to swallow font-loading shifts,
  // hover transform nudges (`.drow:hover { transform: translateX(2px) }`)
  // and fractional sub-pixel reflow jitter, while still letting the
  // menu track a real scroll in well under one frame.
  const DEADBAND_PX = 3;

  const MARGIN = 6;   // gap between trigger and menu
  const VIEWPORT_PAD = 8; // keep at least this much from any viewport edge

  function close() {
    dispatch("close");
  }

  function handleAction(item: MenuItem) {
    if (item.disabled) return;
    if (item.separator) return;
    item.action?.();
    close();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      trigger?.focus();
    }
  }

  function handleClickOutside(e: MouseEvent) {
    if (!open) return;
    if (menuEl?.contains(e.target as Node)) return;
    if (trigger?.contains(e.target as Node)) return;
    close();
  }

  // Read the menu's *natural* (unclamped) dimensions. We measure the
  // menu in a `requestAnimationFrame` callback so the browser has done
  // layout for the freshly-mounted element before we read `offsetHeight`
  // / `offsetWidth` — otherwise the first read can return `0` and the
  // menu would be positioned at the wrong `top` for a frame before the
  // next recompute corrects it (the visible "two-position jump").
  function measure() {
    if (!trigger || !menuEl) return;
    const t = trigger.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const menuH = menuEl.offsetHeight;
    const menuW = menuEl.offsetWidth;
    if (menuH === 0 || menuW === 0) {
      // Layout not ready yet (shouldn't happen with rAF, but be safe).
      return;
    }

    // Deadband: if the trigger hasn't moved more than DEADBAND_PX in
    // either axis since the last measurement, keep the current
    // position. Without this, hover-state transforms (e.g.
    // `.drow:hover { transform: translateX(2px) }`) and font-loading
    // reflows cause the menu to oscillate between two positions.
    if (lastTrigger) {
      const dTop = Math.abs(t.top - lastTrigger.top);
      const dLeft = Math.abs(t.left - lastTrigger.left);
      if (dTop < DEADBAND_PX && dLeft < DEADBAND_PX) {
        return;
      }
    }
    lastTrigger = { top: t.top, left: t.left, width: t.width };

    const maxAllowed = vh - 2 * VIEWPORT_PAD;
    const visibleH = Math.min(menuH, maxAllowed);

    // Vertical placement: prefer "down", flip to "up" if it would clip.
    const downTop = t.bottom + MARGIN;
    const upTop = t.top - MARGIN - visibleH;
    const fitsBelow = downTop + visibleH + VIEWPORT_PAD <= vh;
    const fitsAbove = upTop >= VIEWPORT_PAD;
    let placement: "down" | "up";
    let top: number;
    if (fitsBelow) {
      placement = "down";
      top = downTop;
    } else if (fitsAbove) {
      placement = "up";
      top = upTop;
    } else {
      const roomBelow = vh - downTop - VIEWPORT_PAD;
      const roomAbove = t.top - MARGIN - VIEWPORT_PAD;
      if (roomBelow >= roomAbove) {
        placement = "down";
        top = Math.max(VIEWPORT_PAD, downTop);
      } else {
        placement = "up";
        top = Math.max(VIEWPORT_PAD, upTop);
      }
    }

    // Horizontal placement: anchor to trigger; clamp into viewport.
    let left: number;
    if (align === "right") {
      // Right edge of menu = right edge of trigger.
      left = t.right - menuW;
    } else {
      left = t.left;
    }
    left = Math.max(VIEWPORT_PAD, Math.min(left, vw - menuW - VIEWPORT_PAD));

    // Caret horizontal position: align the tip with the centre of the
    // trigger button, clamped to the menu's body (so the tip never
    // pokes out past the rounded corners).
    const triggerCx = t.left + t.width / 2;
    const rawCaretX = triggerCx - left;
    const caretX = Math.max(14, Math.min(rawCaretX, menuW - 14));

    pos = { top, left, placement, caretX };
  }

  // Position the menu when it opens, in a `requestAnimationFrame` so
  // the freshly-mounted menu has real dimensions by the time we measure.
  function place() {
    // Reset the deadband tracker so a brand-new open always repositions.
    lastTrigger = null;
    requestAnimationFrame(() => {
      if (open) measure();
    });
  }
  // When the menu closes, reset `pos` so the next open doesn't briefly
  // render the menu at the old coordinates (which would be a visible
  // jump from the stale position to the freshly-measured one).
  $: if (open) place();
  $: if (!open) {
    pos = { top: 0, left: 0, placement: "down", caretX: 0 };
    lastTrigger = null;
    pointerOverMenu = false;
  }

  function handleResize() {
    if (open) place();
  }

  // rAF-throttled scroll handler. We **do** follow the row when the
  // user scrolls, but only on the next animation frame (not on every
  // scroll event), and only if the trigger has actually moved (see
  // the deadband in `measure()`). This avoids the visible two-position
  // oscillation that happens when the browser reflows on every paint
  // and the trigger's `getBoundingClientRect()` micro-jitters by a
  // sub-pixel amount.
  let scrollRaf = 0;
  function handleScroll() {
    if (!open) return;
    // Don't reposition while the pointer is inside the menu — the
    // user is about to click an item, and any jump here would feel
    // like the menu is "teleporting" away from their cursor.
    if (pointerOverMenu) return;
    if (scrollRaf) return;
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = 0;
      if (open) measure();
    });
  }

  // Pause scroll-driven repositioning while the pointer is over the
  // menu itself. Without this, if the user nudges the scroll wheel
  // while their cursor is sitting on a menu item, the menu re-anchors
  // to the row's new position *while the user is trying to click* —
  // which is the "two-position jump" the user reported.
  let pointerOverMenu = false;
  function handlePointerEnter() {
    pointerOverMenu = true;
  }
  function handlePointerLeave() {
    pointerOverMenu = false;
  }

  onMount(() => {
    document.addEventListener("click", handleClickOutside);
    document.addEventListener("keydown", handleKeydown);
    window.addEventListener("resize", handleResize);
    window.addEventListener("scroll", handleScroll, true);
    return () => {
      document.removeEventListener("click", handleClickOutside);
      document.removeEventListener("keydown", handleKeydown);
      window.removeEventListener("resize", handleResize);
      window.removeEventListener("scroll", handleScroll, true);
      if (scrollRaf) cancelAnimationFrame(scrollRaf);
    };
  });
</script>

{#if open}
  <div
    class="menu"
    class:flip-up={pos.placement === "up"}
    bind:this={menuEl}
    role="menu"
    style="top: {pos.top}px; left: {pos.left}px; --caret-x: {pos.caretX}px;"
    transition:fly={{ y: pos.placement === "up" ? 4 : -4, duration: 160, easing: cubicOut }}
    on:pointerenter={handlePointerEnter}
    on:pointerleave={handlePointerLeave}
  >
    <!-- Caret pointing at the trigger. A small CSS triangle makes the
         menu→trigger link visually obvious so the user doesn't think the
         menu is "stuck" to the wrong row. -->
    <span class="caret" aria-hidden="true"></span>
    {#each items as item}
      {#if item.separator}
        <div class="separator" role="separator"></div>
      {:else}
        <button
          class="item"
          class:danger={item.danger}
          class:disabled={item.disabled}
          role="menuitem"
          on:click={() => handleAction(item)}
          disabled={item.disabled}
        >
          {#if item.icon}
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d={item.icon} />
            </svg>
          {/if}
          <span class="lbl">{item.label}</span>
          {#if item.shortcut}
            <span class="kbd">{item.shortcut}</span>
          {/if}
        </button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .menu {
    position: fixed;
    z-index: var(--z-popover);
    min-width: 200px;
    max-height: calc(100vh - 16px);
    overflow-y: auto;
    padding: 6px;
    background: var(--color-bg-elevated);
    color: var(--color-text);
    border: 1px solid var(--color-border-strong);
    border-radius: 12px;
    box-shadow: var(--shadow-modal);
    backdrop-filter: blur(16px);
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  /* Caret / arrow pointing at the trigger button. Anchored to the
     right edge (the menu is right-aligned with the chevron), the
     caret is 8px square and matches the menu's border colour so it
     reads as a continuous surface. */
  .caret {
    position: absolute;
    right: auto;
    left: var(--caret-x, 0);
    transform: translateX(-50%);
    width: 0;
    height: 0;
    border-left: 6px solid transparent;
    border-right: 6px solid transparent;
    pointer-events: none;
  }
  .menu:not(.flip-up) .caret {
    top: -6px;
    border-bottom: 6px solid var(--color-border-strong);
  }
  .menu:not(.flip-up) .caret::after {
    content: "";
    position: absolute;
    top: 1px;
    left: -5px;
    width: 0;
    height: 0;
    border-left: 5px solid transparent;
    border-right: 5px solid transparent;
    border-bottom: 5px solid var(--color-bg-elevated);
  }
  .menu.flip-up .caret {
    bottom: -6px;
    border-top: 6px solid var(--color-border-strong);
  }
  .menu.flip-up .caret::after {
    content: "";
    position: absolute;
    bottom: 1px;
    left: -5px;
    width: 0;
    height: 0;
    border-left: 5px solid transparent;
    border-right: 5px solid transparent;
    border-top: 5px solid var(--color-bg-elevated);
  }
  .item {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 6px 10px;
    background: none;
    border: none;
    border-radius: 6px;
    color: var(--color-text);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    text-align: left;
    font-family: inherit;
    transition: background 0.12s;
  }
  .item:hover { background: var(--color-surface-active); }
  .item.danger { color: var(--color-danger); }
  .item.danger:hover { background: var(--color-danger-dim); }
  .item.disabled {
    color: var(--color-text-faint);
    cursor: not-allowed;
    opacity: 0.5;
  }
  .item svg {
    width: 14px;
    height: 14px;
    flex: none;
    color: var(--color-text-muted);
  }
  .item.danger svg { color: var(--color-danger); }
  .lbl { flex: 1; }
  .kbd {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--color-text-faint);
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 3px;
    padding: 1px 5px;
  }
  .separator {
    height: 1px;
    background: var(--color-border-2);
    margin: 3px 6px;
  }
</style>
