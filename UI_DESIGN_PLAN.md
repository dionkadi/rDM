# DM — Detailed UI Design & Refinement Plan

**Goal:** Achieve IDM-parity polish and usability with a modern, graceful frontend.

---

## 1. Visual Design System Overhaul

| Aspect | Current | Target (IDM-level) |
|--------|---------|-------------------|
| **Color System** | Single accent (`#7af0c8`) | Semantic token system: primary, success, warning, danger, info + state variants |
| **Typography** | Fraunces/Sora/JetBrains Mono | Refined scale: display, headline, title, body, caption, mono; better line heights |
| **Spacing** | Ad-hoc px values | 4px base grid, consistent rhythm (4, 8, 12, 16, 24, 32, 48, 64) |
| **Elevation** | Single shadow | 4 elevation levels (flat, raised, floating, modal) with colored shadows |
| **Motion** | Basic transitions | Spring physics (stiffness: 280, damping: 24), staggered entrance, FLIP animations |
| **Border Radius** | 18px/11px | Harmonized: xs(4), sm(8), md(12), lg(16), xl(24), full(9999) |

---

## 2. Layout Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│  Top Bar (48px)  │  App Title | Global Search ⌘K | Speed Cap | ⚙️  │
├──────────┬──────────────────────────────────────────────────────────┤
│          │  Toolbar (44px)  │  Filter Chips | View Toggle | Sort   │
│ Sidebar  ├──────────────────────────────────────────────────────────┤
│ (280px)  │                                                          │
│          │         Download List (Virtualized)                      │
│  • Brand │  ┌────────────────────────────────────────────────────┐  │
│  • Nav   │  │  Row: [Icon] [Info] [Progress] [Speed/ETA] [Actions]│  │
│  • Smart │  │  Expandable: Chunks | Headers | Logs | Preview     │  │
│  • Stats │  └────────────────────────────────────────────────────┘  │
│          │                                                          │
├──────────┴──────────────────────────────────────────────────────────┤
│  Status Bar (32px)  │  Active: 3  │  ⬇ 42 MB/s  │  ↑ 1.2 MB/s  │  │
└─────────────────────────────────────────────────────────────────────┘
```

**Key Changes:**
- **Collapsible sidebar** (280px → 64px icon-only) with smooth animation
- **Global search** (⌘K) with fuzzy matching across filename, URL, category
- **Virtualized list** for 1000+ downloads (using `@tanstack/svelte-virtual`)
- **Persistent status bar** with aggregate metrics
- **Expandable rows** → click chevron to reveal chunk bars, request/response headers, preview

---

## 3. Download Row — "The Hero Component"

```svelte
<!-- Compact (default) -->
<div class="drow" data-status={status}>
  <FileIcon category={category} progress={pct} />
  <div class="info">
    <div class="primary">
      <Filename truncated>{filename}</Filename>
      <StatusBadge>{status}</StatusBadge>
    </div>
    <div class="secondary">
      <ProgressBar segments={chunks} />  <!-- Segmented by chunk! -->
      <MetaLine>{downloaded}/{total} • {speed} • ETA {eta}</MetaLine>
    </div>
  </div>
  <div class="actions">
    <InlineSpeedLimit />  <!-- Per-file limit input -->
    <PrioritySelect />    <!-- High/Normal/Low -->
    <DropdownMenu>        <!-- Pause/Resume/Cancel/Remove/Open Folder/Copy Link -->
  </div>
  <ChevronExpand />
</div>

<!-- Expanded (click chevron) -->
<div class="drow-expanded">
  <ChunkVisualizer chunks={chunks} />  <!-- Animated per-chunk bars -->
  <RequestHeaders />                    <!-- Collapsible -->
  <ResponseHeaders />
  <FilePreview />                       <!-- Image/video/PDF thumbnail -->
  <ErrorDetails />                      <!-- If error -->
</div>
```

**IDM-parity features:**
- ✅ Segmented progress bar (each chunk = visual segment)
- ✅ Per-file speed limit (inline, not modal)
- ✅ Priority queue (high/normal/low)
- ✅ Right-click context menu with all actions
- ✅ Drag-to-reorder queue priority
- ✅ File type preview (images, video, PDF, archives)

---

## 4. Sidebar — Smart Navigation

```
┌──────────────────────────┐
│  DM  Download Manager  ● │  ← Live indicator
├──────────────────────────┤
│  🔍  Search downloads... │  ← Global search (⌘K)
├──────────────────────────┤
│  FILTERS                 │
│  ├─ All (127)            │
│  ├─ 🟢 Active (3)        │
│  ├─ ⏳ Queued (5)        │
│  ├─ ⏸ Paused (2)        │
│  ├─ ✅ Completed (89)    │
│  ├─ ❌ Errors (4)        │
│  └─ 📅 Scheduled (1)     │
├──────────────────────────┤
│  SMART FOLDERS           │
│  ├─ 📁 Video (23)        │
│  ├─ 📦 Archives (18)     │
│  ├─ 💿 Disk Images (4)   │
│  ├─ 📄 Documents (31)    │
│  └─ 🎵 Audio (12)        │
├──────────────────────────┤
│  CATEGORIES              │
│  ├─ Default (~/Downloads)│
│  ├─ Videos (~/Videos)    │
│  └─ Software (~/Software)│
├──────────────────────────┤
│  THROUGHPUT              │
│  ⬇ 42.3 MB/s  ⬆ 1.2 MB/s │
│  ████████░░░░░░░░░░  3 active │
└──────────────────────────┘
```

---

## 5. Add Download — Progressive Disclosure

```
┌────────────────────────────────────────────────────────────┐
│  +  Add Download                                    [⚙]    │
├────────────────────────────────────────────────────────────┤
│  ┌──────────────────────────────────────────────────────┐  │
│  │ https://example.com/file.iso                         │  │
│  └──────────────────────────────────────────────────────┘  │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────┐  │
│  │ Category │ │ Filename │ │ Speed    │ │ Checksum     │  │
│  │ ▼ Video  │ │ file.iso │ │ ▼ 20MB/s │ │ SHA256: ...  │  │
│  └──────────┘ └──────────┘ └──────────┘ └──────────────┘  │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ Advanced ▼  [Connections: 16] [Priority: High] [Tag] │  │
│  └──────────────────────────────────────────────────────┘  │
│  [Cancel]                                    [Start Download]│
└────────────────────────────────────────────────────────────┘
```

---

## 6. Settings — Tabbed & Organized

```
┌────────────────────────────────────────┐
│  Settings                    [×]       │
├────────────────────────────────────────┤
│  General | Network | Scheduler | UI | │
│  Advanced| Extensions| Backup  | About│
├────────────────────────────────────────┤
│  [Tab content with sections, validation]│
│                                         │
│  [Reset to defaults]   [Import] [Export]│
└────────────────────────────────────────┘
```

---

## 7. New Components to Build

| Component | Purpose |
|-----------|---------|
| `DownloadRow.svelte` | Expandable, virtualized, chunk-visualized row |
| `ChunkVisualizer.svelte` | Animated per-chunk progress bars |
| `FileIcon.svelte` | Dynamic icon with progress ring |
| `ProgressBar.svelte` | Segmented (chunks) + shimmer |
| `StatusBadge.svelte` | Animated status pill |
| `InlineSpeedLimit.svelte` | Per-file limit slider/input |
| `PrioritySelect.svelte` | High/Normal/Low badge |
| `DropdownMenu.svelte` | Context menu + keyboard nav |
| `GlobalSearch.svelte` | ⌘K palette with fuzzy search |
| `SmartFilters.svelte` | Auto-generated by file type |
| `VirtualizedList.svelte` | TanStack Virtual wrapper |
| `Toast.svelte` | Non-blocking notifications |
| `CommandPalette.svelte` | ⌘K global actions |
| `OnboardingTour.svelte` | First-run guide |
| `FilePreview.svelte` | Thumbnail/preview for media |
| `SettingsTabs.svelte` | Tabbed settings with validation |

---

## 8. Interaction & Motion Spec

| Interaction | Animation |
|-------------|-----------|
| Row hover | `transform: translateX(4px)`, border brighten (150ms spring) |
| Row expand | Height auto + fade (280ms spring, stagger children 40ms) |
| Chunk bar progress | Width + shimmer (350ms cubic-bezier(0.4, 0, 0.2, 1)) |
| Sidebar collapse | Width + opacity (220ms spring) |
| Toast enter | Slide up + fade (200ms spring) |
| Button press | Scale 0.96 (80ms) |
| Drag preview | Opacity 0.6 + rotate 2deg + scale 1.02 |

---

## 9. Accessibility Checklist

- [ ] Semantic HTML (`<nav>`, `<main>`, `<section>`, `<article>`)
- [ ] ARIA labels on all icon buttons
- [ ] Focus visible outlines (3px accent ring)
- [ ] Keyboard navigation: Tab, arrows, Enter, Escape, ⌘K
- [ ] Screen reader announcements for status changes
- [ ] Color contrast ≥ 4.5:1 (WCAG AA)
- [ ] Reduced motion (`prefers-reduced-motion`)
- [ ] High contrast mode support

---

## 10. Responsive Breakpoints

| Breakpoint | Layout |
|------------|--------|
| ≥ 1400px | Full sidebar + list + side details panel |
| 1024–1399px | Full sidebar + list |
| 768–1023px | Collapsible sidebar (icon-only default) |
| < 768px | Bottom sheet sidebar, stacked toolbar |

---

## 11. Implementation Priority (Phased)

| Phase | Scope | Est. Effort |
|-------|-------|-------------|
| **1** | Design tokens, typography, color system, spacing scale | 2h |
| **2** | `DownloadRow` rewrite with chunk visualization, expand/collapse | 4h |
| **3** | Virtualized list, global search (⌘K), smart filters | 3h |
| **4** | Sidebar collapse, status bar, toolbar | 2h |
| **5** | Context menus, keyboard shortcuts, drag-reorder | 3h |
| **6** | Settings tabs, import/export, validation | 2h |
| **7** | File preview, toast system, command palette | 3h |
| **8** | Onboarding tour, empty states, polish | 2h |
| **9** | Accessibility audit, reduced motion, testing | 2h |

**Total: ~23 hours of focused implementation**

---

## 12. Files to Modify/Create

```
src/
├── lib/
│   ├── design-tokens.ts          # NEW: Centralized design system
│   ├── components/
│   │   ├── DownloadRow.svelte    # NEW: Hero component
│   │   ├── ChunkVisualizer.svelte # NEW
│   │   ├── FileIcon.svelte       # NEW
│   │   ├── ProgressBar.svelte    # NEW (segmented)
│   │   ├── StatusBadge.svelte    # NEW
│   │   ├── InlineSpeedLimit.svelte # NEW
│   │   ├── PrioritySelect.svelte # NEW
│   │   ├── DropdownMenu.svelte   # NEW
│   │   ├── GlobalSearch.svelte   # NEW (⌘K)
│   │   ├── SmartFilters.svelte   # NEW
│   │   ├── VirtualizedList.svelte # NEW
│   │   ├── Toast.svelte          # NEW
│   │   ├── CommandPalette.svelte # NEW
│   │   ├── FilePreview.svelte    # NEW
│   │   ├── SettingsTabs.svelte   # NEW
│   │   ├── OnboardingTour.svelte # NEW
│   │   └── index.ts              # NEW: barrel export
│   ├── hooks/
│   │   ├── useKeyboardShortcuts.ts # NEW
│   │   ├── useVirtualizer.ts     # NEW
│   │   └── useToast.ts           # NEW
│   ├── utils/
│   │   ├── fuzzySearch.ts        # NEW
│   │   ├── filePreview.ts        # NEW
│   │   └── formatters.ts         # ENHANCED
│   └── stores/
│       ├── downloads.ts          # NEW: Svelte store
│       ├── ui.ts                 # NEW: sidebar, toasts, search
│       └── settings.ts           # NEW
├── app.css                       # REWRITE: Design tokens + components
├── App.svelte                    # REWRITE: Shell + composition
├── main.ts                       # ENHANCED: Stores, shortcuts
└── vite-env.d.ts                 # UNCHANGED
```