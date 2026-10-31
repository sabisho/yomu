# Architecture

## Runtime Topology

Yomu is structured into three clean architectural tiers:

1. **Core Domain Layer (`src/lib.rs` / `yomu` crate):** GUI-independent Rust (`#![forbid(unsafe_code)]`), decoupled from GPUI, windowing, and presentation frameworks. Combines pure algorithms (geometry calculations, cache eviction planning, Vim grammar parsing, search filtering, selection state machines) with filesystem and persistence operations (archive enumeration, SIMD image decoding and scaling, atomic JSON persistence). Fully testable without a window or graphics compositor.
2. **UI Presentation Layer (`src/ui/`):** Built on GPUI Kit 0.8.1 (`gpui-pre = 0.3.10` runtime). Encapsulates views, components, dialogs, theme loading, and platform-adaptive momentum scrolling adapters (`ui::scroll`).
3. **Application Orchestration (`src/main.rs` / planned `src/app/`):** Manages the application window lifecycle, top-level state routing, input dispatch, background decode task scheduling, GPU texture garbage collection, and debounced persistence.

```text
GPUI Kit Window / Yomu Application (src/main.rs / src/app/)
  ├─ View Router (MainView)
  │    ├─ Sidebar (src/ui/sidebar.rs) ── persistent 240px navigation
  │    ├─ Toolbar (src/ui/toolbar.rs) ── search, view mode, selection bar
  │    ├─ Library View (src/ui/library_view.rs) ── cards, folders, list rows
  │    ├─ Continue Reading (src/ui/continue_reading.rs) ── hero banner & queue
  │    ├─ Manga Details (src/ui/details.rs) ── metadata matrix & synopsis
  │    ├─ Collections (src/ui/collections_view.rs) ── smart & custom folders
  │    ├─ History (src/ui/history_view.rs) ── in-progress & completed manga
  │    ├─ Settings (src/ui/settings_view.rs) ── 6-tab split-pane preferences
  │    ├─ Overlays (src/ui/overlays.rs) ── dialogs, menus, command prompt, help
  │    └─ Reader (src/ui/reader.rs) ── edge-to-edge canvas with autohiding HUD
  ├─ Core Library (src/library.rs) ── JSON persistence & XDG data management
  ├─ Reader Controller ── page decode tasks, prefetch queue, and GPU texture lifecycle
  ├─ Momentum Controller (src/scroll.rs -> planned src/ui/scroll.rs) ── platform momentum adapter
  └─ Background Executors ── multithreaded decode, resize, and import pipeline
```

---

## Modules and Boundaries

### Core Domain Modules (`src/`)

| Module | Responsibility | Invariants & Boundaries |
|---|---|---|
| `archive.rs` | Enumerate and read CBZ/CBR/ZIP/RAR pages and image folders | Natural-sorts entries; filters hidden/system files; enforces bounded allocation caps against archive bombs. Never mutates source archives on disk. |
| `imaging.rs` | Decode supported image formats (JPEG, PNG, WebP, GIF), SIMD scaling, JPEG cover generation | Strictly decoupled from GPUI. Pixel buffers are BGRA for GPUI textures. Large images decode and scale off the UI thread via `zune-jpeg` and `fast_image_resize`. |
| `library.rs` | Book models, reading progress, marks, user settings, JSON persistence, and import pipeline | Format version 3. Backwards-compatible and lenient with missing fields. Deleting books or collections never deletes original comic files. |
| `collections.rs` | Smart categories (Favorites, Reading, Want to Read, Completed) and custom user collections | Grouping metadata only; decoupled from book archive storage. |
| `layout.rs` | Fit modes (Page, Width, Height, Original), zoom calculation, viewport geometry, decode tiers | Pure math. Page indices are strictly zero-based and ordered in reading sequence regardless of RTL display travel. |
| `motion.rs` | Frame-time scroll easing, page-turn transitions, reduced-motion detection | Driven by delta time ($dt$), not frame counts. Pure analytical decay; zero UI dependencies. |
| `pagecache.rs` | Prefetch order and texture eviction planning | Pure policy. Preserves current page texture even if it exceeds budget. Adaptive decoded image budget: defaults to 1.0 GiB (`BUDGET_BYTES`) and scales between 512 MiB and 3.0 GiB based on 1/8th of detected host RAM. Static metrics cached to prevent UI-thread I/O. |
| `vim.rs` | Parse key tokens, counts, motions, and `:` command-line strings into typed `Action`s | Pure parser without UI dependencies. The UI applies the resulting `Action` enum. |
| `selection.rs` | Multi-selection state, toggle, and range selection anchors | Pure state machine. Emits selected IDs in stable library order for deterministic batch operations. |
| `search.rs` | Case-insensitive title, source path, and series substring matching | Pure helper. Query state is transient in the UI and never alters persisted library order. |
| `menu.rs` | Context menu item generation, validation, and confirmation requirements | Decides available book and library actions; UI renders and dispatches them. |
| `util.rs` | Natural sorting, FNV-1a path hashing, date/size formatting | Reusable pure helpers. |

### UI & Presentation Modules (`src/ui/` & `src/scroll.rs`)

| Module | Responsibility | Design & Integration |
|---|---|---|
| `scroll.rs` | Cross-platform momentum scrolling coordinator (`PlatformScrollController`) | Connects pure `motion::KineticScroll` to GPUI `ScrollHandle`. Preserves native AppKit momentum on macOS; provides kinetic glide on Linux and Windows. Planned relocation to `src/ui/scroll.rs`. |
| `sidebar.rs` | Persistent 240px navigation sidebar | Native macOS traffic lights on macOS; clean sections on Linux/Windows. Navigates between primary sections with badge counts. |
| `toolbar.rs` | Unified 52px header toolbar | Symmetrical back button, centered search with `⌘K`/`Ctrl+K` shortcut, view mode toggle, import action, and in-place selection toolbar. |
| `library_view.rs` | Book cards, folder cards, and list rows | Responsive card sizing from `ThumbnailSize`. Selection highlights, hover states, and drag/click handling. |
| `continue_reading.rs`| Continue reading dashboard | Hero featured manga banner, Up Next queue column, and Recently Added responsive cover grid. |
| `details.rs` | Dedicated manga inspector screen | High-resolution cover, romaji/native titles, expandable synopsis, underline tabs, and 4x2 metadata matrix. |
| `collections_view.rs`| Smart and custom collections screen | 2x2 smart collection cards with accent icons; custom collection folder grid with management menus. |
| `history_view.rs` | Reading history screen | Chronological reading log of in-progress and completed books with progress bars and quick navigation. |
| `settings_view.rs` | Split-pane macOS preferences | 6 dedicated categories (General, Appearance, Reader, Library, Shortcuts, Advanced) with native GPUI Kit switches, dropdowns, and tables. |
| `overlays.rs` | Modal dialogs, menus, command prompt, help | Consolidated modal layer. Uses `OverlayStyle` for consistent geometry. Floating command prompt with live auto-completion; shortcuts modal. |
| `reader.rs` | Reader viewport state and continuous strip | Edge-to-edge canvas; autohiding header with fit and reading direction controls; floating progress pill; lateral chevrons. |
| `components.rs` | Shared atomic visual primitives | Window controls, responsive grid math, status badges, and genre tags. |
| `theme.rs` | Theme management and TOML loader | Bridges GPUI Kit themes and custom `themes.toml` files with automatic SHA-256 fingerprint reloading. |

---

## Data and Control Flows

### 1. Import Flow
1. User triggers import via toolbar button, shortcut (`⌘O`/`Ctrl+O`), or drag-and-drop.
2. Background executor scans paths recursively, identifies archives (`.cbz`, `.cbr`, `.zip`, `.rar`) or image directories, and extracts covers.
3. Completed books are streamed sequentially to the UI thread via an `mpsc` channel.
4. UI inserts each book into `Library` and triggers debounced persistence to `$XDG_DATA_HOME/yomu/library.json`.

### 2. Reading and Image Pipeline
1. Opening a book unmounts the desktop shell and mounts the edge-to-edge `Reader`.
2. Reader requests page dimensions from archive metadata.
3. `pagecache::plan_eviction` computes needed prefetch and drop sets around the current viewport.
4. Off-UI threads decode and resize requested pages to match the active viewport scale (`imaging::decode_fit_bgra`).
5. Decoded BGRA pixel buffers are uploaded as GPUI textures.
6. Dropped textures are placed in `garbage` and explicitly released in `Yomu::render` via `window.drop_image`.
7. **Task Cancellation Guarantee:** Prefetch tasks cancelled due to rapid scrubbing are safely dropped without polluting `reader.failed`.

### 3. Navigation and History
1. View state is driven by `Yomu.active_view` (`MainView`).
2. Symmetrical history navigation is tracked in `Yomu.nav_history` (`Vec<NavDestination>`).
3. Entering a folder group or details screen pushes a destination; the toolbar Back button pops back without losing selection or scroll position.

### 4. Input and Command Execution
1. Global and vim key presses pass through `Yomu::on_key`.
2. Vim mode is opt-in via settings or temporary CLI flag (`--vim`).
3. Reader `:` key opens the floating command box. Suggestions filter dynamically against supported commands and themes.
4. Pressing Enter passes the command string to `vim::Vim::feed`, producing typed `Action` values executed by `Yomu::apply`.

---

## Design System Tokens

Yomu renders UI components using centralized semantic tokens derived from the active GPUI Kit theme (`UiColors`):

| Token | Semantic Role |
|---|---|
| `background` | Root window canvas background |
| `surface` | Card surfaces, panels, sidebars, dialogs |
| `surface_hover` | Interactive hover highlight on list items and cards |
| `border` | Subtle 1px structural outlines and dividers |
| `foreground` | Primary text, titles, high-contrast glyphs |
| `muted` | Secondary metadata, subtitles, shortcut hints |
| `accent` | Focus rings, active selections, primary CTA buttons |
| `accent_foreground`| Text rendered over solid accent backgrounds |
| `danger` | Destructive action highlights and delete confirmations |

Corner radii follow user preferences (`CornerRounding::None`, `Small`, `Medium`, `Large`, `Full`).

---

## Persistence and Invariants

- **Storage Locations:**
  - Library data: `$XDG_DATA_HOME/yomu/library.json` (overridable via `YOMU_DATA_DIR`).
  - Generated covers: `$XDG_DATA_HOME/yomu/covers/<book-id>.jpg`.
  - Custom themes: `$XDG_CONFIG_HOME/yomu/themes.toml`.
- **Architectural Invariants:**
  1. Deleting books or collections from the UI **never** removes comic archives or folders from user disk.
  2. Page indices are strictly zero-based internally. Reading direction (LTR vs RTL) alters only visual motion and navigation mapping, never archive indexing.
  3. If `library.json` fails to parse, a timestamped `.corrupt` backup is created automatically to prevent data loss.
  4. **Adaptive Decoded Memory Budget:** Decoded page memory is bounded by `pagecache::system_memory_budget()`, which defaults to 1.0 GiB (`BUDGET_BYTES`) and scales between 512 MiB and 3.0 GiB based on 1/8th of detected host RAM.
  5. **No Synchronous I/O on UI Thread:** Never read files, procfs, or metadata inside the render loop or animation frames.
  6. **Asynchronous Cancellation Safety:** Cancellation is not an error; aborted prefetch tasks must not mark pages as permanently failed.
