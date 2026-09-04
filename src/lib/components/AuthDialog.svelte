<script lang="ts">
  // Per-download authentication dialog. Triggered from the
  // download row's dropdown menu (the "Auth…" entry) so the
  // user can attach headers / cookies / credentials to a
  // single transfer without affecting the rest of the queue.
  //
  // Three kinds of auth are supported:
  //
  //  1. **HTTP Basic** — username + password, sent as
  //     `Authorization: Basic base64(user:pass)`.
  //  2. **HTTP Bearer** — token, sent as
  //     `Authorization: Bearer <token>`.
  //  3. **Custom headers** — Referer, User-Agent, Cookie,
  //     X-Requested-With, etc. Each is one row in the
  //     "Extra headers" table.
  //
  // The browser cookie import is the high-leverage path:
  // most login-walled downloads are gated by a session
  // cookie the user already has in their browser. The
  // "Import from browser…" button calls
  // `api.importBrowserCookies(...)` and shows the user a
  // checklist of matching cookies; the chosen ones are
  // injected as a `Cookie:` header (combined with anything
  // the user typed in the headers table).
  //
  // All auth data is in-memory only — a restart clears it.
  // That is intentional: auth secrets shouldn't sit in a
  // plain-text database file, and the use case is "I'm
  // downloading one file from a site that needs login",
  // not "every future download uses these credentials".

  import { onMount } from "svelte";
  import { fly, fade } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import type { Download, BrowserKind, CookieImportResult } from "../types";
  import * as api from "../api";
  import { showToast } from "../stores/ui";

  // ── Visibility ────────────────────────────────────────────
  // The dialog is a singleton controlled by the authStore
  // (a small writable holding the current download id, or
  // null when closed). The App.svelte component renders
  // this component once and toggles the store from the
  // DownloadRow's dropdown menu.
  //
  // The store is defined in `authTarget.ts` (a separate
  // module) rather than as a default-exported value from
  // this Svelte component, because Svelte's TypeScript
  // surface does not re-export named values from inside
  // the `<script>` block — only the default export (the
  // component class). A standalone `authTarget` module
  // is the standard Svelte workaround.
  import { authTarget, type AuthDialogTarget } from "./authTarget";
  let visible: AuthDialogTarget = null;
  $: visible = $authTarget;

  // ── Form state ────────────────────────────────────────────
  // Auth kind: "none" / "basic" / "bearer" / "digest".
  // "Digest" is not really implemented on the engine side
  // (it downgrades to Basic with a clear warning), so we
  // surface it as a labelled option but warn the user.
  let authKind: "none" | "basic" | "bearer" | "digest" = "none";
  let basicUser = "";
  let basicPass = "";
  let bearerToken = "";
  let digestUser = "";
  let digestPass = "";

  // Extra headers table: array of {key, value} pairs. The
  // engine stores them as a BTreeMap on the Download, so
  // duplicate keys are coalesced by the last write.
  type HeaderRow = { key: string; value: string };
  let headers: HeaderRow[] = [];

  // Cookie import flow
  let cookieImport: CookieImportResult | null = null;
  let cookieImporting = false;
  let cookieBrowser: BrowserKind = "firefox";
  let cookieHost = "";
  let cookiePathOverride = "";
  // Per-cookie checklist (selected cookies get into the
  // `Cookie:` header that we build when the user clicks
  // "Apply").
  let selectedCookieNames: Set<string> = new Set();

  let busy = false;
  // Track if anything actually changed so the Cancel-vs-X
  // button can be a no-op without writing back stale state
  // to the engine.
  let dirty = false;

  // Reset the form when a new download is targeted.
  $: if (visible) resetFromDownload(visible.download);
  function resetFromDownload(d: Download) {
    authKind = "none";
    basicUser = "";
    basicPass = "";
    bearerToken = "";
    digestUser = "";
    digestPass = "";
    // Start with the cookie row already there so the user
    // can just paste the Cookie header. The other rows are
    // blank by default — most users won't touch them.
    headers = [
      { key: "Referer", value: "" },
      { key: "User-Agent", value: "" },
      { key: "Cookie", value: "" },
    ];
    cookieImport = null;
    cookieImporting = false;
    selectedCookieNames = new Set();
    // Pre-fill the cookie-host filter from the URL so the
    // user can press "Import" without typing.
    try {
      cookieHost = new URL(d.url).host;
    } catch {
      cookieHost = "";
    }
    dirty = false;
  }

  function addHeaderRow() {
    headers = [...headers, { key: "", value: "" }];
    dirty = true;
  }
  function removeHeaderRow(i: number) {
    headers = headers.filter((_, idx) => idx !== i);
    dirty = true;
  }
  // Trim empty rows out of the actual command payload so
  // the engine doesn't have to filter them.
  $: cleanHeaders = headers
    .filter((h) => h.key.trim() !== "" && h.value !== "")
    .map((h) => [h.key.trim(), h.value] as [string, string]);

  async function importCookies() {
    if (cookieImporting) return;
    cookieImporting = true;
    try {
      const result = await api.importBrowserCookies(
        cookieBrowser,
        cookieHost,
        cookiePathOverride || undefined,
      );
      cookieImport = result;
      // Default to "all selected" so a single click on
      // "Apply" is the most common path.
      selectedCookieNames = new Set(result.cookies.map((c) => c.name));
      if (result.count === 0) {
        showToast({
          kind: "info",
          title: "No cookies found",
          message: cookieHost
            ? `No cookies for "${cookieHost}" in ${cookieBrowser}.`
            : `No cookies in ${cookieBrowser}.`,
        });
      } else {
        showToast({
          kind: "success",
          title: `Found ${result.count} cookie${result.count === 1 ? "" : "s"}`,
          message: cookieHost
            ? `For "${cookieHost}" in ${cookieBrowser}.`
            : `In ${cookieBrowser}.`,
        });
      }
    } catch (e) {
      showToast({ kind: "error", title: "Cookie import failed", message: String(e) });
      cookieImport = null;
    } finally {
      cookieImporting = false;
    }
  }

  async function pickCookieFile() {
    // Tauri file picker — useful when the browser is in a
    // non-standard location (snap, flatpak, AppImage, …).
    // The user picks a `Cookies` / `cookies.sqlite` file
    // directly; we hand the path to the engine.
    try {
      const picked = await openDialog({
        multiple: false,
        directory: false,
        title: "Choose the browser cookie database",
        filters: [
          { name: "SQLite databases", extensions: ["sqlite", "db"] },
          { name: "All files", extensions: ["*"] },
        ],
      });
      if (typeof picked === "string") {
        cookiePathOverride = picked;
        dirty = true;
      }
    } catch (e) {
      // `openDialog` throws when cancelled in some Tauri
      // versions; the user closing the file picker is not
      // an error.
      if (!String(e).toLowerCase().includes("cancel")) {
        showToast({ kind: "error", title: "File picker failed", message: String(e) });
      }
    }
  }

  function applyCookieImportToHeader() {
    if (!cookieImport) return;
    // Take the selected cookies (default = all) and build
    // a `Cookie:` header value. The engine applies the
    // header verbatim so we don't need to URL-encode here
    // (the browser-side cookies we read are already
    // URL-safe per RFC 6265).
    const picked = cookieImport.cookies.filter((c) =>
      selectedCookieNames.has(c.name),
    );
    if (picked.length === 0) {
      showToast({
        kind: "info",
        title: "No cookies selected",
        message: "Pick at least one cookie to apply.",
      });
      return;
    }
    const headerValue = picked
      .map((c) => `${c.name}=${c.value}`)
      .join("; ");
    // Replace any existing Cookie row so the user can re-
    // import without leaving stale cookies behind.
    headers = headers.map((h) =>
      h.key.trim().toLowerCase() === "cookie" ? { key: "Cookie", value: headerValue } : h,
    );
    if (!headers.some((h) => h.key.trim().toLowerCase() === "cookie")) {
      headers = [...headers, { key: "Cookie", value: headerValue }];
    }
    dirty = true;
  }

  /**
   * Toggle a cookie's membership in the `selectedCookieNames`
   * set. Extracted to a named function because the Svelte
   * template parser rejects inline TypeScript `as` casts
   * inside arrow functions — the cast lives here in the
   * `<script>` block where TypeScript is allowed. We assign
   * a fresh `Set` so Svelte's reactivity re-runs the
   * `class:checked` evaluation.
   */
  function toggleCookieSelection(name: string, checked: boolean) {
    if (checked) selectedCookieNames.add(name);
    else selectedCookieNames.delete(name);
    selectedCookieNames = new Set(selectedCookieNames);
  }

  /**
   * Bulk helpers for the cookie checklist. Extracted to
   * named functions for the same reason as
   * `toggleCookieSelection` — the Svelte template parser
   * chokes on `!` non-null assertions in inline arrow
   * bodies, so the implementation has to live in the
   * script block.
   */
  function selectAllCookies() {
    if (!cookieImport) return;
    selectedCookieNames = new Set(cookieImport.cookies.map((c) => c.name));
  }
  function deselectAllCookies() {
    selectedCookieNames = new Set();
  }

  function authPayload(): api.HeadersAuth {
    if (authKind === "none") return { kind: "none" as const };
    if (authKind === "basic")
      return { kind: "basic" as const, username: basicUser, password: basicPass };
    if (authKind === "bearer")
      return { kind: "bearer" as const, token: bearerToken };
    return { kind: "digest" as const, username: digestUser, password: digestPass };
  }

  async function apply() {
    if (!visible || busy) return;
    busy = true;
    try {
      // Build the headers BTreeMap from the table. The
      // engine filters out restricted keys (Host, Content-
      // Length, Accept-Encoding) and warns on invalid
      // header names / values.
      const headerMap: Record<string, string> = {};
      for (const [k, v] of cleanHeaders) headerMap[k] = v;
      await api.setDownloadAuth(visible.id, headerMap, authPayload());
      showToast({ kind: "success", title: "Auth applied" });
      close();
    } catch (e) {
      showToast({ kind: "error", title: "Failed to apply auth", message: String(e) });
    } finally {
      busy = false;
    }
  }

  function close() {
    if (busy) return;
    authTarget.set(null);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      apply();
    }
  }

  onMount(() => {
    // No-op: the form is reset on each `visible` change.
  });
</script>

<svelte:window on:keydown={onKeydown} />

{#if visible}
  <div class="backdrop" transition:fade={{ duration: 150 }} aria-hidden="true"></div>
  <div
    class="modal"
    role="dialog"
    aria-modal="true"
    aria-labelledby="auth-title"
    transition:fly={{ y: 12, duration: 200, easing: cubicOut }}
  >
    <header class="hd">
      <div class="icn" aria-hidden="true">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <rect x="3" y="11" width="18" height="11" rx="2"/>
          <path d="M7 11V7a5 5 0 0 1 10 0v4"/>
        </svg>
      </div>
      <div>
        <h2 id="auth-title">Auth &amp; headers</h2>
        <div class="src">{visible.download.filename || visible.download.url}</div>
      </div>
    </header>

    <div class="bd">
      <!-- ── Auth kind ─────────────────────────────────────── -->
      <div class="auth-kind">
        <label class="lbl">Authorization</label>
        <div class="seg">
          <button
            class="seg-btn"
            class:active={authKind === "none"}
            on:click={() => { authKind = "none"; dirty = true; }}
            type="button"
          >None</button>
          <button
            class="seg-btn"
            class:active={authKind === "basic"}
            on:click={() => { authKind = "basic"; dirty = true; }}
            type="button"
          >Basic</button>
          <button
            class="seg-btn"
            class:active={authKind === "bearer"}
            on:click={() => { authKind = "bearer"; dirty = true; }}
            type="button"
          >Bearer</button>
          <button
            class="seg-btn"
            class:active={authKind === "digest"}
            on:click={() => { authKind = "digest"; dirty = true; }}
            type="button"
            title="Digest auth not yet implemented — engine downgrades to Basic"
          >Digest</button>
        </div>
      </div>

      {#if authKind === "basic"}
        <div class="grid-2">
          <label class="field">
            <span class="lbl">Username</span>
            <input class="inp" type="text" bind:value={basicUser} on:input={() => dirty = true} autocomplete="off" />
          </label>
          <label class="field">
            <span class="lbl">Password</span>
            <input class="inp" type="password" bind:value={basicPass} on:input={() => dirty = true} autocomplete="off" />
          </label>
        </div>
      {:else if authKind === "bearer"}
        <label class="field">
          <span class="lbl">Token</span>
          <input class="inp" type="password" bind:value={bearerToken} on:input={() => dirty = true} placeholder="eyJhbGciOi…" autocomplete="off" />
        </label>
      {:else if authKind === "digest"}
        <div class="grid-2">
          <label class="field">
            <span class="lbl">Username</span>
            <input class="inp" type="text" bind:value={digestUser} on:input={() => dirty = true} autocomplete="off" />
          </label>
          <label class="field">
            <span class="lbl">Password</span>
            <input class="inp" type="password" bind:value={digestPass} on:input={() => dirty = true} autocomplete="off" />
          </label>
        </div>
        <div class="warn">
          <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 8v4m0 4h.01M4.93 19.07h14.14V4.93H4.93z"/></svg>
          Digest auth is downgraded to Basic by the engine in v1.
        </div>
      {/if}

      <!-- ── Extra headers table ───────────────────────────── -->
      <div class="hdr-section">
        <div class="hdr-head">
          <span class="lbl">Extra headers</span>
          <button class="mini" type="button" on:click={addHeaderRow}>+ row</button>
        </div>
        <div class="hdr-table">
          {#each headers as row, i (i)}
            <div class="hdr-row">
              <input
                class="inp k"
                type="text"
                bind:value={row.key}
                on:input={() => dirty = true}
                placeholder="Header-Name"
                spellcheck="false"
              />
              <input
                class="inp v"
                type="text"
                bind:value={row.value}
                on:input={() => dirty = true}
                placeholder="value"
                spellcheck="false"
              />
              <button class="rm" type="button" on:click={() => removeHeaderRow(i)} title="Remove row" aria-label="Remove row">
                <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2"><path d="M18 6L6 18M6 6l12 12"/></svg>
              </button>
            </div>
          {/each}
        </div>
        <p class="hint">
          Restricted headers (<code>Host</code>, <code>Content-Length</code>,
          <code>Accept-Encoding</code>) are filtered out by the engine.
        </p>
      </div>

      <!-- ── Cookie import ────────────────────────────────── -->
      <div class="cookie-section">
        <div class="hdr-head">
          <span class="lbl">Import cookies from browser</span>
        </div>
        <div class="cookie-controls">
          <select class="sel" bind:value={cookieBrowser} on:change={() => { dirty = true; cookieImport = null; }}>
            <option value="firefox">Firefox</option>
            <option value="chromium">Chromium (Linux only)</option>
          </select>
          <input
            class="inp grow"
            type="text"
            bind:value={cookieHost}
            on:input={() => { dirty = true; cookieImport = null; }}
            placeholder="Host filter (e.g. example.com)"
            spellcheck="false"
          />
          <button class="mini" type="button" on:click={pickCookieFile} title="Pick a non-default cookie file">
            File…
          </button>
          <button class="mini primary" type="button" on:click={importCookies} disabled={cookieImporting}>
            {cookieImporting ? "Reading…" : "Import"}
          </button>
        </div>
        {#if cookiePathOverride}
          <p class="hint mono">{cookiePathOverride}</p>
        {/if}

        {#if cookieImport}
          {#if cookieImport.cookies.length > 0}
            <div class="cookie-list">
              {#each cookieImport.cookies as c (c.name + "@" + c.host)}
                <label class="ck-row" class:checked={selectedCookieNames.has(c.name)}>
                  <input
                    type="checkbox"
                    checked={selectedCookieNames.has(c.name)}
                    on:change={(e) => toggleCookieSelection(c.name, e.currentTarget.checked)}
                  />
                  <span class="ck-name">{c.name}</span>
                  <span class="ck-value mono">{c.value.length > 60 ? c.value.slice(0, 60) + "…" : c.value}</span>
                  <span class="ck-host">{c.host}</span>
                  {#if c.secure}<span class="ck-secure" title="Secure-only">🔒</span>{/if}
                </label>
              {/each}
            </div>
            <div class="cookie-actions">
              <button class="mini" type="button" on:click={selectAllCookies}>Select all</button>
              <button class="mini" type="button" on:click={deselectAllCookies}>Select none</button>
              <div class="spacer"></div>
              <button class="mini primary" type="button" on:click={applyCookieImportToHeader}>
                Apply as Cookie header
              </button>
            </div>
          {:else}
            <p class="hint">No cookies matched.</p>
          {/if}
        {/if}
      </div>
    </div>

    <footer class="ft">
      <button class="btn ghost" type="button" on:click={close} disabled={busy}>Cancel</button>
      <div class="spacer"></div>
      <span class="dirty-tag" class:active={dirty}>●</span>
      <button class="btn primary" type="button" on:click={apply} disabled={busy}>
        {busy ? "Applying…" : "Apply"}
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
    width: min(640px, calc(100vw - 32px));
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
    max-width: 460px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
  .bd {
    padding: 14px 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .lbl {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .auth-kind {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .seg {
    display: inline-flex;
    border: 1px solid var(--color-border);
    border-radius: 8px;
    overflow: hidden;
    width: fit-content;
  }
  .seg-btn {
    background: var(--color-surface);
    color: var(--color-text-muted);
    border: none;
    padding: 6px 12px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.12s, color 0.12s;
  }
  .seg-btn + .seg-btn { border-left: 1px solid var(--color-border); }
  .seg-btn:hover:not(.active) { background: var(--color-surface-active); color: var(--color-text); }
  .seg-btn.active {
    background: var(--color-accent);
    color: #fff;
  }
  .field { display: flex; flex-direction: column; gap: 5px; }
  .grid-2 { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
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
  /* Reserved for future use; the cookie path override
   * line below uses the `.hint.mono` selector instead. */
  /* .inp.mono { font-family: var(--font-mono); font-size: 11.5px; } */
  .inp:focus, .sel:focus {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 3px var(--color-accent-dim);
  }
  .warn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: var(--color-warning-dim, rgba(255, 180, 0, 0.10));
    border: 1px solid var(--color-warning-strong, #ffb400);
    border-radius: 6px;
    color: var(--color-warning-strong, #ffb400);
    font-size: 11.5px;
  }
  .hdr-section, .cookie-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .hdr-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .hdr-table {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .hdr-row {
    display: grid;
    grid-template-columns: 160px 1fr 28px;
    gap: 6px;
    align-items: center;
  }
  .hdr-row .k { font-family: var(--font-mono); font-size: 11.5px; }
  .rm {
    background: transparent;
    border: 1px solid var(--color-border);
    color: var(--color-text-muted);
    border-radius: 6px;
    width: 28px;
    height: 28px;
    cursor: pointer;
    display: grid;
    place-items: center;
  }
  .rm:hover { color: var(--color-danger); border-color: var(--color-danger-strong); }
  .mini {
    background: var(--color-surface);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: 6px;
    padding: 5px 10px;
    font-size: 11.5px;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
    transition: background 0.12s, border-color 0.12s;
  }
  .mini:hover:not(:disabled) { background: var(--color-surface-active); border-color: var(--color-border-strong); }
  .mini:disabled { opacity: 0.5; cursor: not-allowed; }
  .mini.primary { background: var(--color-accent); color: #fff; border-color: transparent; }
  .mini.primary:hover:not(:disabled) { filter: brightness(1.08); }
  .hint {
    color: var(--color-text-muted);
    font-size: 11.5px;
    margin: 0;
  }
  .hint code {
    font-family: var(--font-mono);
    font-size: 10.5px;
    background: var(--color-surface);
    padding: 0 3px;
    border-radius: 3px;
  }
  .hint.mono { font-family: var(--font-mono); font-size: 10.5px; word-break: break-all; }
  .cookie-controls {
    display: flex;
    gap: 6px;
    align-items: stretch;
  }
  .cookie-controls .grow { flex: 1 1 0; min-width: 0; }
  .cookie-list {
    max-height: 180px;
    overflow: auto;
    border: 1px solid var(--color-border);
    border-radius: 7px;
    background: var(--color-surface);
    display: flex;
    flex-direction: column;
  }
  .ck-row {
    display: grid;
    grid-template-columns: 18px 100px 1fr 130px 18px;
    gap: 6px;
    align-items: center;
    padding: 5px 8px;
    border-bottom: 1px solid var(--color-border);
    font-size: 11.5px;
    cursor: pointer;
  }
  .ck-row:last-child { border-bottom: none; }
  .ck-row:hover { background: var(--color-surface-active); }
  .ck-name { font-weight: 600; color: var(--color-text); }
  .ck-value { color: var(--color-text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ck-host { color: var(--color-text-faint); font-family: var(--font-mono); font-size: 10.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ck-secure { color: var(--color-success); }
  .cookie-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .cookie-actions .spacer { flex: 1; }
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
  .btn.primary { background: var(--color-accent); color: #fff; border-color: transparent; }
  .btn.primary:hover:not(:disabled) { filter: brightness(1.08); }
  .dirty-tag {
    color: transparent;
    font-size: 14px;
    user-select: none;
  }
  .dirty-tag.active { color: var(--color-warning, #ffb400); }
</style>
