<script lang="ts">
  import { extOf } from "../utils/formatters";

  // Type marker for the ledger's icon column. Deliberately flat:
  // a square neutral tile with a three-letter code in the category
  // hue. Progress is the meter column's job, status is the status
  // column's job — this tile only answers "what kind of file".
  export let filename: string = "";
  export let url: string = "";
  export let size: number = 28;

  type Kind =
    | "video"
    | "audio"
    | "archive"
    | "document"
    | "image"
    | "binary"
    | "other";

  const TYPES: Record<string, { code: string; kind: Kind }> = {
    mp4: { code: "VID", kind: "video" },
    mkv: { code: "VID", kind: "video" },
    webm: { code: "VID", kind: "video" },
    mov: { code: "VID", kind: "video" },
    avi: { code: "VID", kind: "video" },
    flv: { code: "VID", kind: "video" },
    m3u8: { code: "HLS", kind: "video" },
    mp3: { code: "AUD", kind: "audio" },
    wav: { code: "AUD", kind: "audio" },
    flac: { code: "AUD", kind: "audio" },
    ogg: { code: "AUD", kind: "audio" },
    aac: { code: "AUD", kind: "audio" },
    m4a: { code: "AUD", kind: "audio" },
    zip: { code: "ARC", kind: "archive" },
    rar: { code: "ARC", kind: "archive" },
    "7z": { code: "ARC", kind: "archive" },
    gz: { code: "ARC", kind: "archive" },
    tar: { code: "ARC", kind: "archive" },
    tgz: { code: "ARC", kind: "archive" },
    bz2: { code: "ARC", kind: "archive" },
    pdf: { code: "PDF", kind: "document" },
    doc: { code: "DOC", kind: "document" },
    docx: { code: "DOC", kind: "document" },
    xls: { code: "XLS", kind: "document" },
    xlsx: { code: "XLS", kind: "document" },
    ppt: { code: "PPT", kind: "document" },
    pptx: { code: "PPT", kind: "document" },
    txt: { code: "TXT", kind: "document" },
    md: { code: "MD", kind: "document" },
    rtf: { code: "RTF", kind: "document" },
    jpg: { code: "JPG", kind: "image" },
    jpeg: { code: "JPG", kind: "image" },
    png: { code: "PNG", kind: "image" },
    gif: { code: "GIF", kind: "image" },
    webp: { code: "WEBP", kind: "image" },
    svg: { code: "SVG", kind: "image" },
    bmp: { code: "BMP", kind: "image" },
    // Disk images and installers are software, not pictures — the old
    // map painted ISO/DMG with the image hue.
    iso: { code: "ISO", kind: "binary" },
    img: { code: "IMG", kind: "binary" },
    dmg: { code: "DMG", kind: "binary" },
    exe: { code: "EXE", kind: "binary" },
    msi: { code: "MSI", kind: "binary" },
    appimage: { code: "APP", kind: "binary" },
    deb: { code: "DEB", kind: "binary" },
    rpm: { code: "RPM", kind: "binary" },
    apk: { code: "APK", kind: "binary" },
  };

  const HUES: Record<Kind, string> = {
    video: "var(--color-category-video)",
    audio: "var(--color-category-audio)",
    archive: "var(--color-category-archive)",
    document: "var(--color-category-document)",
    image: "var(--color-category-image)",
    binary: "var(--color-category-binary)",
    other: "var(--color-category-other)",
  };

  $: ext = extOf(filename || url);
  $: type = TYPES[ext] ?? { code: (ext.slice(0, 3) || "BIN").toUpperCase(), kind: "other" as Kind };
</script>

<div
  class="tile"
  style="--hue: {HUES[type.kind]}; --size: {size}px;"
  title={ext ? `.${ext}` : undefined}
  aria-hidden="true"
>
  <span class="code">{type.code}</span>
</div>

<style>
  .tile {
    width: var(--size);
    height: var(--size);
    border-radius: 4px;
    display: grid;
    place-items: center;
    background: var(--color-surface-hover);
    border: 1px solid var(--color-border);
    flex: none;
  }
  .code {
    font-family: var(--font-mono);
    font-weight: 600;
    /* 9.5 px at the 28 px row tile: below this the 3-letter code
       turns into texture. The tile used to be a 26 x 19 rectangle
       with 8.4 px type inside a 26 px grid column. */
    font-size: 9.5px;
    letter-spacing: 0.02em;
    color: var(--hue);
    line-height: 1;
  }
</style>
