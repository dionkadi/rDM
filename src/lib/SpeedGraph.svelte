<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import uPlot from "uplot";
  import "uplot/dist/uPlot.min.css";

  // Current aggregate speed in bytes/sec, supplied by the parent. The
  // component samples it once per second into a rolling window and renders the
  // history as a soft gradient area chart that matches the app's mint accent.
  export let speed = 0;

  let el: HTMLDivElement;
  let plot: uPlot | undefined;
  let timer: ReturnType<typeof setInterval> | undefined;

  const MAX_POINTS = 90;
  const xs: number[] = [];
  const ys: number[] = [];

  const ACCENT = "#7af0c8";

  function fmtRate(bps: number): string {
    const kb = bps / 1024;
    if (kb < 1024) return kb.toFixed(0) + " KB/s";
    const mb = kb / 1024;
    if (mb < 1024) return mb.toFixed(2) + " MB/s";
    return (mb / 1024).toFixed(2) + " GB/s";
  }

  onMount(() => {
    const width = el.clientWidth || 600;
    const opts: uPlot.Options = {
      width,
      height: 150,
      cursor: { show: false },
      scales: { x: { time: false } },
      legend: { show: false },
      series: [
        {},
        {
          label: "Speed",
          stroke: ACCENT,
          width: 2,
          fill: (u: uPlot) => {
            const ctx = u.ctx;
            const g = ctx.createLinearGradient(0, u.bbox.top, 0, u.bbox.top + u.bbox.height);
            g.addColorStop(0, "rgba(122,240,200,0.30)");
            g.addColorStop(1, "rgba(122,240,200,0)");
            return g;
          },
          points: { show: false },
          value: (_u: uPlot, v: number | null) => (v == null ? "--" : fmtRate(v)),
        },
      ],
      axes: [
        { show: false },
        {
          show: true,
          stroke: "rgba(255,255,255,0.28)",
          grid: { show: true, stroke: "rgba(255,255,255,0.05)", width: 1 },
          ticks: { show: false },
          font: "11px 'JetBrains Mono', monospace",
          size: 50,
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
  <div class="plot" bind:this={el}></div>
</div>

<style>
  .speed-graph {
    width: 100%;
  }
  .plot {
    width: 100%;
    height: 150px;
  }
  .plot :global(canvas) {
    border-radius: 12px;
  }
</style>
