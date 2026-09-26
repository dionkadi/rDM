// Downloads store — reactive state for the download list
// Downloads store — reactive state for the download list
import { writable, derived, get } from "svelte/store";
import type { CapturedUrl, Download, FrontendEvent } from "../types";
import * as api from "../api";

// ── Capture queue (IDM-style) ───────────────────────────────────
//
// When the browser extension forwards a URL, the Tauri side emits a
// `Captured` event on the `download-event` channel. We push it into
// `captureQueue`; the `CaptureDialog` component renders the most
// recent capture and lets the user confirm category / path /
// filename before the URL is queued. The queue is a small array
// (usually 0–2 items; rare to have more than one pending at a time)
// so we keep it simple — no separate "pending approvals" store.

export const captureQueue = writable<CapturedUrl[]>([]);
const seenCaptureNonces = new Set<string>();

export function enqueueCapture(c: CapturedUrl): void {
  // Dedupe: the same URL can arrive twice in quick succession
  // (e.g. the user double-clicks a link, the extension fires two
  // onCreated events, or two tabs both surface the same media).
  // The Rust side stamps each event with a monotonic nonce; if
  // we've already seen it, drop the duplicate.
  if (c.nonce && seenCaptureNonces.has(c.nonce)) return;
  if (c.nonce) seenCaptureNonces.add(c.nonce);
  // Cap the queue to 10 entries — the dialog shows the head, and
  // anything beyond 10 is almost certainly a misbehaving extension
  // flooding the channel. Old entries are dropped silently.
  captureQueue.update((q) => {
    const next = [...q, c];
    if (next.length > 10) next.shift();
    return next;
  });
}

export function popCapture(): void {
  captureQueue.update((q) => q.slice(1));
}

export function clearCaptures(): void {
  captureQueue.set([]);
}

// ── Core state ──────────────────────────────────────────────────────
export const downloads = writable<Download[]>([]);
export const speedMap = writable<Record<string, number>>({});
export const aggregateSpeed = writable(0);
export const isLoading = writable(false);
export const error = writable("");

// ── Live speed tracking ────────────────────────────────────────────
const prevMap: Record<string, number> = {};
let monitorTimer: ReturnType<typeof setInterval> | null = null;

export function startSpeedMonitor(): void {
  stopSpeedMonitor();
  monitorTimer = setInterval(() => {
    const next: Record<string, number> = {};
    let agg = 0;
    const list = get(downloads);
    for (const d of list) {
      if (d.status === "downloading" || d.status === "connecting") {
        const prev = prevMap[d.id] ?? d.downloaded;
        const s = Math.max(0, d.downloaded - prev);
        next[d.id] = s;
        agg += s;
      }
    }
    // Update prev map for next tick
    for (const d of list) prevMap[d.id] = d.downloaded;
    speedMap.set(next);
    aggregateSpeed.set(agg);
  }, 1000);
}

export function stopSpeedMonitor(): void {
  if (monitorTimer) clearInterval(monitorTimer);
  monitorTimer = null;
}

// ── CRUD operations ────────────────────────────────────────────────
export async function refreshDownloads(): Promise<void> {
  isLoading.set(true);
  try {
    const list = await api.listDownloads();
    // The server list is authoritative: if an id we were blocking for
    // `removedIds` came back, stop blocking it, otherwise a late event
    // for it would be swallowed forever.
    for (const d of list) removedIds.delete(d.id);
    // One-task pair view: hide the second halves (keeping their state
    // for progress folding) and fold their progress into the visible
    // first rows.
    const visible: Download[] = [];
    for (const d of list) {
      if (hiddenSeconds.has(d.id)) {
        hiddenRows.set(d.id, d);
        continue;
      }
      const partnerId = pairPartner.get(d.id);
      const hidden = partnerId ? hiddenRows.get(partnerId) : undefined;
      if (hidden) {
        const pending = pendingMerges.get(d.id);
        visible.push({
          ...d,
          filename: pending ? pending.mergedName : d.filename,
          downloaded: (d.downloaded || 0) + (hidden.downloaded || 0),
          totalSize:
            (d.totalSize || 0) + (hidden.totalSize || 0) > 0
              ? (d.totalSize || 0) + (hidden.totalSize || 0)
              : d.totalSize,
        });
        continue;
      }
      visible.push(d);
    }
    downloads.set(visible);
    error.set("");
  } catch (e) {
    error.set(String(e));
  } finally {
    isLoading.set(false);
  }
}

export async function addDownload(opts: {
  url: string;
  category?: string | null;
  filename?: string | null;
  speedLimit?: number | null;
  checksum?: { algorithm: string; expected: string } | null;
  /** Per-download HTTP headers (Referer, User-Agent, Cookie, …).
   *  In-memory only — a restart clears them. The Tauri command
   *  attaches them via the same `set_headers_auth` path that the
   *  AuthDialog uses. */
  headers?: Record<string, string> | null;
  /** Per-download auth spec (Basic / Bearer / Digest). In-memory
   *  only. */
  auth?: import("../api").HeadersAuth | null;
}): Promise<void> {
  try {
    await api.addDownload(opts);
    await refreshDownloads();
  } catch (e) {
    error.set(String(e));
    throw e;
  }
}

// ── Paired captures (DASH video + audio → one download) ─────────────
//
// The browser extension curates a grab down to ONE logical download;
// for a DASH page that is a *pair* of `.m4s` streams. The capture
// dialog confirms the pair once, both halves are queued here, and the
// merge runs automatically when both finish — the IDM contract of one
// grab → one (playable) file, with no manual "select two rows → Merge"
// step.

/** Queue both halves of a paired capture. Same category / headers /
 *  speed limit for both (they are halves of one download); filenames
 *  come from the engine's `suggest_filename` (the halves are
 *  temporary). Returns the created row ids. */
export interface PairMeta {
  title?: string | null;
  date?: string | null;
  artist?: string | null;
}

export async function addPairedDownloads(opts: {
  firstUrl: string;
  secondUrl: string;
  category?: string | null;
  speedLimit?: number | null;
  headers?: Record<string, string> | null;
  meta?: PairMeta | null;
}): Promise<{ firstId: string; secondId: string }> {
  const build = (url: string) => ({
    url,
    category: opts.category ?? null,
    filename: null,
    speedLimit: opts.speedLimit ?? null,
    checksum: null,
    headers: opts.headers ?? null,
  });
  try {
    const first = await api.addDownload(build(opts.firstUrl));
    const second = await api.addDownload(build(opts.secondUrl));
    await refreshDownloads();
    return { firstId: first.id, secondId: second.id };
  } catch (e) {
    error.set(String(e));
    throw e;
  }
}

/**
 * Pending auto-merges: pairs queued from one capture that must become
 * one file when both halves finish. Keyed by BOTH part ids (either
 * half's completion event can trigger the merge); `firstId` is the row
 * the merged file gets attached to, `mergedName` is what the user
 * typed in the capture dialog.
 */
const pendingMerges = new Map<
  string,
  { firstId: string; otherId: string; mergedName: string; meta?: PairMeta | null }
>();

// ── One-task pair display ───────────────────────────────────────────
//
// A confirmed pair queues TWO engine rows (the halves), but the list
// shows ONE task: the first row, whose progress is the sum of both
// halves. The second row is hidden (tracked here) and the pair
// dissolves when the merge completes. Persisted so restarting the app
// mid-download keeps both the single-task view and the auto-merge
// arming.
const PAIR_STORAGE_KEY = "dmPendingPairs";
const pairPartner = new Map<string, string>(); // both directions
const hiddenSeconds = new Set<string>(); // second-half ids (hidden rows)
const hiddenRows = new Map<string, Download>(); // last known hidden-half state

function persistPairs(): void {
  try {
    if (typeof localStorage === "undefined") return;
    const entries: {
      firstId: string;
      otherId: string;
      mergedName: string;
      meta?: PairMeta | null;
    }[] = [];
    for (const entry of pendingMerges.values()) {
      if (entry.firstId === entry.otherId) continue;
      // One record per pair (keyed by the first id).
      if (entries.some((e) => e.firstId === entry.firstId)) continue;
      entries.push({
        firstId: entry.firstId,
        otherId: entry.otherId,
        mergedName: entry.mergedName,
        meta: entry.meta ?? null,
      });
    }
    localStorage.setItem(PAIR_STORAGE_KEY, JSON.stringify(entries));
  } catch (_e) {
    // Storage unavailable (private mode) — the in-memory state still
    // covers this session.
  }
}

function restorePairs(): void {
  try {
    if (typeof localStorage === "undefined") return;
    const raw = localStorage.getItem(PAIR_STORAGE_KEY);
    if (!raw) return;
    const entries = JSON.parse(raw) as {
      firstId: string;
      otherId: string;
      mergedName: string;
      meta?: PairMeta | null;
    }[];
    for (const e of entries) {
      if (!e || !e.firstId || !e.otherId) continue;
      const entry = {
        firstId: e.firstId,
        otherId: e.otherId,
        mergedName: e.mergedName,
        meta: e.meta ?? null,
      };
      pendingMerges.set(e.firstId, entry);
      pendingMerges.set(e.otherId, entry);
      pairPartner.set(e.firstId, e.otherId);
      pairPartner.set(e.otherId, e.firstId);
      hiddenSeconds.add(e.otherId);
    }
  } catch (_e) {
    // Corrupt payload — start clean.
  }
}
restorePairs();

export function armAutoMerge(
  firstId: string,
  secondId: string,
  mergedName: string,
  meta?: PairMeta | null,
): void {
  const entry = {
    firstId,
    otherId: secondId,
    mergedName,
    meta: meta ?? null,
  };
  pendingMerges.set(firstId, entry);
  pendingMerges.set(secondId, entry);
  // One-task display: the second half is hidden; its progress folds
  // into the first row.
  pairPartner.set(firstId, secondId);
  pairPartner.set(secondId, firstId);
  hiddenSeconds.add(secondId);
  persistPairs();
}

function dissolvePair(firstId: string, secondId: string): void {
  pairPartner.delete(firstId);
  pairPartner.delete(secondId);
  hiddenSeconds.delete(secondId);
  hiddenRows.delete(secondId);
  persistPairs();
}

/** Fold the hidden half's progress into the visible first row. The
 * visible task always carries the FINAL name the user typed in the
 * capture dialog — never the engine's part filename. */
function foldPairProgress(secondId: string): void {
  const firstId = pairPartner.get(secondId);
  if (!firstId) return;
  const second = hiddenRows.get(secondId);
  const pending = pendingMerges.get(firstId);
  downloads.update((list) => {
    const i = list.findIndex((x) => x.id === firstId);
    if (i < 0) return list;
    const first = list[i];
    const combined: Download = { ...first };
    if (pending) combined.filename = pending.mergedName;
    if (second) {
      combined.downloaded = (first.downloaded || 0) + (second.downloaded || 0);
      combined.totalSize =
        (first.totalSize || 0) + (second.totalSize || 0) > 0
          ? (first.totalSize || 0) + (second.totalSize || 0)
          : first.totalSize;
      // While the hidden half is still moving, the task is not
      // "completed" — the first half finishing first must not flip the
      // single task to Completed and back.
      if (
        second.status === "downloading" ||
        second.status === "connecting"
      ) {
        combined.status = "downloading";
      }
      // Surface the hidden half's failure on the visible task: a pair
      // with a dead half can't merge.
      if (
        (second.status === "error" || second.status === "canceled") &&
        first.status !== "error"
      ) {
        combined.status = second.status === "error" ? "error" : first.status;
        combined.error = second.error ?? combined.error;
      }
    }
    const next = list.slice();
    next[i] = combined;
    return next;
  });
}

function dropPendingMerge(id: string): void {
  const entry = pendingMerges.get(id);
  if (!entry) return;
  pendingMerges.delete(entry.firstId);
  pendingMerges.delete(entry.otherId);
}

/** Fired from the event listener when a pending-merge part completes.
 *  Runs the merge exactly once, when BOTH halves are completed. */
async function maybeAutoMerge(id: string): Promise<void> {
  const entry = pendingMerges.get(id);
  if (!entry) return;
  const list = get(downloads);
  const first = list.find((d) => d.id === entry.firstId);
  // The hidden half never enters the visible list — read its last
  // known state from the pair registry.
  const other = hiddenRows.get(entry.otherId);
  if (!first || !other) return; // partner's event has not arrived yet
  if (first.status !== "completed" || other.status !== "completed") return;

  // Clear BEFORE awaiting so a duplicate event can't double-merge.
  pendingMerges.delete(entry.firstId);
  pendingMerges.delete(entry.otherId);
  // Snapshot the first part's on-disk file BEFORE the merge: the merge
  // retargets the row at the merged file, and the original part would
  // otherwise linger in the save directory next to the result.
  const firstPartPath = first.savePath;
  try {
    await api.mergeDownloads(
      entry.firstId,
      entry.otherId,
      entry.mergedName,
      entry.meta ?? null,
    );
    // The second half is a temporary fragment of this merge; the merged
    // file is now attached to the first row, so the leftover row (and
    // its part file) only adds noise. Move it to the OS trash —
    // recoverable, unlike a hard delete.
    await api.trashDownload(entry.otherId).catch(() => {});
    // The first half's original part file lingers for the same reason
    // (the row survives, retargeted at the merged file). Trash it too,
    // leaving exactly ONE file for the whole download.
    await api.trashPaths([firstPartPath]).catch(() => {});
    // The pair is done: dissolve the one-task display state so the
    // merged row stands alone.
    dissolvePair(entry.firstId, entry.otherId);
    // The halves' own completions were suppressed for the one-task
    // view — the merged file is the completion the user cares about.
    api
      .notify_on_complete(
        "Download Complete",
        `${entry.mergedName} is ready`,
      )
      .catch(() => {});
    await refreshDownloads();
    const ui = await import("./ui");
    ui.showToast({
      kind: "success",
      title: "Video merged",
      message: entry.mergedName,
      duration: 6000,
    });
  } catch (e) {
    // Both rows stay (the parts are intact); the user can retry via
    // the manual "Merge" bulk action after fixing the cause (usually
    // a missing ffmpeg).
    const ui = await import("./ui");
    ui.showToast({
      kind: "error",
      title: "Auto-merge failed",
      message: String(e),
      duration: 8000,
    });
  }
}

/** A pending-merge part that ended in a terminal failure state cancels
 *  the auto-merge: merging half a download produces nothing useful. */
function cancelAutoMergeFor(id: string, status: string): void {
  if (!pendingMerges.has(id)) return;
  const entry = pendingMerges.get(id)!;
  dropPendingMerge(id);
  const ui = import("./ui");
  void ui.then((m) =>
    m.showToast({
      kind: "error",
      title: "Auto-merge cancelled",
      message: `"${entry.mergedName}" will not be merged — one of its parts ended as ${status}.`,
      duration: 8000,
    }),
  );
}

/**
 * Paired downloads are ONE task in the UI but TWO engine rows; control
 * actions on the visible row must reach the hidden half as well, or a
 * pause would halt only the video while the audio keeps downloading.
 */
function withHiddenPartner(ids: string[]): string[] {
  const out = new Set<string>(ids);
  for (const id of ids) {
    const partner = pairPartner.get(id);
    if (partner && hiddenSeconds.has(partner)) out.add(partner);
  }
  return [...out];
}

export async function pauseDownload(id: string): Promise<void> {
  for (const target of withHiddenPartner([id])) await api.pauseDownload(target);
  await refreshDownloads();
}

export async function resumeDownload(id: string): Promise<void> {
  for (const target of withHiddenPartner([id])) await api.resumeDownload(target);
  await refreshDownloads();
}

export async function cancelDownload(id: string): Promise<void> {
  for (const target of withHiddenPartner([id])) await api.cancelDownload(target);
  await refreshDownloads();
}

export async function removeDownload(id: string): Promise<void> {
  for (const target of withHiddenPartner([id])) await api.removeDownload(target);
  await refreshDownloads();
}

export async function setDownloadSpeedLimit(
  id: string,
  limit: number | null,
): Promise<void> {
  await api.setSpeedLimit(id, limit);
  await refreshDownloads();
}

// ── Bulk operations ──────────────────────────────────────────────
//
// `Promise.allSettled` rather than `Promise.all`: a single bad id
// (e.g. a row that was removed in the same tick) shouldn't abort
// the whole batch and surface as a hard error. Instead we collect
// failures and report them via a single toast. One `refreshDownloads`
// at the end is enough — the engine's `StatusChanged` events will
// also have fired for each row, but the store-level merge handles
// dedupe.

/** Apply an action to a list of ids in parallel; return the
 *  number of successful applications and the first error message
 *  (if any). The caller decides whether the error warrants a toast. */
async function bulkApply(
  ids: readonly string[],
  action: (id: string) => Promise<void>,
): Promise<{ ok: number; failed: number; firstError: string | null }> {
  const results = await Promise.allSettled(ids.map((id) => action(id)));
  let ok = 0;
  let failed = 0;
  let firstError: string | null = null;
  for (const r of results) {
    if (r.status === "fulfilled") ok++;
    else {
      failed++;
      if (!firstError) firstError = String(r.reason);
    }
  }
  return { ok, failed, firstError };
}

export async function bulkPause(ids: readonly string[]): Promise<void> {
  const r = await bulkApply(withHiddenPartner([...ids]), (id) =>
    api.pauseDownload(id),
  );
  await refreshDownloads();
  if (r.failed > 0)
    throw new Error(`${r.ok} paused, ${r.failed} failed: ${r.firstError}`);
}

export async function bulkResume(ids: readonly string[]): Promise<void> {
  const r = await bulkApply(withHiddenPartner([...ids]), (id) =>
    api.resumeDownload(id),
  );
  await refreshDownloads();
  if (r.failed > 0)
    throw new Error(`${r.ok} resumed, ${r.failed} failed: ${r.firstError}`);
}

export async function bulkRemove(ids: readonly string[]): Promise<void> {
  const r = await bulkApply(withHiddenPartner([...ids]), (id) =>
    api.removeDownload(id),
  );
  await refreshDownloads();
  if (r.failed > 0)
    throw new Error(`${r.ok} removed, ${r.failed} failed: ${r.firstError}`);
}

/**
 * Move every selected download to the OS trash in parallel.
 * Each per-id call goes through `api.trashDownload`, which
 * (a) moves the on-disk file + `.part` to the OS trash and
 * (b) drops the SQLite row + in-memory entry. We deliberately
 * do **not** swallow individual failures — the user picked
 * these downloads on purpose and a partial trash is worse
 * than a loud error. The toast on success says "N moved to
 * Trash".
 */
export async function bulkTrash(ids: readonly string[]): Promise<void> {
  const r = await bulkApply(withHiddenPartner([...ids]), (id) =>
    api.trashDownload(id),
  );
  await refreshDownloads();
  if (r.failed > 0)
    throw new Error(`${r.ok} trashed, ${r.failed} failed: ${r.firstError}`);
}

export async function bulkSetLimit(
  ids: readonly string[],
  limit: number | null,
): Promise<void> {
  const r = await bulkApply(ids, (id) => setDownloadSpeedLimit(id, limit));
  await refreshDownloads();
  if (r.failed > 0)
    throw new Error(`${r.ok} updated, ${r.failed} failed: ${r.firstError}`);
}

// ── Reorder / priority ──────────────────────────────────────
//
// `reorder` calls the Rust `reorder_downloads` command, which
// assigns fresh `sort_key` values and emits per-row
// `StatusChanged` events. The events carry the new `sort_key`
// and the frontend store's merge handler picks them up — we
// don't need to manually re-sort or call `refreshDownloads`
// because the events are the source of truth.

export async function reorderDownloads(ids: string[]): Promise<void> {
  await api.reorderDownloads(ids);
}

export async function setDownloadPriority(
  id: string,
  priority: number,
): Promise<void> {
  await api.setDownloadPriority(id, priority);
}

// ── Event handling (Tauri live updates) ────────────────────────────
let unlistenFn: (() => void) | null = null;

/**
 * Ids the user has removed during this session.
 *
 * Defence in depth for the "canceled ghost" bug. `remove` drops the
 * task and emits `Removed`, but an in-flight chunk worker only notices
 * the cancel flag when it next polls — and its finalize path used to
 * persist `status = Canceled` and emit `StatusChanged` *after* the
 * `Removed`. The generic branch below inserts unknown ids, so that late
 * event recreated the row as a ghost which only a full list refresh
 * cleared (hence "it disappeared when I clicked Resume").
 *
 * The engine no longer emits or persists for a removed task (see
 * `DownloadControl::remove` and the finalize guard in `task.rs`); this
 * set means the UI stays correct even if some other path ever does.
 * `refreshDownloads` un-blocks any id the server still knows about, so
 * an id can't stay blocked after a legitimate re-add.
 */
const removedIds = new Set<string>();

/**
 * Rows whose completion has already been announced. The engine emits a
 * completed status more than once per row (the task's own `Completed`
 * event, then the merged file's retargeting `StatusChanged`) — without
 * this guard each emission became a separate OS notification
 * ("Download complete" spam).
 */
const notifiedComplete = new Set<string>();

export async function startEventListener(): Promise<void> {
  try {
    const { listen } = await import("@tauri-apps/api/event");
    unlistenFn = await listen<FrontendEvent>("download-event", (ev) => {
      const e = ev.payload;
      if (e.kind === "captured") {
        // Browser-extension capture: push into the capture queue
        // so the `CaptureDialog` component can show the
        // confirmation prompt. The dialog calls `addDownload()`
        // when the user clicks "Download", which is what actually
        // queues the task in the engine.
        // Rust serializes Captured flat (no "download" wrapper), so
        // construct CapturedUrl directly from the event payload.
        const captured: CapturedUrl = {
          source: e.source,
          url: e.url,
          pairSecond: e.pairSecond ?? null,
          metaTitle: e.metaTitle ?? null,
          metaDate: e.metaDate ?? null,
          metaArtist: e.metaArtist ?? null,
          suggestedFilename: e.suggestedFilename,
          defaultSaveDir: e.defaultSaveDir,
          referer: e.referer,
          userAgent: e.userAgent,
          nonce: e.nonce,
        };
        enqueueCapture(captured);
        // Tell the user the way they'll notice from wherever they are.
        // They just clicked a link in their browser, so this window is
        // almost always behind it and the CaptureDialog modal is
        // invisible to them — that is why "it doesn't notify" was the
        // complaint. When the window *is* focused the dialog plus the
        // in-app toast are feedback enough, so we skip the duplicate.
        if (typeof document !== "undefined" && !document.hasFocus()) {
          // Name the capture after the page's title when the grab could
          // read one — "41875472475-1-100022.m4s" tells nobody anything.
          api
            .notify(
              "New download captured",
              captured.metaTitle ||
                captured.suggestedFilename ||
                captured.url,
            )
            .catch(() => {});
        }
        return;
      }
      if (e.kind === "removed") {
        const removedId = e.id;
        removedIds.add(removedId);
        if (hiddenSeconds.has(removedId)) {
          // The hidden half's row was dropped engine-side (auto-merge
          // cleanup or control fan-out) — nothing visible to remove.
          hiddenRows.delete(removedId);
          hiddenSeconds.delete(removedId);
          const firstId = pairPartner.get(removedId);
          pairPartner.delete(removedId);
          if (firstId) pairPartner.delete(firstId);
          persistPairs();
          return;
        }
        downloads.update((list) => list.filter((d) => d.id !== removedId));
        // A removed row can be half of a pending auto-merge; there is
        // nothing left to merge for it. If the removed row was the
        // visible half of a pair, dissolve the whole pair (the
        // control fan-out below already removed the hidden half
        // engine-side).
        const removedPartner = pairPartner.get(removedId);
        if (removedPartner) dissolvePair(removedId, removedPartner);
        dropPendingMerge(removedId);
        // Lazy-import to avoid a circular dependency: `ui.ts` is
        // also imported by the download store, so reaching back
        // into it from a top-level `import` would be a cycle.
        import("./ui").then((ui) =>
          ui.pruneSelection(new Set(get(downloads).map((d) => d.id))),
        );
        return;
      }
      // For Added, Progress, StatusChanged, Completed, Error: download fields are flat
      // SAFETY: Rust serializes these variants flat (no "download" wrapper) due to
      // #[serde(tag = "kind", rename_all = "camelCase")]. The TypeScript type
      // matches this flat structure, so the cast is safe when kind is one of these.
      const d: Download = e as unknown as Download;
      // A late event for a row the user already removed must not
      // resurrect it. See `removedIds` above.
      if (removedIds.has(d.id)) return;
      // Hidden pair halves never enter the list: their progress folds
      // into the visible first row (one task in the UI), and their
      // completion drives the auto-merge. Notifications for halves are
      // suppressed — the merged row announces itself.
      if (hiddenSeconds.has(d.id)) {
        hiddenRows.set(d.id, d);
        if (d.status === "completed") {
          void maybeAutoMerge(d.id);
        } else if (d.status === "error" || d.status === "canceled") {
          cancelAutoMergeFor(d.id, d.status);
        }
        foldPairProgress(d.id);
        return;
      }
      // Merge the event into the list FIRST: the auto-merge trigger
      // below reads the store, and it must see this row's new status
      // (the second half's Completed event is what fires the merge).
      downloads.update((list) => {
        const i = list.findIndex((x) => x.id === d.id);
        if (i >= 0) {
          const next = list.slice();
          next[i] = d;
          return next;
        }
        return [d, ...list];
      });
      // Now that the store reflects `d`, run the paired-capture hooks.
      const isPairFirst = pairPartner.has(d.id);
      if (isPairFirst) {
        // Keep the single task's name/progress combined after every
        // engine event.
        const partner = pairPartner.get(d.id);
        if (partner) foldPairProgress(partner);
      }
      // Notify on completion — once per row, and never for a half of a
      // pending pair: the merged file announces itself when the merge
      // succeeds.
      if (d.status === "completed") {
        if (isPairFirst && pendingMerges.has(d.id)) {
          // Half of a pending pair — the merge notification follows.
        } else if (!notifiedComplete.has(d.id)) {
          notifiedComplete.add(d.id);
          api
            .notify_on_complete(
              "Download Complete",
              `${d.filename} finished downloading`,
            )
            .catch(() => {});
        }
        // Paired capture: this may be one half finishing. The merge
        // runs when BOTH halves are completed (maybeAutoMerge is
        // idempotent and waits for the partner otherwise).
        void maybeAutoMerge(d.id);
      } else if (d.status === "error" || d.status === "canceled") {
        notifiedComplete.delete(d.id);
        cancelAutoMergeFor(d.id, d.status);
      }
    });
  } catch {
    // Tauri not available (preview mode)
  }
}

export function stopEventListener(): void {
  if (unlistenFn) unlistenFn();
  unlistenFn = null;
}

// ── Derived stores ──────────────────────────────────────────────────
export const counts = derived(downloads, ($downloads) => ({
  all: $downloads.length,
  active: $downloads.filter(
    (d) => !["completed", "error", "canceled"].includes(d.status),
  ).length,
  queued: $downloads.filter(
    (d) => d.status === "queued" || d.status === "scheduled",
  ).length,
  paused: $downloads.filter((d) => d.status === "paused").length,
  completed: $downloads.filter((d) => d.status === "completed").length,
  errored: $downloads.filter((d) => d.status === "error").length,
  canceled: $downloads.filter((d) => d.status === "canceled").length,
  scheduled: $downloads.filter((d) => d.status === "scheduled").length,
}));

export const totalDownloaded = derived(downloads, ($downloads) =>
  $downloads.reduce((s, d) => s + (d.downloaded || 0), 0),
);

export const hasActive = derived(downloads, ($downloads) =>
  $downloads.some(
    (d) => d.status === "downloading" || d.status === "connecting",
  ),
);

// ── Smart folders (file type groupings) ────────────────────────────
export const smartFolders = derived(downloads, ($downloads) => {
  const buckets: Record<
    string,
    { count: number; downloaded: number; total: number }
  > = {
    video: { count: 0, downloaded: 0, total: 0 },
    audio: { count: 0, downloaded: 0, total: 0 },
    archive: { count: 0, downloaded: 0, total: 0 },
    document: { count: 0, downloaded: 0, total: 0 },
    image: { count: 0, downloaded: 0, total: 0 },
    binary: { count: 0, downloaded: 0, total: 0 },
    other: { count: 0, downloaded: 0, total: 0 },
  };

  const extMap: Record<string, string> = {
    mp4: "video",
    mkv: "video",
    webm: "video",
    mov: "video",
    avi: "video",
    flv: "video",
    m3u8: "video",
    mp3: "audio",
    wav: "audio",
    flac: "audio",
    ogg: "audio",
    aac: "audio",
    m4a: "audio",
    zip: "archive",
    rar: "archive",
    "7z": "archive",
    gz: "archive",
    tar: "archive",
    tgz: "archive",
    bz2: "archive",
    pdf: "document",
    doc: "document",
    docx: "document",
    xls: "document",
    xlsx: "document",
    ppt: "document",
    pptx: "document",
    txt: "document",
    md: "document",
    rtf: "document",
    jpg: "image",
    jpeg: "image",
    png: "image",
    gif: "image",
    webp: "image",
    svg: "image",
    bmp: "image",
    exe: "binary",
    msi: "binary",
    appimage: "binary",
    deb: "binary",
    rpm: "binary",
    apk: "binary",
    iso: "binary",
    img: "binary",
    dmg: "binary",
  };

  for (const d of $downloads) {
    const ext = (d.filename || d.url).split(".").pop()?.toLowerCase() || "";
    const bucket = extMap[ext] || "other";
    buckets[bucket].count++;
    buckets[bucket].downloaded += d.downloaded || 0;
    buckets[bucket].total += d.totalSize || 0;
  }

  return buckets;
});
