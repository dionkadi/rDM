<script lang="ts">
  import { aggregateSpeed, counts, totalDownloaded } from "../stores/downloads";
  import { globalSpeedLimitLabel, sidebarCollapsed } from "../stores/ui";
  import { fmtBytes, fmtRate } from "../utils/formatters";
</script>

<footer class="status-bar" class:collapsed={$sidebarCollapsed} role="status" aria-label="Download statistics">
  <div class="seg">
    <span class="dot" class:active={$counts.active > 0}></span>
    <span class="k">Active</span>
    <span class="v">{$counts.active}</span>
  </div>
  <div class="divider"></div>
  <div class="seg">
    <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M12 4v11m0 0l-4-4m4 4l4-4"/>
    </svg>
    <span class="k">Down</span>
    <span class="v speed">{fmtRate($aggregateSpeed)}</span>
  </div>
  <div class="divider"></div>
  <div class="seg">
    <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <path d="M5 12h14M11 18l-6-6 6-6"/>
    </svg>
    <span class="k">Up</span>
    <span class="v muted">0 B/s</span>
  </div>
  <div class="divider"></div>
  <div class="seg">
    <span class="k">Total</span>
    <span class="v">{fmtBytes($totalDownloaded)}</span>
  </div>
  <div class="divider"></div>
  <div class="seg">
    <span class="k">Cap</span>
    <span class="v">{$globalSpeedLimitLabel}</span>
  </div>
  <div class="spacer"></div>
  <div class="seg">
    <span class="k">Completed</span>
    <span class="v success">{$counts.completed}</span>
  </div>
  {#if $counts.errored > 0}
    <div class="seg">
      <span class="k">Errors</span>
      <span class="v danger">{$counts.errored}</span>
    </div>
  {/if}
</footer>

<style>
  .status-bar {
    position: fixed;
    bottom: 0;
    left: var(--size-sidebar-expanded);
    right: 0;
    z-index: var(--z-sticky);
    display: flex;
    align-items: center;
    gap: 12px;
    height: var(--size-statusbar);
    padding: 0 18px;
    background: var(--status-bar-bg);
    backdrop-filter: blur(14px);
    border-top: 1px solid var(--color-border);
    color: var(--color-text);
    font-family: var(--font-mono);
    font-size: 11px;
    transition: left 0.25s cubic-bezier(0.4, 0, 0.2, 1), background 0.25s var(--easing-out);
  }
  .status-bar.collapsed {
    left: var(--size-sidebar-collapsed);
  }
  .seg {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .divider {
    width: 1px;
    height: 14px;
    background: var(--hair-2);
  }
  .k {
    color: var(--text-faint);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    font-size: 9.5px;
  }
  .v {
    color: var(--text);
    font-weight: 600;
  }
  .v.muted { color: var(--text-faint); }
  .v.speed { color: var(--color-success); }
  .v.success { color: var(--color-success); }
  .v.danger { color: var(--color-danger); }
  .ico {
    width: 11px;
    height: 11px;
    color: var(--text-faint);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-faint);
  }
  .dot.active {
    background: var(--accent);
    box-shadow: 0 0 8px var(--accent);
    animation: pulse 2s ease-out infinite;
  }
  .spacer { flex: 1; }
  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
  }
</style>