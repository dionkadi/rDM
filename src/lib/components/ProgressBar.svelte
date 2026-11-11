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
  export let showShimmer: boolean | undefined = undefined;

  $: isActive = status === "downloading" || status === "connecting";
  $: shimmer = showShimmer ?? isActive;
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

  // Per-chunk fill percentages (the chunked path); the HLS path uses
  // the `segmentProgress` prop instead — do not reuse the name.
  $: chunkFill = hasSegments
    ? chunks.map((c) => {
        const size = c.end - c.start + 1;
        return Math.min(100, (c.downloaded / size) * 100);
      })
    : [] as number[];
</script>

<div class="bar" style="height: {height}px" class:segmented={hasSegments} role="progressbar" aria-valuenow={progress} aria-valuemin="0" aria-valuemax="100" aria-label="Download progress">
  {#if hasSegments}
    {#each chunkFill as seg, i}
      <div class="seg" style="--pct: {seg}%">
        <div class="seg-fill" class:shimmer={isActive}></div>
        {#if i < chunkFill.length - 1}
          <div class="divider"></div>
        {/if}
      </div>
    {/each}
  {:else}
    <div
      class="fill {status}"
      class:shimmer={shimmer}
      style="width: {fillPct}%"
    ></div>
    {#if activePct > 0}
      <div class="inflight" style="left: {fillPct}%; width: {activePct}%"></div>
    {/if}
  {/if}
</div>

<style>
  .bar {
    width: 100%;
    background: rgba(255, 255, 255, 0.06);
    border-radius: 9999px;
    overflow: hidden;
    position: relative;
  }
  .fill {
    height: 100%;
    border-radius: 9999px;
    background: linear-gradient(90deg, var(--accent), var(--accent-2));
    box-shadow: 0 0 14px -2px rgba(122, 240, 200, 0.6);
    transition: width 0.35s cubic-bezier(0.4, 0, 0.2, 1);
    position: relative;
  }
  .fill.completed {
    background: linear-gradient(90deg, var(--color-success), #9af0c2);
    box-shadow: 0 0 14px -2px rgba(116, 224, 164, 0.6);
  }
  .fill.error, .fill.canceled {
    background: linear-gradient(90deg, var(--color-danger), #ffa6b4);
    box-shadow: 0 0 14px -2px rgba(255, 122, 143, 0.5);
  }
  .fill.paused {
    background: linear-gradient(90deg, var(--text-muted), var(--text-faint));
  }
  .inflight {
    position: absolute;
    top: 0;
    bottom: 0;
    background: var(--accent);
    opacity: 0.3;
    border-radius: 9999px;
    transition: left 0.3s cubic-bezier(0.4, 0, 0.2, 1), width 0.3s cubic-bezier(0.4, 0, 0.2, 1);
  }
  .fill.shimmer::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(100deg, transparent 20%, rgba(255, 255, 255, 0.55) 50%, transparent 80%);
    background-size: 220% 100%;
    animation: shimmer 1.6s linear infinite;
  }

  .bar.segmented {
    display: flex;
    gap: 1px;
    background: rgba(255, 255, 255, 0.04);
  }
  .seg {
    flex: 1;
    position: relative;
    background: rgba(255, 255, 255, 0.06);
    overflow: hidden;
  }
  .seg-fill {
    position: absolute;
    inset: 0;
    width: var(--pct);
    background: linear-gradient(90deg, var(--accent), var(--accent-2));
    box-shadow: 0 0 8px -1px rgba(122, 240, 200, 0.5);
    transition: width 0.3s cubic-bezier(0.4, 0, 0.2, 1);
  }
  .seg-fill.shimmer::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(100deg, transparent 20%, rgba(255, 255, 255, 0.5) 50%, transparent 80%);
    background-size: 200% 100%;
    animation: shimmer 1.4s linear infinite;
  }

  @keyframes shimmer {
    from { background-position: 220% 0; }
    to { background-position: -120% 0; }
  }

  @media (prefers-reduced-motion: reduce) {
    .fill.shimmer::after, .seg-fill.shimmer::after { animation: none; }
  }
</style>