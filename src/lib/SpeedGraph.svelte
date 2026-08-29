<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import uPlot from "uplot";
  import "uplot/dist/uPlot.min.css";

  // Current aggregate speed in bytes/sec, supplied by the parent on each
  // progress event. The component samples it once per second into a rolling
  // window and renders the history with uPlot.
  export let speed = 0;

  let el: HTMLDivElement;
  let plot: uPlot | undefined;
  let timer: ReturnType<typeof setInterval> | undefined;

  const MAX_POINTS = 120;
  const xs: number[] = [];
  const ys: number[] = [];

  function fmtRate(bps: number): string {
    const kb = bps / 1024;
    if (kb < 1024) return kb.toFixed(1) + " KB/s";
    return (kb / 1024).toFixed(2) + " MB/s";
  }

  onMount(() => {
    const width = el.clientWidth || 600;
    const opts: uPlot.Options = {
      width,
      height: 140,
      cursor: { show: false },
      scales: { x: { time: false } },
      legend: { show: false },
      series: [
        {},
        {
          label: "Speed",
          stroke: "#4f8cff",
          width: 2,
          value: (_u: uPlot, v: number | null) => (v == null ? "--" : fmtRate(v)),
        },
      ],
      axes: [
        { show: false },
        {
          size: 52,
          values: (_u: uPlot, vals: (string | number | null)[]) =>
            vals.map((v) => fmtRate(Number(v))),
        },
      ],
    };
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
    if (timer) clearInterval(timer);
    plot?.destroy();
    plot = undefined;
  });
</script>

<div class="speed-graph">
  <div class="label">Aggregate speed: <strong>{fmtRate(speed)}</strong></div>
  <div class="plot" bind:this={el}></div>
</div>

<style>
  .speed-graph {
    margin: 12px 0 4px;
  }
  .label {
    color: var(--muted);
    font-size: 13px;
    margin-bottom: 4px;
  }
  .plot {
    width: 100%;
  }
</style>
