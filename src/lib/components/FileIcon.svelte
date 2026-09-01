<script lang="ts">
  import { extOf } from "../utils/formatters";

  export let filename: string = "";
  export let url: string = "";
  export let progress: number = 0; // 0-100
  export let status: string = "";
  export let size: number = 52;

  const HUES: Record<string, { code: string; hue: string }> = {
    mp4: { code: "VID", hue: "var(--color-category-video)" },
    mkv: { code: "VID", hue: "var(--color-category-video)" },
    webm: { code: "VID", hue: "var(--color-category-video)" },
    mov: { code: "VID", hue: "var(--color-category-video)" },
    avi: { code: "VID", hue: "var(--color-category-video)" },
    mp3: { code: "AUD", hue: "var(--color-category-audio)" },
    wav: { code: "AUD", hue: "var(--color-category-audio)" },
    flac: { code: "AUD", hue: "var(--color-category-audio)" },
    ogg: { code: "AUD", hue: "var(--color-category-audio)" },
    zip: { code: "ARC", hue: "var(--color-category-archive)" },
    rar: { code: "ARC", hue: "var(--color-category-archive)" },
    "7z": { code: "ARC", hue: "var(--color-category-archive)" },
    gz: { code: "ARC", hue: "var(--color-category-archive)" },
    tar: { code: "ARC", hue: "var(--color-category-archive)" },
    pdf: { code: "PDF", hue: "var(--color-danger)" },
    doc: { code: "DOC", hue: "var(--color-category-document)" },
    docx: { code: "DOC", hue: "var(--color-category-document)" },
    xls: { code: "DOC", hue: "var(--color-category-document)" },
    xlsx: { code: "DOC", hue: "var(--color-category-document)" },
    txt: { code: "TXT", hue: "var(--color-category-document)" },
    iso: { code: "IMG", hue: "var(--color-category-image)" },
    img: { code: "IMG", hue: "var(--color-category-image)" },
    dmg: { code: "DMG", hue: "var(--color-category-image)" },
    jpg: { code: "JPG", hue: "var(--color-category-image)" },
    jpeg: { code: "JPG", hue: "var(--color-category-image)" },
    png: { code: "PNG", hue: "var(--color-category-image)" },
    gif: { code: "GIF", hue: "var(--color-category-image)" },
    webp: { code: "WEBP", hue: "var(--color-category-image)" },
    svg: { code: "SVG", hue: "var(--color-category-image)" },
    exe: { code: "EXE", hue: "var(--color-category-binary)" },
    msi: { code: "MSI", hue: "var(--color-category-binary)" },
    appimage: { code: "BIN", hue: "var(--color-category-binary)" },
    deb: { code: "DEB", hue: "var(--color-category-binary)" },
    rpm: { code: "RPM", hue: "var(--color-category-binary)" },
    apk: { code: "APK", hue: "var(--color-category-binary)" },
  };

  $: ext = extOf(filename || url);
  $: icon = HUES[ext] || { code: (ext.slice(0, 3) || "FILE").toUpperCase(), hue: "var(--slate)" };
  $: isActive = status === "downloading" || status === "connecting";
  $: isDone = status === "completed";
</script>

<div
  class="icon"
  class:active={isActive}
  class:done={isDone}
  style="--hue: {icon.hue}; --size: {size}px;"
  aria-hidden="true"
>
  <svg class="ring" viewBox="0 0 52 52" width={size} height={size}>
    <circle class="bg-ring" cx="26" cy="26" r="24" />
    {#if isActive || isDone}
      <circle
        class="progress-ring"
        cx="26"
        cy="26"
        r="24"
        stroke-dasharray={`${(Math.min(progress, 100) / 100) * 150.8} 150.8`}
        transform="rotate(-90 26 26)"
      />
    {/if}
  </svg>
  <span class="code">{icon.code}</span>
</div>

<style>
  .icon {
    position: relative;
    width: var(--size);
    height: var(--size);
    border-radius: 14px;
    display: grid;
    place-items: center;
    font-family: var(--font-mono);
    font-weight: 700;
    letter-spacing: 0.02em;
    color: var(--hue);
    background: color-mix(in srgb, var(--hue) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--hue) 28%, transparent);
    box-shadow: 0 0 24px -10px var(--hue);
    flex: none;
    transition: box-shadow 0.3s, transform 0.2s;
  }
  .icon.active {
    box-shadow: 0 0 28px -6px var(--hue);
  }
  .icon.done {
    background: color-mix(in srgb, var(--color-success) 14%, transparent);
    border-color: color-mix(in srgb, var(--color-success) 30%, transparent);
    color: var(--color-success);
  }

  .ring {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .bg-ring {
    fill: none;
    stroke: color-mix(in srgb, var(--hue) 12%, transparent);
    stroke-width: 2;
  }
  .progress-ring {
    fill: none;
    stroke: currentColor;
    stroke-width: 2.2;
    stroke-linecap: round;
    transition: stroke-dasharray 0.4s cubic-bezier(0.4, 0, 0.2, 1);
  }

  .code {
    position: relative;
    font-size: calc(var(--size) * 0.22);
    z-index: 1;
  }
</style>