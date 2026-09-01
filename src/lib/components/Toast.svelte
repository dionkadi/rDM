<script lang="ts">
  import { fly, fade } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { flip } from "svelte/animate";
  import { toasts, dismissToast } from "../stores/ui";

  const ICONS = {
    success: "M20 6L9 17l-5-5",
    error: "M12 8v4m0 4h.01M4.93 19.07h14.14V4.93H4.93z",
    warning: "M12 9v4m0 4h.01M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z",
    info: "M12 16v-4m0-4h.01M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20z",
  };

  function handleAction(t: typeof $toasts[number]) {
    if (t.action) {
      t.action.handler();
      dismissToast(t.id);
    }
  }
</script>

<div class="toast-stack" role="region" aria-label="Notifications" aria-live="polite">
  {#each $toasts as t (t.id)}
    <div
      class="toast {t.kind}"
      role="status"
      in:fly={{ y: 20, duration: 220, easing: cubicOut }}
      out:fade={{ duration: 160 }}
      animate:flip={{ duration: 200 }}
    >
      <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d={ICONS[t.kind]} />
      </svg>
      <div class="body">
        <div class="title">{t.title}</div>
        {#if t.message}<div class="msg">{t.message}</div>{/if}
      </div>
      {#if t.action}
        <button class="action" on:click={() => handleAction(t)}>
          {t.action.label}
        </button>
      {/if}
      <button class="close" on:click={() => dismissToast(t.id)} aria-label="Dismiss">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><path d="M6 6l12 12M18 6L6 18"/></svg>
      </button>
    </div>
  {/each}
</div>

<style>
  .toast-stack {
    position: fixed;
    bottom: 24px;
    right: 24px;
    z-index: var(--z-toast);
    display: flex;
    flex-direction: column-reverse;
    gap: 10px;
    pointer-events: none;
    max-width: 420px;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 12px 14px;
    background: var(--color-bg-elevated);
    color: var(--color-text);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: var(--shadow-modal);
    backdrop-filter: blur(12px);
    min-width: 280px;
  }
  .ico {
    width: 18px;
    height: 18px;
    flex: none;
    margin-top: 1px;
  }
  .toast.success .ico { color: var(--color-success); }
  .toast.error .ico { color: var(--color-danger); }
  .toast.warning .ico { color: var(--color-warning); }
  .toast.info .ico { color: var(--color-info); }
  .body { flex: 1; min-width: 0; }
  .title {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--text);
  }
  .msg {
    font-size: 12.5px;
    color: var(--muted);
    margin-top: 2px;
    line-height: 1.4;
  }
  .action {
    background: none;
    border: none;
    color: var(--accent);
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 6px;
    transition: background 0.15s;
  }
  .action:hover { background: var(--color-primary-dim); }
  .close {
    background: none;
    border: none;
    color: var(--text-faint);
    cursor: pointer;
    padding: 4px;
    border-radius: 6px;
    display: grid;
    place-items: center;
    transition: all 0.15s;
  }
  .close:hover { color: var(--text); background: rgba(255, 255, 255, 0.05); }
</style>