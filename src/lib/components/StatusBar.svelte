<script lang="ts">
  import { aggregateSpeed, counts, totalDownloaded } from "../stores/downloads";
  import { demoMode, globalSpeedLimitLabel } from "../stores/ui";
  import { fmtBytes, fmtRate } from "../utils/formatters";
  import SpeedGraph from "../SpeedGraph.svelte";
</script>

<!-- Single home for aggregate metrics + the throughput sparkline.
     Labels are UI text; only the values are readout mono. The old bar
     set the whole strip in mono, which made "active / total / done"
     look like terminal output and fought the ledger for attention. -->
<footer class="status-bar" aria-label="Download statistics">
  <div class="seg">
    <span class="dot" class:active={$counts.active > 0}></span>
    <span class="n">{$counts.active}</span>
    <span class="k">active</span>
  </div>
  <div class="divider"></div>
  <div class="seg">
    <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M12 4v11m0 0l-4-4m4 4l4-4"/>
    </svg>
    <span class="n">{$counts.active > 0 ? fmtRate($aggregateSpeed) : "—"}</span>
  </div>
  {#if $counts.active > 0}
    <div class="spark"><SpeedGraph speed={$aggregateSpeed} /></div>
  {/if}
  <div class="divider"></div>
  <div class="seg">
    <span class="n">{fmtBytes($totalDownloaded)}</span>
    <span class="k">downloaded</span>
  </div>
  <div class="divider"></div>
  <div class="seg">
    <span class="k">limit</span>
    <span class="v">{$globalSpeedLimitLabel}</span>
  </div>
  <div class="spacer"></div>
  {#if $demoMode}
    <span class="demo">demo data</span>
    <div class="divider"></div>
  {/if}
  <div class="seg">
    <span class="n success">{$counts.completed}</span>
    <span class="k">done</span>
  </div>
  {#if $counts.errored > 0}
    <div class="seg">
      <span class="n danger">{$counts.errored}</span>
      <span class="k">errors</span>
    </div>
  {/if}
</footer>

<style>
  .status-bar {
    position: fixed;
    bottom: 0;
    left: 0;
    right: 0;
    z-index: var(--z-sticky);
    display: flex;
    align-items: center;
    gap: 9px;
    height: var(--size-statusbar);
    padding: 0 16px;
    background: var(--status-bar-bg);
    border-top: 1px solid var(--color-border);
    color: var(--color-text);
    font-family: var(--font-ui);
    font-size: 11px;
    white-space: nowrap;
  }
  .seg {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    white-space: nowrap;
  }
  .divider {
    width: 1px;
    height: 12px;
    background: var(--color-border);
    flex: none;
  }
  .k {
    color: var(--color-faint);
  }
  .v {
    color: var(--color-text);
  }
  .n {
    color: var(--color-text);
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
  .n.success { color: var(--color-success); }
  .n.danger { color: var(--color-danger); }
  .ico {
    width: 10px;
    height: 10px;
    color: var(--color-faint);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--color-faint);
    flex: none;
  }
  .dot.active {
    background: var(--color-primary);
  }
  .spark {
    width: 120px;
    height: 22px;
    overflow: hidden;
    flex: none;
    opacity: 0.9;
  }
  .demo {
    color: var(--color-warning);
    background: var(--color-warning-dim);
    border: 1px solid var(--color-warning-strong);
    border-radius: var(--radius-xs);
    padding: 1px 6px;
    font-size: 11px;
    white-space: nowrap;
  }
  .spacer { flex: 1; }

  @media (max-width: 720px) {
    .spark { display: none; }
  }
</style>
