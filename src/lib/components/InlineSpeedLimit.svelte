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
    title={value == null ? "No speed limit" : `Limited to ${label} MB/s — click to change`}
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
    {#if value != null}
      <span class="num">{label}<span class="u">M</span></span>
    {/if}
  </button>
{/if}

<style>
  .display {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 9px;
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--hair);
    border-radius: 7px;
    color: var(--muted);
    font-family: var(--font-mono);
    font-size: 11.5px;
    cursor: pointer;
    transition: all 0.15s;
  }
  .display:hover {
    color: var(--text);
    border-color: var(--border-strong);
    background: rgba(255, 255, 255, 0.05);
  }
  .display.limited {
    color: var(--color-warning);
    border-color: var(--color-warning-strong);
    background: var(--color-warning-dim);
  }
  .display.compact {
    padding: 3px 6px;
    font-size: 10.5px;
  }
  .display svg {
    width: 12px;
    height: 12px;
  }
  .num {
    font-weight: 600;
  }
  .u {
    margin-left: 1px;
    opacity: 0.7;
  }
  .editor {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 8px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--border-focus);
    border-radius: 7px;
    box-shadow: var(--shadow-glow);
  }
  .editor input {
    width: 50px;
    padding: 2px 4px;
    border: none;
    background: transparent;
    color: var(--text);
    font-family: var(--font-mono);
    font-size: 11.5px;
    outline: none;
  }
  .unit {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--text-faint);
  }
</style>