<script lang="ts">
  import { createEventDispatcher } from "svelte";

  export let value: number | null = null; // bytes/sec
  export let compact: boolean = false;

  const dispatch = createEventDispatcher();
  let editing = false;
  let inputValue = "";

  function open() {
    if (value == null) {
      inputValue = "";
    } else {
      inputValue = String(Math.round((value / (1024 * 1024)) * 10) / 10);
    }
    editing = true;
  }

  function commit() {
    const v = inputValue.trim();
    if (v === "") {
      dispatch("change", null);
    } else {
      const mb = Number(v);
      if (!isNaN(mb) && mb >= 0) {
        dispatch("change", Math.round(mb * 1024 * 1024));
      }
    }
    editing = false;
  }

  function cancel() {
    editing = false;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") commit();
    else if (e.key === "Escape") cancel();
  }

  $: label = value == null ? null : (value / (1024 * 1024)).toFixed(value < 1024 * 1024 ? 0 : 1);
</script>

{#if editing}
  <form
    class="editor"
    role="group"
    aria-label="Set speed limit"
    on:submit|preventDefault={commit}
  >
    <input
      type="number"
      min="0"
      step="0.1"
      placeholder="MB/s"
      bind:value={inputValue}
      on:blur={commit}
      on:keydown={handleKeydown}
    />
    <span class="unit">MB/s</span>
  </form>
{:else}
  <button
    class="display"
    class:compact
    class:limited={value != null}
    on:click={open}
    title={value == null ? "No speed limit — click to set one" : `Limited to ${label} MB/s — click to change`}
  >
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      {#if value != null}
        <path d="M12 14l4-4M12 14L8 10M12 14v7" />
        <path d="M3 3v18h18" />
      {:else}
        <path d="M3 3v18h18" />
        <path d="M12 7v4M12 17h.01" />
      {/if}
    </svg>
    <span class="num">
      {#if value != null}
        {label}<span class="u"> MB/s</span>
      {:else}
        No limit
      {/if}
    </span>
  </button>
{/if}

<style>
  .display {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding: 0 8px;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-muted);
    font-family: var(--font-mono);
    font-size: 11px;
    cursor: pointer;
    transition:
      background 0.12s var(--easing-out),
      border-color 0.12s var(--easing-out),
      color 0.12s var(--easing-out);
  }
  .display:hover {
    color: var(--color-text);
    border-color: var(--color-border-strong);
    background: var(--color-surface-hover);
  }
  .display.limited {
    color: var(--color-warning);
    border-color: var(--color-warning-strong);
    background: var(--color-warning-dim);
  }
  .display.compact {
    height: 22px;
    padding: 0 6px;
    font-size: 11px;
  }
  .display svg {
    width: 12px;
    height: 12px;
  }
  .num {
    font-weight: 500;
  }
  .u {
    margin-left: 1px;
    opacity: 0.65;
  }
  .editor {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 7px;
    background: var(--color-surface);
    border: 1px solid var(--color-primary);
    border-radius: var(--radius-sm);
    box-shadow: var(--shadow-glow);
  }
  .editor input {
    width: 48px;
    padding: 2px 4px;
    border: none;
    background: transparent;
    color: var(--color-text);
    font-family: var(--font-mono);
    font-size: 11px;
    outline: none;
  }
  .unit {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--color-faint);
  }
</style>