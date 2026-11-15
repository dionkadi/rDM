# DM — UI Design Plan (v2.1)

**Brief:** redesign the app and the browser-extension popup around *简约但不简单、不简陋* —
simple but not simplistic. Information density and scannability over decoration.
This document is the UX contract: read it before touching any UI code.

**v2.1 (review pass)** reconciles the contract with the shipped interface. The v2
system was sound; the implementation drifted in five places: light-theme tints,
filter counts, status-bar typography, the sticky selection bar, and the
add-flow's global speed cap. Those drifts, their severity and the fixes are
catalogued in [`UI_REVIEW.md`](UI_REVIEW.md). This revision also removes the
contradictions v2 carried (row height, ping placement, modal motion), and
closes the two items the review had left open: the sparkline's theme response
and the popup's title-row status (§7).

Supersedes the deleted v1 plan (git `9c27090^`). The v1 plan targeted "IDM-parity
polish" and prescribed springs, glows, shimmer, grain and gradient washes — the
result is the interface we are replacing. See "Counter-cases" below.

---

## 1. Research inputs

**Borrowed**

| Source | What we take |
|---|---|
| Motrix / FDM modern shells | Calm single-purpose chrome; the list is the product, not a dashboard around it |
| IDM (efficiency only) | Row-level information density: size, rate, ETA, connections visible without opening anything |
| Linear / Things (discipline, not palette) | Hairline structure, one accent, sentence case, shadow only on floating layers |
| Instrument panels (DAWs, network monitors) | Tabular numeric readouts; meters as the primary visual element; flat surfaces |
| agents-level extension popups | ≤ 2 toggles visible, one primary action, status as a dot + one line |

**Counter-cases (do not repeat)**

- **DM v0.7 itself**: ambient radial gradients + film grain + glowing gradient buttons
  + tracked ALL-CAPS eyebrows + 3 font families + shimmer + pulse rings. Decoration
  with zero information content; the eye has nowhere to rest; low information density
  per pixel (72 px card rows, a 200 px "aggregate throughput" hero above the list).
- **IDM / JDownloader dialogs**: control sprawl — every option always visible.
  → progressive disclosure; defaults visible, the rest behind one disclosure.
- **Generated-page tells** (cream+terracotta, acid-green on near-black, identical
  rounded card kits, mono used as decoration, `A · B · C` meta strings): none of
  these encode information. The accent hue is kept for brand continuity but is
  **calibrated** (jade, not glow-stick mint) and used only for action and progress.
- **Google Fonts at runtime**: the app must render identically offline and on
  networks where `fonts.googleapis.com` is unreachable (fallback to Georgia was a
  real reason the old UI read as "cheap"). No runtime font fetches.

---

## 2. Design principles

1. **Information is the only ornament.** Every visible element answers "what is this
   file doing right now?". If it doesn't, cut it.
2. **One accent, shared jobs.** The accent color marks *action* (buttons, links, focus)
   and *progress* (fills). It may also mark the single live row (label + dot) so the
   eye lands on what is moving; every other status uses the quiet semantic set
   (success/warning/danger/info). Accent must never appear in four places in one row.
3. **Numbers do the talking.** Byte counts, rates, percentages and ETAs render in
   tabular figures (mono readout font). Labels are UI text in sentence case, never
   tracked caps and never mono. The readout face is a property of the *value*, not
   of the container: a status bar full of mono words reads as a terminal, not data.
4. **Flat surfaces, hairline structure.** No shadows on rows or panels; shadow exists
   only for things that float above the page (menus, modals, popovers). 1 px hairlines
   do the grouping work that cards and radii used to do.
5. **Density over spaciousness.** A download row is one 40 px line of record
   (34 px in compact mode); the meter and the numbers share the row. Aggregate stats
   live in **one** place (status bar), plus one contextual selection bar that floats
   above it only while rows are selected.
6. **Quiet motion.** Transitions confirm state changes (≤ 150 ms, opacity/color).
   Nothing animates on a timer; rows do not fly or fade in on load; `prefers-reduced-motion`
   is respected. The single allowed loop is the tiny live-dot on an active row.

---

## 3. Color system

Named bases (hex shown for the dark theme default; light in parentheses):

| Role | Dark | Light | Use |
|---|---|---|---|
| `bg` | `#17191E` | `#F4F5F7` | window background |
| `panel` | `#1E2128` | `#FFFFFF` | floating surfaces, inputs |
| `panel-2` | `#242830` | `#ECEEF2` | hover fill |
| `track` | `#2A2F38` | `#E2E5EA` | selected fill, meter track |
| `hairline` | `rgba(255,255,255,.08)` | `rgba(17,20,26,.10)` | all borders/dividers |
| `text` | `#E6E8EB` | `#1B1E24` | primary text |
| `muted` | `#9BA1AB` | `#5F6673` | secondary text |
| `faint` | `#8C95A4` | `#676E7C` | tertiary / placeholders (≥4.5:1 AA) |
| `accent` (jade) | `#41C9A2` | `#0D7A5E` | action + progress fill only |
| `success` | `#54C68F` | `#1D7A4E` | completed |
| `warning` | `#DFA84E` | `#966100` | queued / limits |
| `danger` | `#E5716A` | `#C0392F` | error / destructive |
| `info` | `#7BA3E8` | `#3159B8` | connecting / scheduled / hints |

Rules: no gradient fills anywhere; no glow (`box-shadow` only for focus rings and
floating layers); category hues are low-chroma and appear only in the file-type
tile. Both themes define the same variable set (`app.css`).

**Category hues are not semantic hues.** The first pass reused warning amber for
video, success green for archives and danger red for PDF, so a file type read as
a status. The tile palette is now deliberately offset from the semantic set:

| Category | Dark | Light |
|---|---|---|
| video | `#B58AD8` (plum) | `#6B4C8F` |
| audio | `#5FB6AE` (teal) | `#1D6B66` |
| archive | `#C3A363` (ochre) | `#7A5A16` |
| document | `#8CA6C9` (slate blue) | `#3B5A86` |
| image | `#C98FA8` (mauve) | `#8A3F5C` |
| binary | `#98A2B3` (slate) | `#4E5665` |

Static assets resolve by *meaning*, not extension colour: `iso`/`img`/`dmg` are
software (`binary`), `pdf`/office files are documents, `m3u8` is video. A PDF is
never painted like an error.

## 4. Typography

- **UI:** `system-ui, -apple-system, "Segoe UI", "Noto Sans SC", "PingFang SC",
  "Microsoft YaHei", sans-serif` — zero network dependency, CJK-safe.
  (Optional upgrade: bundle `@fontsource/ibm-plex-sans` latin subsets; CJK keeps
  the system fallback. Not required for this iteration.)
- **Readout:** `ui-monospace, "JetBrains Mono", "SF Mono", "Noto Sans Mono", Menlo,
  Consolas, monospace` with `font-variant-numeric: tabular-nums` — **numbers only**:
  sizes, rates, percentages, ETAs, counts. Never headings, never body copy.
- Scale: 11 / 12 / 13 / 14 / 16 / 20 px. Nothing larger than 20 px outside the
  command palette and empty states. No display/serif face.
- Sentence case everywhere. No `letter-spacing` above `0.02em`. No uppercase labels.
- The readout face is applied to the value span, never to a whole bar. Labels next
  to a number use the UI face at 11 px (e.g. `12 active`, `3.6 GB downloaded`).
- One engraving exception: the three-letter code inside the 28 px type tile may sit
  at 9.5 px. It is a texture marker, not body text.

## 5. Layout & information architecture

The sidebar was removed entirely — the list owns the window. Filters
live in one horizontal strip; the add flow is a bare one-line input;
rows are a single line of record in fixed numeric columns (a ledger).

```
┌──────────────────────────────────────────────────────────────────┐
│ ⬇ DM  [Search… ⌘K]                              [⚙]             │  topbar 48px
│ All 6 · Active 2 · Queued 3 · Paused · Done 8 · Errors │ Video … │  filter strip 38px
│ [ Paste a URL and press Enter…        ] [Default▾] [More▾] [Add] │  add line (bare)
├──────────────────────────────────────────────────────────────────┤
│    NAME                       PROGRESS    SIZE   SPEED ETA STATUS│  column header 28px
│ ☐ ▣ ubuntu-24.04.iso         ▓▓▓▓▓░░░ 62% 2.4/3.9 GB 2.8MB/s ×4 │  rows 40px
│ ☐ ▣ product-demo_cut.mp4     ▓▓▓▓▓▓░░ 65% 874/1.3 GB 2.0MB/s ×3 │  (hairlines only,
│ …                                                            ⋯ ⌄ │   no cards)
│        [2 selected  Pause Resume Limit Merge Trash Remove]        │  selection bar
├──────────────────────────────────────────────────────────────────┤
│ ● 2 active ↓ 4.8 MB/s ▁▃▅▂  3.6 GB downloaded · limit ∞ · 8 done │  status bar 32px
└──────────────────────────────────────────────────────────────────┘
```

Decisions (v2.1, supersedes the sidebar layout of the first pass):

- **No sidebar.** Status + smart-folder filters become one horizontal
  chip strip (scrolls when full); categories join it only when they
  have counts. Counts live **only** here — the column header no longer
  repeats "N shown / N total". Search is a topbar trigger for the command palette.
- **Full-bleed shell, solid chrome.** The ledger owns the window width.
  Topbar, filter strip and status bar are opaque with hairline borders; no
  backdrop blur on scrolling chrome. The column header sticks under the
  48 + 38 px pair.
- **Ledger rows**: one 40px line with shared grid `--row-grid`
  (checkbox · 28px type tile · name · meter+% · size · speed · eta · status ·
  actions). Click selects (bulk), Enter/chevron expands the detail
  panel. Priority + per-file speed limit live in the expanded panel;
  hover actions are pause/resume · menu · chevron only.
- **Column headers with right-aligned tabular numerics** — the eye
  scans down a column of rates/sizes like an instrument readout.
  Least-critical columns (ETA, then Speed) drop out at 1180 / 980px
  via the `--row-grid-no-*` variants.
- **Add is a bare line**, not a panel: URL input + category + `More▾`
  (checksum, *per-download* limit) + Add. Enter submits; Add is disabled
  until the input is a complete http(s) URL.
- **The global speed cap is not an add-field.** It lives in
  Settings → Network → Bandwidth and is reported read-only in the status
  bar, so configuring one URL can never throttle the whole app.
- **Selection is contextual, not a toolbar.** Selecting rows raises a
  compact bar just above the status bar. It overlays the tail of the list
  (the shell reserves scroll room) instead of pushing the ledger down or
  fighting the sticky column header.
- **Aggregate metrics live in the status bar only** (rate, sparkline,
  downloaded total, limit, done/errors, demo flag). Engine ping moved to
  Settings → About. No sidebar stats block, no throughput hero, no activity strip.
- Size column shares the unit (`2.4 / 3.9 GB`) to avoid truncation;
  speed appends `×N` connections when > 1.

## 6. The signature element: the segment-aware meter

DM is a *segmented* downloader; the meter shows that. The fill is a
flat accent bar with **hairline ticks carved out at chunk boundaries**
(one continuous meter, not dashed blocks), so a 16-connection download
is visibly different from a single stream. Live rows render the meter
at 5 px, idle rows at 3 px — hierarchy through the instrument itself.
HLS rows show the bucketed cell strip (existing behavior, retuned).
Rules: `radius 1.5px`, flat fill (no gradient, no shimmer, no glow),
track is `track`. This is the one place the design is allowed to be
expressive — it encodes real engine state. One continuous fill carries the
overall percentage; the tick marks carry the connection layout.

The icon column is a flat, square **type tile** — 28 × 28 px, neutral
surface, hairline border, a three-letter mono code at 9.5 px in the
category hue. It answers only "what kind of file". Progress is the
meter's job; status is the status column's job. The old ring/glow icon
was redundant with the meter and read as decoration. The tile must fit
its grid column exactly (a 26 px column holding a 28 px tile caused a
2 px bleed into the filename).

## 7. Surface specs

- **Topbar** (48 px): brand 14 px semibold + 20 px mark; search trigger; settings.
  Solid background, no blur. No taglines. No Ping (engine health lives in
  Settings → About).
- **Status bar** (32 px): `active · ↓rate · ↓downloaded · limit · done/errors`.
  Labels use the UI face at 11 px; only the values use the readout face. Speed is
  neutral text, not a second green. With zero active rows the rate reads `—` and
  the sparkline is omitted. The sparkline resolves its line/grid colours from
  theme tokens at draw time, so a theme switch (including an OS preference flip
  in `system` mode) repaints it immediately rather than leaving the old accent.
  Right side: demo flag when applicable. Truncates gracefully under 720 px.
- **Selection bar**: floats 10 px above the status bar, max `calc(100vw - 24px)`,
  opaque `panel`, radius 12, `floating` shadow. Ghost 28 px actions; Resume is the
  only accent. It is not sticky inside the list and never shares the column
  header's offset.
- **Row actions**: appear on hover/focus as ghost icon buttons (24 px) — no pills.
  Priority + per-file speed limit live in the expanded panel (they are
  occasional, not per-row); hover actions are pause/resume · menu · chevron.
  Enter on a row toggles detail; Enter on a child control must not bubble.
- **Status recedes when finished**: only live rows use the accent on the
  label; completed/error rows carry meaning in the dot + meter, the label
  stays quiet. Completed rows show a single size (`219.3 MB`), not the
  running/total split. The percentage steps up to primary text on live rows
  instead of a third accent.
- **Column header** (28 px, 11 px labels) sticks under the topbar + filter strip
  on scroll (solid `bg`, z-index below the topbar) so the ledger columns stay
  anchored on long lists. It carries column labels only, not counts.
- **Expanded panel**: aligned with the filename column; a definition grid capped
  at 860 px (keys `muted` 11 px, values readout); chunk/segment maps keep current
  behavior; errors in a `danger-dim` strip. Dashed rules are not part of the line
  language.
- **Modals / palette**: floating layers only; `backdrop` blur ≤ 8 px; radius 12 px;
  `modal` shadow on centred dialogs, `floating` on menus/toasts. Palette, dialogs
  and toasts fade (120 ms); no slide-in. Command palette is the same instrument
  language (text mode switch, readout shortcuts on the right, no decoration).
- **Settings drawer**: 720 px max, 56 px header, 200 px nav on `bg`; title 16 px;
  sentence-case field labels; flat switch (accent track, no gradient); status dots
  without glow; radius 8 for option cards.
- **Extension popup** (300 px): title row = brand mark + `DM Grabber` on the
  left and the host state as a dot + word (`Connected` / `Disconnected` /
  `Checking…`) on the right; one ellipsised status line beneath it, with the full
  text in `title`. Primary button `Grab page media` (full-width, flat accent,
  inverse text); host re-check / test as text buttons; two switch rows (no
  fieldset legend); error detail expands inline directly under the status line —
  there is no status block at the bottom. Same tokens as the app;
  `prefers-color-scheme` drives theme.

## 8. Motion

| Interaction | Treatment |
|---|---|
| hover / focus | color/background 120 ms ease-out; no transforms on rows (DropdownMenu re-anchor) |
| row insert / reorder | no entry animation on load; reorder 180 ms (`flip`) |
| row expand | height 180 ms ease-out, children fade only |
| meter fill | width 250 ms linear-ish (data-driven) |
| toast / palette / modal | opacity 120 ms; no slide, no spring |
| settings drawer | slide from the right 200 ms (it is a drawer; a centred modal never slides) |
| live dot (active row only) | 2 s opacity pulse, disabled under reduced motion |

## 9. Accessibility floor

- Focus-visible ring = 2 px `accent` outline offset 2 (visible on both themes).
- Contrast ≥ 4.5:1 for text, ≥ 3:1 for UI borders on both themes.
- All icon buttons keep `aria-label`; rows keep `role="button"` + key handlers.
  Child controls stop Enter from bubbling to the row.
- The status bar is **not** an `aria-live` region: it updates every second and
  would announce the rate forever. Toasts are the live region.
- `prefers-reduced-motion` kills every animation (already global; keep).
- No meaning by color alone: status = dot + label, reinforced by the meter.

## 10. Implementation map

| File | Change |
|---|---|
| `src/app.css` | token rewrite (semantic palette, category hues, track/chevron, shell sizes), solid sticky chrome, ledger grid, 11 px labels |
| `src/lib/design-tokens.ts` | sync to `app.css` (single source of truth = `app.css`; this file is the JS mirror) |
| `src/App.svelte` | bare add line + URL validation, per-download limit, filter semantics, floating selection room, no row entry animation |
| `src/lib/stores/downloads.ts` | `counts.active` = bytes moving (downloading + connecting) |
| `src/lib/stores/settings.ts` | hydrate/sync the global limit label |
| `src/lib/components/SmartFilters.svelte` | counts from the store; text-only chips |
| `src/lib/components/DownloadRow.svelte` | ledger row; 28 px tile; status-aware detail panel; no redundant error bubble |
| `src/lib/components/FileIcon.svelte` | square 28 px tile; corrected type map (PDF/document, ISO/binary, HLS/video) |
| `src/lib/components/ProgressBar.svelte` | flat meter + chunk ticks; segmented path respects status |
| `src/lib/components/StatusBar.svelte` | single home for aggregate metrics; UI labels + readout values; no ping |
| `src/lib/SpeedGraph.svelte` | theme-reactive sparkline: tokens resolved at draw time, repaint on effective-theme change |
| `src/lib/stores/ui.ts` | `effectiveThemeStore` tracks the rendered theme, including OS preference flips |
| `src/lib/components/BulkActionBar.svelte` | floating selection bar above the status bar |
| `src/lib/components/SettingsTabs.svelte` | sentence case, flat switch, Bandwidth + Engine health; no dead data buttons |
| `src/lib/components/Toast.svelte` | clears the status bar; fade only |
| `src/lib/components/CommandPalette.svelte` | light-theme tokens, text mode switch, working download selection |
| `browser-extension/popup.html` / `popup.js` | instrument-language popup per §7; real brand mark; title-row host dot + one-line status |
| `scripts/check-css-vars.mjs` | must stay green (all `var()` resolve in both themes) |

Verification: `npm run check` (0/0), `node scripts/check-css-vars.mjs`,
`npm run build`, and visual passes in both themes via `npm run dev` (demo mode).
The review that produced v2.1, including the issue list and priorities, lives in
[`UI_REVIEW.md`](UI_REVIEW.md).
