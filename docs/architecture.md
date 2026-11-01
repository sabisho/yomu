# Architecture

## Runtime Topology

Yomu is structured into three clean architectural tiers:

1. **Core Domain Layer (`src/lib.rs` / `yomu` crate):** GUI-independent Rust (`#![forbid(unsafe_code)]`), decoupled from GPUI, windowing, and presentation frameworks. Combines pure algorithms (geometry calculations, cache eviction planning, Vim grammar parsing, search filtering, selection state machines) with filesystem and persistence operations (archive enumeration, SIMD image decoding and scaling, atomic JSON persistence). Fully testable without a window or graphics compositor.
2. **UI Presentation Layer (`src/ui/`):** Built on GPUI Kit 0.8.1 (`gpui-pre = 0.3.10` runtime). Encapsulates views, components, dialogs, theme loading, and platform-adaptive momentum scrolling adapters (`ui::scroll`).
3. **Application Orchestration (`src/app/` & `src/main.rs`):** Manages the application window lifecycle, top-level state routing, input dispatch, background decode task scheduling, GPU texture garbage collection, and debounced persistence.

```text
GPUI Kit Window / Yomu Application (src/main.rs / src/app/)
  ├─ Application Orchestration (`src/app/`, `src/main.rs`)
  │    ├─ State & Router (`src/app/state.rs`) ── root state, navigation history, lifecycle init
  │    ├─ Reader Controller (`src/app/reader_controller.rs`) ── page decode tasks, prefetch queue, navigation, gestures
  │    ├─ Keybindings (`src/app/keybindings.rs`) ── keyboard event routing, Vim tokens, action execution
  │    ├─ Persistence (`src/app/persistence.rs`) ── debounced autosave, atomic writes, background flush coordination
  │    ├─ Actions & Menus (`src/app/actions.rs`) ── context menus, action dispatch, confirmations, removal workflows
  │    ├─ View Adapters (`src/app/views/`) ── shell, library, continue reading, collections, history, details, settings
  │    ├─ Desktop (`src/app/desktop.rs`) ── desktop entry generation, icon installation, XDG integration
  │    └─ Main (`src/main.rs`) ── entry point, window setup, and event dispatch
  ├─ Presentation Shell (`src/ui/`)
  │    ├─ Sidebar (src/ui/sidebar.rs) ── persistent 240px navigation
  │    ├─ Toolbar (src/ui/toolbar.rs) ── search, view mode, selection bar
  │    ├─ Library View (src/ui/library_view.rs) ── cards, folders, list rows
  │    ├─ Continue Reading (src/ui/continue_reading.rs) ── hero banner & queue
  │    ├─ Manga Details (src/ui/details.rs) ── metadata matrix & synopsis
  │    ├─ Collections (src/ui/collections_view.rs) ── smart & custom folders
  │    ├─ History (src/ui/history_view.rs) ── in-progress & completed manga
  │    ├─ Settings (src/ui/settings_view.rs) ── 6-tab split-pane preferences
  │    ├─ Overlays (src/ui/overlays.rs) ── dialogs, menus, help
  │    ├─ Reader (src/ui/reader.rs) ── edge-to-edge canvas with autohiding HUD
  │    └─ Momentum Controller (src/ui/scroll.rs) ── platform momentum adapter
  ├─ Core Library (src/library.rs) ── JSON persistence & XDG data management
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
| `layout.rs` | Fit modes (Page, Width, Height, Original), zoom calculation, viewport geometry, decode tiers, dual-page spreads, and mixed-source SpreadPlan | Pure math. Page indices are strictly zero-based and ordered in reading sequence regardless of RTL display travel. Spread plans precalculate single-image vs two-image spreads with zero per-frame allocation. |
| `motion.rs` | Frame-time scroll easing, page-turn transitions, reduced-motion detection | Driven by delta time ($dt$), not frame counts. Pure analytical decay; zero UI dependencies. |
| `pagecache.rs` | Prefetch order and texture eviction planning | Pure policy. Preserves current page texture even if it exceeds budget. Adaptive decoded image budget: defaults to 1.0 GiB (`BUDGET_BYTES`) and scales between 512 MiB and 3.0 GiB based on 1/8th of detected host RAM. Static metrics cached to prevent UI-thread I/O. |
| `vim.rs` | Parse key tokens, counts, and motions into typed `Action`s | Pure parser without UI dependencies. The UI applies the resulting `Action` enum. |
| `selection.rs` | Multi-selection state, toggle, and range selection anchors | Pure state machine. Emits selected IDs in stable library order for deterministic batch operations. |
| `search.rs` | Case-insensitive title, source path, and series substring matching | Pure helper. Query state is transient in the UI and never alters persisted library order. |
| `menu.rs` | Context menu item generation, validation, and confirmation requirements | Decides available book and library actions; UI renders and dispatches them. |
| `util.rs` | Natural sorting, FNV-1a path hashing, date/size formatting | Reusable pure helpers. |

### UI & Presentation Modules (`src/ui/`)

| Module | Responsibility | Design & Integration |
|---|---|---|
| `scroll.rs` | Cross-platform momentum scrolling coordinator (`PlatformScrollController`) | Connects pure `motion::KineticScroll` to GPUI `ScrollHandle`. Preserves native AppKit momentum on macOS; provides kinetic glide on Linux and Windows. Located in `src/ui/scroll.rs`. |
| `sidebar.rs` | Persistent 240px navigation sidebar | Native macOS traffic lights on macOS; clean sections on Linux/Windows. Navigates between primary sections with badge counts. |
| `toolbar.rs` | Unified 52px header toolbar | Symmetrical back button, centered search with `⌘K`/`Ctrl+K` shortcut, view mode toggle, import action, and in-place selection toolbar. |
| `library_view.rs` | Book cards, folder cards, and list rows | Responsive card sizing from `ThumbnailSize`. Selection highlights, hover states, and drag/click handling. |
| `continue_reading.rs`| Continue reading dashboard | Hero featured manga banner, Up Next queue column, and Recently Added responsive cover grid. |
| `details.rs` | Dedicated manga inspector screen | High-resolution cover, romaji/native titles, expandable synopsis, underline tabs, and 4x2 metadata matrix. |
| `collections_view.rs`| Smart and custom collections screen | 2x2 smart collection cards with accent icons; custom collection folder grid with management menus. |
| `history_view.rs` | Reading history screen | Chronological reading log of in-progress and completed books with progress bars and quick navigation. |
| `settings_view.rs` | Split-pane macOS preferences | 6 dedicated categories (General, Appearance, Reader, Library, Shortcuts, Advanced) with native GPUI Kit switches, dropdowns, and tables. |
| `overlays.rs` | Modal dialogs, menus, help | Consolidated modal layer. Uses native GPUI Kit components; shortcuts modal. |
| `reader.rs` | Reader viewport state, continuous strip, and visual presentation | Edge-to-edge canvas with decoded page layers (`page_layer`); autohiding header with fit mode and window controls (`reader_header`); lateral turn chevrons (`lateral_turners`); and continuous vertical scrollbar integration (`ReaderScrollHandle`). |
| `components.rs` | Shared atomic visual primitives | Window controls, responsive grid math, status badges, and genre tags. |
| `theme.rs` | Theme management and TOML loader | Bridges GPUI Kit themes and custom `themes.toml` files with automatic SHA-256 fingerprint reloading. |

### Application Orchestration (`src/app/` & `src/main.rs`)

| Module | Responsibility | Invariants & Boundaries |
|---|---|---|
| `state.rs` | Root application state (`struct Yomu`), navigation history, and lifecycle initialization | Holds `pub(crate)` fields for `Yomu` across binary crate modules; implements `new`, `with_dirs`, navigation router (`navigate_to`, `navigate_to_dest`, `apply_destination`, `navigate_back`, `navigate_forward`), search activation, and theme file watcher. |
| `reader_controller.rs` | Reader lifecycle, background decoding queue, prefetch planning, zoom/fit/continuous modes, and gestures | Decoupled from GPUI presentation views; coordinates async archive decoding and prefetch scheduling off UI thread; strictly preserves Phase 1 task identity and cancellation invariants. |
| `keybindings.rs` | Keyboard event routing, Vim token translation, overlay shortcuts, and action execution | Implements `on_key`, `apply`, `on_library_key`, and `on_overlay_key` on `Yomu`; respects Vim-mode settings, overlay priority, and platform modifier standards. |
| `persistence.rs` | Application persistence, debounced autosave, atomic filesystem synchronization, and graceful shutdown flush | Manages `save_library` (immediate non-blocking background dispatch), `save_library_debounced` (500ms trailing timer for page turns/scrolling), `flush_save_background`, `request_quit` (async wait on in-flight saves before exiting), non-blocking zero-I/O `Drop for Yomu`, full-window native GPUI pointer occlusion barrier, and error recovery confirmation dialog with `ForceQuit`; enforces monotonic `save_seq` ordering via `Arc<AtomicU64>`, background-only `write_lock: Arc<Mutex<()>>`, and 4-tier shutdown mutation isolation; guarantees zero synchronous filesystem I/O or mutex contention on the UI thread during rendering, scrolling, or user interaction. |
| `actions.rs` | Context menu generation, action dispatch, confirmation dialog routing, and removal workflows | Encapsulates book/group/library context menus (`open_book_menu`, `open_context_menu`, `open_group_context_menu`, `open_library_menu`), menu hover tracking (`menu_hover`), action routing (`run_book_action`, `perform_book_action`, `run_library_menu_action`), confirmation routing (`confirm_accept`), series dialog handling (`open_series_dialog`, `save_series_dialog`), and book/selection removal pipelines (`request_remove_books`, `remove_books`, `request_remove_selected`, `remove_selected`, `remove_book`); strictly enforces shutdown `is_quitting` guards and delegates persistence to `persistence.rs`. |
| `views/` | View presentation adapters bridging application state and presentation modules | Subdivided into focused adapters: `shell.rs` (two-pane shell, sidebar/toolbar adapters), `library.rs` (grid/list rows, cards, all manga, downloads), `continue_reading.rs` (continue reading dashboard), `collections.rs` (smart and custom collection views), `history.rs` (reading history log), `details.rs` (manga inspector details), and `settings.rs` (settings modal view and tab callback wiring). |
| `desktop.rs` | Desktop environment integration, application icons, and XDG desktop entry installation | Implements `desktop_entry_content`, `app_icon`, `install_desktop_integration_to`, and `install_desktop_integration`; embeds logo and hicolor icons across standard desktop resolutions; preserves user customizations when `--force` is omitted; platform-gated for Linux with graceful non-Linux bails. |
| `main.rs` | Executable entry point, window initialization, GPUI lifecycle, event dispatch, and frame coordination | Focused on top-level runtime startup, action dispatch, window event handlers, reader adapter, and `Render` trait implementation (frame timing, texture GC, shutdown barrier). |

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

### 4. Input and Navigation Execution
1. Global and vim key presses pass through `Yomu::on_key`.
2. Vim mode is opt-in via settings or temporary CLI flag (`--vim`).
3. When Vim mode is enabled, reading navigation and motions pass to `vim::Vim::feed`, producing typed `Action` values executed by `Yomu::apply`.

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
  7. **Mixed-Source Spread Support:** In dual-page mode, extra-wide single-image double-page spreads are detected automatically via aspect ratio threshold and comic context, or forced via stable archive entry overrides (`Book.spread_overrides`). Single-image spreads occupy an entire spread alone and are never paired with adjacent pages; normal pages pair into two-image spreads; and isolated normal pages remain alone. Zero pages are skipped, duplicated, or re-ordered. Logical progress strictly tracks archive entry indices.

---

## Motion & Animation Architecture

Yomu adheres to a restrained, desktop-native motion language built exclusively on official **GPUI Kit 0.8.1** animation APIs. Motion is applied purposefully for spatial continuity and tactile feedback, never as decorative distraction or performance overhead.

### 1. Toolkit Animation Mechanisms
Animation in Yomu leverages the native capabilities provided by the `gpui-kit` / `gpui-base` / `gpui-component` 0.8.1 ecosystem:
- **`EffectTransition` (`gpui_kit::base::animation::EffectTransition`):** Declarative, composable transition combinator for GPUI elements. Supports `.fade(from, to)`, `.slide_y(from, to)`, `.slide_x(from, to)`, `.width(from, to)`, `.height(from, to)`, and easing curves (`ease_out_cubic`). Applied directly to elements via `.apply(element, element_id)`.
- **Value Motion & Springs (`gpui_kit::base::motion::*`):** Pure functional interpolation primitives (`transition`, `spring`, `Presence`, `animate_keyframes`) available for scalar property changes.
- **`with_animation` Execution Loop:** Native GPUI frame hook that automatically bypasses frames and settles instantaneously when `cx.reduce_motion()` is active or duration is zero.
- **Native Component Transitions:**
  - `Sidebar`: Built-in offcanvas sliding transition (`SidebarCollapsible::Offcanvas`, 200ms cubic ease).
  - `Dialog` & `AlertDialog`: Built-in native open/dismiss cubic bezier scale and fade transitions.
  - `Spinner`: Built-in continuous rotational animation driven by `with_animation`.

### 2. Implemented Motion Patterns
| Transition Area | Location | Mechanism & Curve | Visual Behavior | Trigger |
|---|---|---|---|---|
| **Settings Tab Switch** | `src/ui/settings_view.rs` | `EffectTransition` (`ease_out_cubic`, 140ms) | Subtle opacity drift ($0.85 \to 1.0$) on tab container | Category selection change in Settings sidebar |
| **Main View Switch** | `src/app/views/shell.rs` | `EffectTransition` (`ease_out_cubic`, 140ms) | Subtle opacity drift ($0.90 \to 1.0$) on view canvas | Navigation between All Manga, History, Details, etc. |
| **Reader Help Modal** | `src/ui/overlays.rs` | `EffectTransition` (`ease_out_cubic`, 120ms) | Micro vertical slide ($-6\text{px} \to 0\text{px}$) & fade ($0.80 \to 1.0$) | Keyboard shortcut overlay toggle (`?`) in Reader |
| **Sidebar Collapse** | GPUI Kit `Sidebar` | Built-in `EffectTransition` (200ms) | Native horizontal off-canvas slide | Sidebar toggle (`⌘B` / `Ctrl+B`) |
| **Modal & Alert Dialogs** | GPUI Kit `Dialog` | Built-in dialog transition | Native cubic bezier entrance and fade | Delete confirmations, edit series, info popups |
| **Loading Indicators** | GPUI Kit `Spinner` | Built-in continuous animation | Smooth 360-degree rotation | Async cover decodes and library imports |

### 3. Removed Custom Implementations
All legacy, ad-hoc, and custom animation workarounds have been eradicated:
- Removed obsolete custom `reader_progress_bar` and custom progress animations.
- Removed custom frame-driven interpolation timers and manual tick accumulators.
- No redundant custom wrappers or styling that override native GPUI Kit motion behavior.

### 4. High-Frequency Surfaces Intentionally Kept Static
To maximize rendering performance, battery life, and visual stability, high-frequency surfaces are intentionally static:
- **Library Grid & List Items:** Cards and rows update instantly without staggered cascading entrances or layout reflow animations.
- **Search Filtering:** Results render immediately as query tokens are parsed, avoiding visual jitter during rapid typing.
- **Decoded Comic Page Delivery:** Decoded BGRA buffers upload directly to GPU textures without crossfade or zoom animations to preserve comic line art clarity.
- **Bulk Selection & Toggles:** Checkboxes and selection rings snap immediately with zero bounce.

### 5. Permanent Animation Policy
Yomu enforces a strict policy where all animations remain permanently enabled across all platforms and environments:
1. **No Application Animation Disable Setting:** There is no toggle or configuration option in Settings or persistence structures to disable or reduce animations.
2. **Independent of Operating System Reduced-Motion Preferences:** Yomu does not follow OS-level reduced-motion settings (such as GNOME/KDE XDG portals, macOS Accessibility, or Windows Visual Effects).
3. **GPUI Runtime Enforcement:** Application startup and window initialization explicitly set `cx.set_reduce_motion(false)`, ensuring the GPUI runtime executes all frame-driven animations, transitions, and component motion at full fidelity.
4. **Backward Compatibility:** Older configuration files containing `disable_animations` load safely without error, with animations staying active.

