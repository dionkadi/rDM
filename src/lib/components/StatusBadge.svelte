<script lang="ts">
  import type { DownloadStatus } from "../types";

  export let status: DownloadStatus;
  export let size: "sm" | "md" = "sm";

  const ICONS: Record<DownloadStatus, string> = {
    downloading: "M5 12h14",
    connecting: "M12 4v4m0 8v4M4 12h4m8 0h4",
    paused: "M8 5v14M16 5v14",
    completed: "M20 6L9 17l-5-5",
    error: "M12 8v4m0 4h.01M4.93 19.07h14.14V4.93H4.93z",
    canceled: "M6 6l12 12M18 6L6 18",
    queued: "M12 8v4l3 3",
    scheduled: "M8 2v4M16 2v4M3 10h18",
  };

  const LABELS: Record<DownloadStatus, string> = {
    downloading: "Downloading",
    connecting: "Connecting",
    paused: "Paused",
    completed: "Completed",
    error: "Error",
    canceled: "Canceled",
    queued: "Queued",
    scheduled: "Scheduled",
  };
</script>

<span class="badge status-{status} size-{size}" role="status" aria-label="Status: {LABELS[status]}">
  {#if status === "downloading"}
    <span class="dot pulse"></span>
  {:else}
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
      <path d={ICONS[status]} />
    </svg>
  {/if}
  <span class="lbl">{LABELS[status]}</span>
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border-radius: 9999px;
    font-weight: 600;
    letter-spacing: 0.02em;
    border: 1px solid var(--hair);
    background: rgba(255, 255, 255, 0.03);
    color: var(--muted);
    white-space: nowrap;
    user-select: none;
  }
  .size-sm { font-size: 11px; padding: 3px 9px 3px 8px; }
  .size-md { font-size: 12.5px; padding: 5px 12px 5px 10px; }
  .badge svg { width: 11px; height: 11px; }

  .status-downloading {
    color: var(--accent);
    border-color: var(--color-primary-strong);
    background: var(--color-primary-dim);
  }
  .status-connecting {
    color: var(--info);
    border-color: var(--color-info-strong);
    background: var(--color-info-dim);
  }
  .status-paused {
    color: var(--muted);
    border-color: var(--hair);
    background: rgba(255, 255, 255, 0.03);
  }
  .status-completed {
    color: var(--success);
    border-color: var(--color-success-strong);
    background: var(--color-success-dim);
  }
  .status-error {
    color: var(--danger);
    border-color: var(--color-danger-strong);
    background: var(--color-danger-dim);
  }
  .status-canceled {
    color: var(--text-faint);
    border-color: var(--hair);
    background: rgba(255, 255, 255, 0.02);
  }
  .status-queued {
    color: var(--warning);
    border-color: var(--color-warning-strong);
    background: var(--color-warning-dim);
  }
  .status-scheduled {
    color: var(--info);
    border-color: var(--color-info-strong);
    background: var(--color-info-dim);
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: currentColor;
    flex: none;
  }
  .dot.pulse {
    animation: pulse 1.6s cubic-bezier(0.4, 0, 0.6, 1) infinite;
  }
  @keyframes pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.4; transform: scale(0.85); }
  }
</style>