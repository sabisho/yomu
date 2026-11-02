# Yomu — UI/UX Implementation Handoff & Architecture Guide

## 1. Scope & Implementation Boundaries

This handoff document provides engineering guidance for translating the verified FigCraft redesign into safe Rust 2024 using **GPUI Kit 0.8.1** (`gpui-pre = 0.3.10` runtime) under `#![forbid(unsafe_code)]`.

> [!IMPORTANT]
> **Design-Only Scope Notice:** No Rust source files, manifests, or dependencies have been modified during this assignment. The implementation phase must strictly follow the repository's root [`AGENTS.md`](file:///home/sabisho/Projects/yomu/AGENTS.md).

---

## 2. Deliverables & Verified FigCraft Frames

The complete redesign is authored in the native FigCraft document located at:
`design/figcraft/yomu_redesign.figcraft` (1266 nodes, 13 top-level frames, 10 embedded high-res assets).

All 13 frames are exported to standalone PNG previews inside `design/figcraft/`:

| Frame ID | Preview File | Canvas Size | Key Functional Highlights |
|---|---|---|---|
| `00` | [`00_Design_System_Tokens.png`](file:///home/sabisho/Projects/yomu/design/figcraft/00_Design_System_Tokens.png) | 1600×1100 | Canonical Gruvbox Dark & Light color swatches, typographic scale, and control sizing scales. |
| `01` | [`01_UI_Component_Library.png`](file:///home/sabisho/Projects/yomu/design/figcraft/01_UI_Component_Library.png) | 1600×1100 | Button variants, ButtonGroup, Input, Select, verified Lucide vector icons, and card/row components. |
| `02` | [`02_Library_Grid_View_1440x900.png`](file:///home/sabisho/Projects/yomu/design/figcraft/02_Library_Grid_View_1440x900.png) | 1440×900 | Main library grid (4 columns @ 260px), authentic manga covers, format tags, and progress bars. |
| `03` | [`03_Library_List_View_1440x900.png`](file:///home/sabisho/Projects/yomu/design/figcraft/03_Library_List_View_1440x900.png) | 1440×900 | High-density virtualized list view with mini cover thumbnails, progress bars, and context menus. |
| `04` | [`04_Library_Multi_Selection_1440x900.png`](file:///home/sabisho/Projects/yomu/design/figcraft/04_Library_Multi_Selection_1440x900.png) | 1440×900 | Multi-selection state with orange accent card borders and floating action sheet (640×52px). |
| `05` | [`05_Empty_And_Search_States_1440x900.png`](file:///home/sabisho/Projects/yomu/design/figcraft/05_Empty_And_Search_States_1440x900.png) | 1440×900 | Split showcase of empty library onboarding state and zero-result search state. |
| `06` | [`06_Continue_Reading_Hero_1440x900.png`](file:///home/sabisho/Projects/yomu/design/figcraft/06_Continue_Reading_Hero_1440x900.png) | 1440×900 | Hero continue reading banner (1128×280px) with primary CTA and recently active 3-column queue. |
| `07` | [`07_Collections_And_History_1440x900.png`](file:///home/sabisho/Projects/yomu/design/figcraft/07_Collections_And_History_1440x900.png) | 1440×900 | Split secondary view: custom Collections cards (left) and chronological Reading History (right). |
| `08` | [`08_Reader_Dual_Spread_1440x900.png`](file:///home/sabisho/Projects/yomu/design/figcraft/08_Reader_Dual_Spread_1440x900.png) | 1440×900 | Dual-spread manga reader: Page 25 (Right) and Page 24 (Left) in authentic RTL progression. |
| `09` | [`09_Reader_Single_Page_1440x900.png`](file:///home/sabisho/Projects/yomu/design/figcraft/09_Reader_Single_Page_1440x900.png) | 1440×900 | Edge-to-edge single page immersive reader mode with floating page indicator. |
| `10` | [`10_Settings_Preferences_1440x900.png`](file:///home/sabisho/Projects/yomu/design/figcraft/10_Settings_Preferences_1440x900.png) | 1440×900 | Two-pane settings matching Yomu's exact 6 categories (General, Appearance, Reader, Library, Shortcuts, Advanced). |
| `11` | [`11_Dialogs_Modals_Menus_1440x900.png`](file:///home/sabisho/Projects/yomu/design/figcraft/11_Dialogs_Modals_Menus_1440x900.png) | 1440×900 | Destructive removal AlertDialog, archive Properties modal dialog, and right-click ContextMenu. |
| `12` | [`12_Library_Responsive_Narrow_1024x768.png`](file:///home/sabisho/Projects/yomu/design/figcraft/12_Library_Responsive_Narrow_1024x768.png) | 1024×768 | Responsive narrow layout: 64px icon-rail sidebar and 3-column card grid reflow. |

---

## 3. Screen-by-Screen Implementation Directives

### A. Main Library (Grid & List Views)
- **Grid Card (`render_manga_card`):** 260px wide by 370px high. Cover container occupies the top 316px (1:1.215 ratio), with 52px reserved for title and author below. Covers are decoded asynchronously on `cx.background_executor()`. Solid Gruvbox `#3c3836` surface, 8px corner radius, 1px `#504945` border. On hover, surface elevates to `#504945` with cursor pointer.
- **Progress Pill:** In-progress archives show an accent bar (`#fe8019`) along the bottom of the cover art with page count pill on bottom-right.
- **Multi-Selection Floating Toolbar:** When 2 or more books are selected, render a floating bottom action sheet (`FloatingSelectionBar`, 640×52px, 10px radius, drop shadow `0 8px 16px #00000066`). Actions: `Add to Collection ▾`, `Mark as Read`, `Remove` (`#fb4934`), `Deselect (Esc)`.

### B. Secondary Views
- **Continue Reading (`src/app/views/continue_reading.rs`):** Hero banner card (1128×280px) displaying the most recently active book with a 40px primary CTA `"▶ Resume Reading"` and inline progress bar. Below, render a 3-column "Up Next" queue.
- **Collections (`src/app/views/collections.rs`):** Card grid with custom colored icon badges (`Favorites`, `Completed Series`, `Currently Reading`). Right-click displays collection context menu (Rename, Delete Collection).
- **History (`src/app/views/history.rs`):** Chronologically grouped rows (`TODAY`, `YESTERDAY`, date headers) displaying reading session duration and timestamp.

### C. Reader Viewport & Spread Logic (`src/ui/reader.rs`)
- **Immersive Canvas:** Rendered on a pure deep canvas (`#161616`). Zero visible UI chrome during active reading.
- **Floating Header Overlay:** 48px height, 8px radius, translucent `#1d2021ee` with 120ms ease-out slide animation. Contains back button, book title, `Single` / `Double` segmented control (`ButtonGroup`), `RTL (Manga) ▾` direction selector, `Fit Height ▾` mode selector, page badge (`Page 24-25 / 224`), zoom level indicator, and fullscreen toggle.
- **Spread Modes:** Exactly two layout modes exist in `src/layout.rs`: `PageLayout::Single` and `PageLayout::Double`. Single-image spreads exceeding aspect ratio 1.25 are auto-detected.
- **Reading Direction:** Under `ReadingDirection::RightToLeft`, Page 25 renders on the Right and Page 24 on the Left, preserving Japanese reading progression. Physical archive indices remain strictly zero-based and chronological.
- **Lateral Chevrons:** Floating translucent zones (36×80px) on left and right screen edges with subtle vector chevrons.

### D. Settings Interface (`src/ui/settings_view.rs`)
- Two-pane preferences view with the exact 6 categories from source:
  1. **General:** Continuous vertical scrolling toggle, About Yomu v0.1.0 info.
  2. **Appearance:** Theme Mode (Dark/Light/System), Color Scheme (`Gruvbox Dark`, `Gruvbox Light` only), Card corner rounding, Card border size.
  3. **Reader:** Default Page Layout (Single/Double), Reading Direction (Right-to-Left / Left-to-Right), Spread Detection Threshold (1.25), First page is cover toggle, Remember zoom level, Vim keybindings toggle.
  4. **Library:** Series grouping toggle, Thumbnail size preset.
  5. **Shortcuts:** Interactive keybinding cheat-sheet table (`h/j/k/l`, `g/G`, `/`, `⌘,`).
  6. **Advanced:** Cover cache size + Clear Cache, Reset Settings, Reset Library & Data.
- **Permanent Motion Invariant:** No motion-disable switch. Application initialization enforces `cx.set_reduce_motion(false)`.

### E. Dialogs & Menus (`src/app/actions.rs`)
- **AlertDialog:** Native destructive dialog for removing items from library. Follows the Safety Invariant: *"Comic files on your disk will never be deleted."*
- **Properties Dialog:** Shows metadata, archive container type, compressed file size, SIMD resized resolution, and filesystem path.
- **ContextMenu:** Contextual actions (`Open Reader`, `Mark as Read`, `Add to Collection...`, `Book Properties...`, `Remove from Library`) with keyboard shortcuts right-aligned in `Kbd` style.

---

## 4. Animation & Physics Policy

- **Curve:** Standard cubic bezier ease-out (`cubic-bezier(0.16, 1, 0.3, 1)`).
- **Duration:** 120ms to 140ms for toolbars, dropdowns, and modal presentations via official `EffectTransition`.
- **Static High-Frequency Surfaces:** Comic page textures, book card cover grids, and search result rows remain static with zero animation during scroll to preserve 60/144/240 Hz frame pacing.
- **Memory & Allocation Discipline:** Decoded page textures respect `pagecache::system_memory_budget()` (512 MiB to 3.0 GiB based on 1/8th of host RAM). Texture eviction on `window.drop_image`.
