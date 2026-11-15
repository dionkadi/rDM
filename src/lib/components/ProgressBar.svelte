<script lang="ts">
  import type { ChunkState, SegmentProgress } from "../types";

  export let progress: number = 0;        // 0-100
  export let status: string = "";
  export let chunks: ChunkState[] = [];   // for segmented display
  /**
   * HLS rows have no chunks (a segment is a whole request, not a byte
   * range) but they do have several fetches in flight. Drawing that
   * window just past the filled part is what makes the bar look alive
   * instead of frozen between segment boundaries.
   */
  export let segmentProgress: SegmentProgress | null = null;
  export let height: number = 6;

  $: hasSegments = chunks.length > 1;

  // The in-flight band sits at the end of the *byte* fill, not at the
  // segment fraction: the two are close but not identical (the total is
  // an estimate derived from segment durations), and a band that floats
  // away from the fill edge reads as a bug.
  $: fillPct = Math.min(100, Math.max(0, progress));
  $: activePct =
    segmentProgress && segmentProgress.total > 0 && segmentProgress.active > 0
      ? Math.min(
          100 - fillPct,
          Math.max(1.5, (segmentProgress.active / segmentProgress.total) * 100)
        )
      : 0;

  /**
   * Chunk boundaries as percentages of the whole file. The meter is one
   * continuous fill with hairline notches at these positions — the old
   * implementation drew one bar per chunk, which read as four separate
   * downloads with gaps between them. Per-chunk detail still lives in
   * the expanded panel; the row meter answers "how far along, and how
   * many connections".
   */
  $: tickPositions = (() => {
    if (!hasSegments) return [] as number[];
    const finite = chunks.filter(
      (c) => Number.isFinite(c.end) && c.end < Number.MAX_SAFE_INTEGER,
    );
    if (finite.length < 2) return [] as number[];
    const total = Math.max(...finite.map((c) => c.end + 1));
    if (!total) return [] as number[];
    const raw = finite
      .slice(0, -1)
      .map((c) => Math.min(99.5, Math.max(0.5, ((c.end + 1) / total) * 100)));
    // More than ~16 ticks in a 140 px column turns into moiré; sample
    // every nth boundary so the layout still reads as "many connections".
    const step = Math.ceil(raw.length / 16);
    return step > 1 ? raw.filter((_, i) => i % step === 0) : raw;
  })();
</script>

<div class="bar" style="height: {height}px" role="progressbar" aria-valuenow={progress} aria-valuemin="0" aria-valuemax="100" aria-label="Download progress">
  <div class="fill {status}" style="width: {fillPct}%"></div>
  {#if hasSegments}
    {#each tickPositions as t}
      <span class="tick" style="left: {t}%"></span>
    {/each}
  {/if}
  {#if activePct > 0}
    <div class="inflight" style="left: {fillPct}%; width: {activePct}%"></div>
  {/if}
</div>

<style>
  /* Flat instrument meter: one continuous accent fill, hairline ticks
     carved at chunk boundaries. No gradient / shimmer / glow. */
  .bar {
    width: 100%;
    background: var(--color-track);
    border-radius: 1.5px;
    overflow: hidden;
    position: relative;
  }
  .fill {
    height: 100%;
    background: var(--color-primary);
    transition: width 0.25s var(--easing-out);
    position: relative;
  }
  .fill.completed {
    background: var(--color-success);
    opacity: 0.55;
  }
  .fill.error, .fill.canceled {
    background: var(--color-danger);
    opacity: 0.6;
  }
  .fill.paused {
    background: var(--color-faint);
  }
  .tick {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--color-bg);
    opacity: 0.9;
    pointer-events: none;
  }
  .inflight {
    position: absolute;
    top: 0;
    bottom: 0;
    background: var(--color-primary);
    opacity: 0.35;
    transition: left 0.25s var(--easing-out), width 0.25s var(--easing-out);
  }

  @media (prefers-reduced-motion: reduce) {
    .fill, .inflight { transition: none; }
  }
</style>
