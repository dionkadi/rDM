<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import uPlot from "uplot";
  import "uplot/dist/uPlot.min.css";
  import { effectiveThemeStore } from "./stores/ui";

  // Current aggregate speed in bytes/sec, supplied by the parent. The
  // component samples it once per second into a rolling window and
  // renders it as a quiet sparkline (line only — the meter language of
  // docs/UI_DESIGN_PLAN.md).
  export let speed = 0;

  let el: HTMLDivElement;
  let plot: uPlot | undefined;
  let timer: ReturnType<typeof setInterval> | undefined;
  let unsubscribeTheme: (() => void) | undefined;

  const MAX_POINTS = 90;
  const xs: number[] = [];
  const ys: number[] = [];

  // uPlot paints into a <canvas>, so a colour captured at mount would
  // outlive the theme that produced it. `series.stroke` accepts a
  // function that uPlot re-evaluates on every draw; resolving the
  // custom property there keeps the line in sync with `<html
  // data-theme>` — for manual switches as well as an OS preference
  // flip in "system" mode. Both axes are hidden, so there is no grid
  // stroke to keep in sync (a grid config here used to be dead code).
  function cssColor(name: string, fallback: string): () => string {
    return () => getComputedStyle(el).getPropertyValue(name).trim() || fallback;
  }

  onMount(() => {
    const opts: uPlot.Options = {
      width: el.clientWidth || 240,
      height: 28,
      padding: [2, 0, 0, 0],
      cursor: { show: false },
      scales: { x: { time: false } },
      legend: { show: false },
      series: [
        {},
        {
          label: "Speed",
          stroke: cssColor("--color-primary", "#41c9a2"),
          width: 1.5,
          fill: undefined,
          points: { show: false },
        },
      ],
      axes: [{ show: false }, { show: false }],
    };
    // The colours are already live, but force a repaint on the switch
    // itself instead of waiting up to a second for the next sample.
    // Skip the store's initial emission and the empty-data case: the
    // first sample draws with the current theme on its own.
    unsubscribeTheme = effectiveThemeStore.subscribe(() => {
      if (xs.length > 0) plot?.redraw();
    });
    plot = new uPlot(opts, [[], []], el);

    timer = setInterval(() => {
      xs.push(xs.length);
      ys.push(speed);
      if (xs.length > MAX_POINTS) {
        xs.shift();
        ys.shift();
      }
      plot?.setData([xs.slice(), ys.slice()]);
    }, 1000);
  });

  onDestroy(() => {
    unsubscribeTheme?.();
    if (timer) clearInterval(timer);
    plot?.destroy();
    plot = undefined;
  });
</script>

<div class="speed-graph">
  <div class="plot" bind:this={el}></div>
</div>

<style>
  .speed-graph {
    width: 100%;
  }
  .plot {
    width: 100%;
    height: 28px;
  }
</style>
