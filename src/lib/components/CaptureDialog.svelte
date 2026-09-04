<script lang="ts">
  // IDM-style "new download" dialog. Shown when the browser
  // extension forwards a URL to DM via the native-messaging host.
  //
  // The dialog is the *only* path that calls `addDownload` for
  // browser-captured URLs. The user gets to:
  //   - edit the suggested filename;
  //   - pick a category (or "Default" for the global save dir);
  //   - see the resolved save path (read-only, derived from the
  //     selected category — the engine's `save_dir_for(category)`
  //     is the source of truth);
  //   - optionally attach a speed limit;
  //   - optionally attach a checksum.
  //
  // "Download" confirms the URL into the engine queue; "Skip"
  // drops the capture silently. The dialog processes one capture
  // at a time — when the user clicks Download/Skip, the head of
  // `captureQueue` is popped, and the next capture (if any) shows
  // immediately. This is the same flow IDM and FDM use; users
  // expect to see one prompt per URL, not a stacked pile of
  // modals.

  import { onMount } from "svelte";
  import { fly, fade } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import type { Category, CapturedUrl } from "../types";
  import { captureQueue, popCapture, addDownload, refreshDownloads } from "../stores/downloads";
  import { settings, loadSettings } from "../stores/settings";
  import { showToast } from "../stores/ui";
  import { isUrl } from "../utils/formatters";

  /** Resolved save directory for a given category. `null` means
   *  "default" (the global save dir). */
  let resolvedSaveDir = "";

  // ── Form state ───────────────────────────────────────────
  let filename = "";
  let category = ""; // empty = no category (use default save dir)
  let speedLimit = "";
  let checksumAlgo = "sha256";
  let checksumExpected = "";
  let showChecksum = false;
  let busy = false;

  // Per-download Referer / User-Agent. The browser extension
  // forwards the source page's Referer and the browser's
  // current User-Agent as part of the `CapturedUrl` payload.
  // We pre-fill the fields with these values and let the
  // user confirm or edit them. `sendReferer` / `sendUserAgent`
  // default to `true` when the native host provided values
  // and `false` otherwise — most users want the Referer
  // forwarded, but a privacy-conscious user can opt out.
  let referer = "";
  let userAgent = "";
  let sendReferer = false;
  let sendUserAgent = false;

  // Source label for the dialog title. Maps the engine's
  // `CapturedUrl.source` to a human-readable string.
  const SOURCE_LABEL: Record<string, string> = {
    "browser-click": "Browser wants to download",
    "browser-save-as": "Browser saved link as",
    "browser-grab": "Page media grabbed",
    "native-host": "Browser extension",
    "unknown": "Browser extension",
  };

  // The head of the queue is what the dialog renders. When the
  // user clicks Download or Skip, we pop it and the reactive
  // statement re-binds `current` to the next capture.
  $: current = $captureQueue[0] ?? null;
  // Re-initialise form fields whenever a new capture arrives.
  $: if (current) {
    filename = current.suggestedFilename;
    category = autoSuggestCategory(current);
    speedLimit = "";
    checksumExpected = "";
    showChecksum = false;
    checksumAlgo = "sha256";
    referer = current.referer ?? "";
    userAgent = current.userAgent ?? "";
    // Default the toggles to on when the native host
    // actually provided a value; off otherwise. The user
    // can still toggle before clicking "Download".
    sendReferer = !!(current.referer && current.referer.length > 0);
    sendUserAgent = !!(current.userAgent && current.userAgent.length > 0);
  }

  // Compute the save path preview from the currently selected
  // category. This mirrors the engine's `save_dir_for()` so the
  // user sees exactly where the file will go before clicking
  // "Download". If `category` is empty, we fall back to the
  // global default.
  $: resolvedSaveDir = resolveSaveDir(category, $settings);

  /**
   * Suggest a category for a captured URL by matching the URL's
   * file extension against `settings.categories[*].extensions`.
   * First match wins; if nothing matches, returns "" (= default).
   */
  function autoSuggestCategory(c: CapturedUrl): string {
    if (!$settings) return "";
    const ext = c.suggestedFilename
      .split(".")
      .pop()
      ?.toLowerCase() ?? "";
    if (!ext) return "";
    for (const cat of $settings.categories) {
      if (
        Array.isArray(cat.extensions) &&
        cat.extensions.map((e) => e.toLowerCase()).includes(ext)
      ) {
        return cat.id;
      }
    }
    return "";
  }

  /**
   * Mirror of the engine's `save_dir_for(category)` — used to
   * render the save-path preview. Kept in sync by reading the
   * settings store; the engine is the source of truth at the
   * moment the user actually clicks "Download".
   */
  function resolveSaveDir(catId: string, s: typeof $settings): string {
    if (!s) return "";
    if (!catId) {
      // No category → use the global default save dir.
      return typeof s.defaultDirectory === "string"
        ? s.defaultDirectory
        : String(s.defaultDirectory ?? "");
    }
    const cat = s.categories.find((c: Category) => c.id === catId);
    if (!cat) {
      return typeof s.defaultDirectory === "string"
        ? s.defaultDirectory
        : String(s.defaultDirectory ?? "");
    }
    return cat.directory || String(s.defaultDirectory ?? "");
  }

  async function confirm() {
    if (!current || busy) return;
    if (!isUrl(current.url)) {
      showToast({
        kind: "error",
        title: "Invalid URL",
        message: "The captured URL is not a valid http(s) address.",
      });
      return;
    }
    if (!filename.trim()) {
      showToast({
        kind: "error",
        title: "Filename required",
        message: "Please enter a filename for this download.",
      });
      return;
    }
    busy = true;
    try {
      const limit = speedLimit.trim()
        ? Math.max(0, Math.round(Number(speedLimit) * 1024))
        : null;
      const checksum = showChecksum && checksumExpected.trim()
        ? { algorithm: checksumAlgo, expected: checksumExpected.trim() }
        : null;
      // Build the per-download headers from the Referer /
      // User-Agent form fields. We only include a header if
      // the user has the matching toggle on AND the value
      // is non-empty — the engine filters restricted keys
      // (Host, Content-Length, Accept-Encoding) and warns
      // on the rest, but we save a round-trip by not
      // sending empty / disabled headers at all.
      const headers: Record<string, string> = {};
      if (sendReferer && referer.trim()) headers["Referer"] = referer.trim();
      if (sendUserAgent && userAgent.trim()) headers["User-Agent"] = userAgent.trim();
      // `category === ""` means "use the default save dir" (the
      // engine's `save_dir_for(None)`). We pass null rather than
      // an empty string so the Rust side can pattern-match on
      // `Option<String>`.
      await addDownload({
        url: current.url,
        category: category || null,
        filename: filename.trim(),
        speedLimit: limit,
        checksum,
        headers: Object.keys(headers).length > 0 ? headers : null,
      });
      showToast({
        kind: "success",
        title: "Download added",
        message: filename.trim(),
      });
      popCapture();
      // Refresh the list so the new row shows up immediately.
      await refreshDownloads();
    } catch (e) {
      showToast({ kind: "error", title: "Failed to add", message: String(e) });
    } finally {
      busy = false;
    }
  }

  function skip() {
    if (busy) return;
    popCapture();
  }

  function skipAll() {
    if (busy) return;
    captureQueue.set([]);
  }

  onMount(() => {
    // Settings may not be loaded yet when the first capture
    // arrives. `loadSettings` is idempotent — calling it again
    // just refreshes the in-memory copy.
    if (!$settings) loadSettings();
  });

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      skip();
    } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      confirm();
    }
  }
</script>

<svelte:window on:keydown={onKeydown} />

{#if current}
  <!-- Backdrop. `role="button"` + `tabindex` make it dismissable
       with click *or* keyboard. We intentionally do NOT close the
       dialog on backdrop click — the user might miss a capture if
       they click the wrong pixel. The "Skip" button is the explicit
       dismiss path. -->
  <div class="backdrop" transition:fade={{ duration: 150 }} aria-hidden="true"></div>
  <div
    class="modal"
    role="dialog"
    aria-modal="true"
    aria-labelledby="cd-title"
    transition:fly={{ y: 12, duration: 200, easing: cubicOut }}
  >
    <header class="hd">
      <div class="icn" aria-hidden="true">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 4v11m0 0l-3.4-3.4M12 15l3.4-3.4"/>
          <path d="M5 19h14"/>
        </svg>
      </div>
      <div>
        <h2 id="cd-title">{SOURCE_LABEL[current.source] ?? "Browser extension"}</h2>
        <div class="src">{current.url}</div>
      </div>
      {#if $captureQueue.length > 1}
        <div class="more" title="Captures queued: {$captureQueue.length}">
          +{$captureQueue.length - 1}
        </div>
      {/if}
    </header>

    <div class="bd">
      <label class="field">
        <span class="lbl">Filename</span>
        <input
          class="inp"
          type="text"
          bind:value={filename}
          placeholder="filename"
          autocomplete="off"
          spellcheck="false"
        />
      </label>

      <label class="field">
        <span class="lbl">Category</span>
        <div class="cat-row">
          <select class="sel" bind:value={category}>
            <option value="">Default (no category)</option>
            {#each $settings?.categories ?? [] as cat}
              <option value={cat.id}>{cat.name}</option>
            {/each}
          </select>
          <span class="dir" title="Resolved save directory">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>
            </svg>
            <code>{resolvedSaveDir || "—"}</code>
          </span>
        </div>
      </label>

      <details class="adv" bind:open={showChecksum}>
        <summary>Advanced (optional speed limit / checksum / referer)</summary>
        <div class="adv-body">
          <label class="field">
            <span class="lbl">Speed limit (KB/s)</span>
            <input class="inp" type="number" min="0" placeholder="no limit" bind:value={speedLimit} />
          </label>
          <label class="field">
            <span class="lbl">Checksum</span>
            <div class="chk-row">
              <select class="sel" bind:value={checksumAlgo}>
                <option value="sha256">SHA-256</option>
                <option value="sha1">SHA-1</option>
                <option value="md5">MD5</option>
              </select>
              <input class="inp" type="text" placeholder="expected hex" bind:value={checksumExpected} spellcheck="false" />
            </div>
          </label>
          <label class="field">
            <span class="lbl">
              <label class="chk-inline">
                <input type="checkbox" bind:checked={sendReferer} />
                Send Referer from source page
              </label>
            </span>
            <input
              class="inp"
              type="text"
              bind:value={referer}
              placeholder="https://example.com/page-that-had-the-link"
              spellcheck="false"
              disabled={!sendReferer}
            />
          </label>
          <label class="field">
            <span class="lbl">
              <label class="chk-inline">
                <input type="checkbox" bind:checked={sendUserAgent} />
                Send browser User-Agent
              </label>
            </span>
            <input
              class="inp"
              type="text"
              bind:value={userAgent}
              placeholder="Mozilla/5.0 …"
              spellcheck="false"
              disabled={!sendUserAgent}
            />
          </label>
        </div>
      </details>
    </div>

    <footer class="ft">
      <button class="btn ghost" on:click={skipAll} disabled={busy || $captureQueue.length <= 1}>
        Skip all
      </button>
      <div class="spacer"></div>
      <button class="btn ghost" on:click={skip} disabled={busy}>
        Skip
      </button>
      <button class="btn primary" on:click={confirm} disabled={busy}>
        {busy ? "Adding…" : "Download"}
      </button>
    </footer>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(8, 10, 14, 0.55);
    backdrop-filter: blur(4px);
    z-index: var(--z-modal);
  }
  .modal {
    position: fixed;
    z-index: calc(var(--z-modal) + 1);
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(520px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
    overflow: auto;
    background: var(--color-bg-elevated);
    color: var(--color-text);
    border: 1px solid var(--color-border-strong);
    border-radius: 14px;
    box-shadow: var(--shadow-modal);
    display: flex;
    flex-direction: column;
  }
  .hd {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 18px;
    border-bottom: 1px solid var(--color-border);
  }
  .icn {
    width: 36px;
    height: 36px;
    flex: none;
    border-radius: 9px;
    background: var(--color-accent-dim);
    color: var(--color-accent);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .icn svg { width: 18px; height: 18px; }
  h2 {
    margin: 0;
    font-size: 14.5px;
    font-weight: 600;
    color: var(--color-text);
  }
  .src {
    font-size: 11.5px;
    color: var(--color-text-muted);
    margin-top: 2px;
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
  .more {
    margin-left: auto;
    padding: 3px 8px;
    background: var(--color-surface-active);
    border: 1px solid var(--color-border);
    border-radius: 999px;
    font-size: 11px;
    color: var(--color-text-muted);
    font-weight: 600;
  }
  .bd {
    padding: 14px 18px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .lbl {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .inp, .sel {
    width: 100%;
    padding: 8px 10px;
    background: var(--color-surface);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: 7px;
    font-size: 13px;
    font-family: inherit;
    outline: none;
    transition: border-color 0.12s, box-shadow 0.12s;
  }
  .inp:focus, .sel:focus {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 3px var(--color-accent-dim);
  }
  .cat-row {
    display: flex;
    gap: 8px;
    align-items: stretch;
  }
  .sel { flex: 1 1 0; min-width: 0; }
  .dir {
    flex: 0 1 auto;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 7px;
    color: var(--color-text-muted);
    font-size: 11.5px;
    max-width: 50%;
    overflow: hidden;
  }
  .dir svg { width: 13px; height: 13px; flex: none; }
  .dir code {
    font-family: var(--font-mono);
    color: var(--color-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .adv {
    border: 1px solid var(--color-border);
    border-radius: 8px;
    background: var(--color-surface);
  }
  .adv summary {
    list-style: none;
    cursor: pointer;
    padding: 8px 12px;
    font-size: 12px;
    color: var(--color-text-muted);
    user-select: none;
  }
  .adv summary::-webkit-details-marker { display: none; }
  .adv summary::before {
    content: "▸ ";
    display: inline-block;
    transition: transform 0.12s;
  }
  .adv[open] summary::before { content: "▾ "; }
  .adv-body {
    padding: 4px 12px 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .chk-row {
    display: flex;
    gap: 8px;
  }
  .chk-row .sel { flex: 0 0 100px; }
  .chk-row .inp { flex: 1 1 0; min-width: 0; }
  /* Inline checkbox + label pair used inside the
   * `.lbl` span of the Referer / User-Agent rows. The
   * outer label still owns the field, but the click
   * target for the toggle is the inline `<input type="checkbox">`
   * + its adjacent text. */
  .chk-inline {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    text-transform: none;
    letter-spacing: 0;
    color: var(--color-text);
    font-size: 12px;
    cursor: pointer;
  }
  .chk-inline input[type="checkbox"] {
    margin: 0;
    accent-color: var(--color-accent);
  }
  .ft {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--color-border);
    background: var(--color-surface);
    border-radius: 0 0 13px 13px;
  }
  .spacer { flex: 1; }
  .btn {
    padding: 7px 14px;
    border-radius: 7px;
    border: 1px solid var(--color-border);
    background: var(--color-bg-elevated);
    color: var(--color-text);
    font-size: 12.5px;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
    transition: background 0.12s, transform 0.04s;
  }
  .btn:hover:not(:disabled) { background: var(--color-surface-active); }
  .btn:active:not(:disabled) { transform: translateY(1px); }
  .btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn.ghost { background: transparent; }
  .btn.primary {
    background: var(--color-accent);
    color: #fff;
    border-color: transparent;
  }
  .btn.primary:hover:not(:disabled) { filter: brightness(1.08); }
</style>
