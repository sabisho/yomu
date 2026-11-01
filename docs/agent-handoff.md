# Agent Handoff

This document is the current operational snapshot for coding agents working on Yomu. Read it after [AGENTS.md](../AGENTS.md). For structural invariants and module boundaries, see [docs/architecture.md](architecture.md); for user-facing features and keybindings, see [README.md](../README.md).

---

## 1. Current Implementation Status

Yomu is a high-performance desktop manga and comic reader built in Rust 2024 using **GPUI Kit 0.8.1** (`gpui-pre = 0.3.10` runtime). It operates with `#![forbid(unsafe_code)]` in safe Rust.

- **Primary Target:** Linux (Wayland-native with X11 fallback; Nix/devenv native packaging).
- **Secondary Targets:** macOS and Windows compatibility maintained through abstraction layers.
- **UI Architecture:** Modern macOS HIG two-pane design (persistent 240px sidebar, context-aware 52px toolbar, responsive content views, edge-to-edge immersive reader with native `ScrollbarMode::Scrolling` auto-hide scrollbars and boundary scroll bounce).
- **Test Baseline:** **294 active tests** passing (131 core library unit tests in `src/lib.rs`, 163 binary-target UI tests in `src/main.rs`, plus 3 opt-in performance benchmarks; 297 total).
- **Clean Toolchain:** Zero compiler errors, zero warnings under `cargo check` and `cargo clippy --all-targets -- -D warnings`, fully formatted under `cargo fmt`.

---

## 2. Completed Audit & Standards Validation (Phase 6)

### Architecture & Performance Audit Validation
A comprehensive audit of the entire codebase was conducted and validated against the actual codebase and GPUI Kit 0.8.1 dependency tree:

1. **Critical Defect: Prefetch Cancellation Bug**
   - *Status:* Validated. `page_decoded` treats worker task aborts (`"page decode cancelled"`) as permanent decode failures in `reader.failed`, permanently bricking pages when users scrub rapidly and return.
   - *Remediation Planned:* Differentiate cancellations from decoding errors and ensure cancelled tasks are never added to `reader.failed`.
2. **Critical Defect: UI-Thread Synchronous I/O**
   - *Status:* Validated. `pagecache::system_memory_budget()` executes `std::fs::read_to_string("/proc/meminfo")` on every animation frame via `sync_reader`.
   - *Remediation Planned:* Cache the budget once at startup via `std::sync::OnceLock<u64>` (adhering to the adaptive 512 MiB to 3.0 GiB policy with 1.0 GiB fallback).
3. **Architectural Boundary Leak:**
   - *Status:* Validated. `src/lib.rs` exports `pub mod scroll;`, which imports `gpui_kit`, violating GUI-independent domain separation.
   - *Remediation Planned:* Relocate `scroll.rs` to `src/ui/scroll.rs`.
4. **Main Module Congestion:**
   - *Status:* Validated. `src/main.rs` stands at 5,426 lines and coordinates windowing, gesture recognition, Vim token dispatch, and rendering wrappers.
   - *Remediation Planned:* Decompose into `src/app/` (`state.rs`, `reader_controller.rs`, `keybindings.rs`, `desktop.rs`, `persistence.rs`).
5. **Collection Scalability:**
   - *Status:* Validated. `Library::get()` is an $O(N)$ linear scan, leading to $O(N^2)$ comparisons during card grid rendering.
   - *Remediation Planned:* Use a secondary index `HashMap<String, usize>` without introducing unnecessary external dependencies.
6. **Dependency & Vendor Hygiene:**
   - *Status:* Validated. `memmap2` is completely unused in `Cargo.toml`. `vendor/gpui-component` is maintained solely for a single re-export of `Selectable`.

---

## 3. Known Limitations & Technical Debt

1. **Wayland Compositor Visual Verification:**
   - Headless simulated GPUI tests validate layout, interaction, focus, and state transitions, but do not validate visual appearance (translucent surfaces, backdrop blur, compositor shadows) under real Wayland compositors (e.g., Niri, Sway, GNOME Mutter, KDE KWin).
   - *Rule for agents:* Never claim visual verification on Wayland unless manually inspected on a real desktop session.
2. **Congestion in `src/main.rs` & `src/tests.rs`:**
   - `src/main.rs` (5,426 lines) and `src/tests.rs` (3,601 lines) are oversized and require modular extraction.
3. **Cross-Platform Momentum Scrolling:**
   - Linux and Windows rely on `PlatformScrollController`'s kinetic inertia glide loop because GPUI does not yet synthesize momentum events on non-macOS platforms.
   - *Action required:* Monitor official GPUI Kit releases for native cross-platform momentum support.

---

## 4. Next Recommended Engineering Tasks

Refer to [ROADMAP.md](../ROADMAP.md) for prioritized upcoming work:

1. **Phase 1: Critical Fixes & Reliability (Completed):**
   - Resolved prefetch cancellation defect in `request_page` and `page_decoded`.
   - Cached `/proc/meminfo` memory budget via `OnceLock`.
   - Bounded initial allocation capacity in `archive::read_zip_page` to 8 MiB while supporting pages up to 256 MiB.
2. **Phase 2: Architectural Modularization:**
   - **Phase 2A (Completed):** Relocated `src/scroll.rs` to `src/ui/scroll.rs`, fully decoupling `src/lib.rs` from GPUI.
   - **Phase 2B (Completed):** Established `src/app/` module; extracted `app/state.rs` with `Yomu` root state, navigation router, and lifecycle initialization.
   - **Phase 2C (Completed):** Extracted `app/reader_controller.rs` handling reader lifecycle, async decode queue, prefetch cancellation safety, zoom/fit/reading modes, and gesture navigation.
   - **Phase 2D.1 (Completed):** Extracted `app/keybindings.rs` handling keyboard shortcut event dispatch (`on_key`), Vim actions (`apply`), overlay keys (`on_overlay_key`), library keys (`on_library_key`), and command-line prompt editing / live theme preview sync.
   - **Phase 2D.2 (Completed & Remediated):** Extracted `app/persistence.rs` and resolved UI persistence concurrency:
     - Replaced all ~38 synchronous `save_library()` calls across UI handlers with non-blocking asynchronous dispatch (`save_library(cx)` for discrete actions, `save_library_debounced(cx)` for page turns and scroll progress).
     - Replaced blocking mutex checks on the UI thread with lock-free `last_saved_seq: Arc<AtomicU64>` and serialized background writes via `write_lock: Arc<Mutex<()>>`.
     - Implemented graceful async shutdown (`request_quit(cx)`), window close interception (`on_window_should_close` and CSD titlebar), non-blocking zero-I/O `Drop for Yomu`, full-window native GPUI pointer occlusion barrier (`HitboxBehavior::BlockMouse`), and error recovery dialog with `ForceQuit`.
     - Enforced 4-tier shutdown isolation: presentation occluding barrier, root action guards, domain mutation boundaries, and `save_library` safety gates.
     - Implemented cooperative import cancellation with `Arc<AtomicBool>` token checkpointed across directory traversal, candidate enumeration, archive reads, cover generation, and book reporting.
     - Protected asynchronous import completion loops and file-picker error callbacks against mutating application state or presenting UI toasts during shutdown.
     - Added 23 deterministic concurrency, monotonicity, shutdown, input barrier, import cancellation, token identity, and ForceQuit regression tests in `src/app/persistence.rs` (236 total active tests passing: 106 lib, 130 bin).
   - **Phase 2D.3 (Completed):** Extracted `app/desktop.rs` handling XDG desktop entry content (`desktop_entry_content`), application window icon decodes (`app_icon`), standard hicolor icon resolution tables (`HICOLOR_ICON_SIZES`), and Linux desktop integration installer (`install_desktop_integration_to`, `install_desktop_integration`) with conditional compilation.
   - **Phase 2E.1 (Completed):** Relocated reader visual presentation from `src/main.rs` into `src/ui/reader.rs`:
     - Moved `render_reader`, `page_layer`, `reader_header`, `reader_progress_bar`, `lateral_turners`, and `ReaderScrollHandle` into `src/ui/reader.rs`.
     - Established pure `ReaderProps` and decoupled presentation rendering from `Yomu` application entity state via standard event callbacks.
     - Reduced `src/main.rs` by >300 lines to a concise adapter calling `ui::reader::render_reader(props, ...)`.
     - Added unit tests for `ReaderScrollHandle` and `reader_progress_bar` (241 total active tests passing: 106 lib, 135 bin).
   - **Phase 2E.2 (Completed):** Extracted `src/app/actions.rs` for menus, dialogs, and action orchestration:
     - Relocated book/group/library context menu generation (`open_book_menu`, `open_context_menu`, `open_group_context_menu`, `open_library_menu`), menu hover selection tracking (`menu_hover`), action routing (`run_book_action`, `perform_book_action`, `run_library_menu_action`), confirmation routing (`confirm_accept`), series dialog handling (`open_series_dialog`, `save_series_dialog`), and removal workflows (`request_remove_books`, `remove_books`, `request_remove_selected`, `remove_selected`, `remove_book`).
     - Preserved all shutdown guards (`is_quitting` checks), confirmation flows, selection semantics, toast feedback, and persistence coordination.
     - Full test suite passing (241 total active tests: 106 lib, 135 bin).
   - **Phase 2E.3 (Completed):** Extracted `src/app/views/` for shell and view presentation adapters:
     - Relocated view adapters into dedicated submodules: `shell.rs` (two-pane shell, sidebar/toolbar adapters), `library.rs` (card/list items, all manga, downloads), `continue_reading.rs` (dashboard hero banner and up next queue), `collections.rs` (smart and custom collection grids), `history.rs` (reading history log), `details.rs` (manga inspector screen), `settings.rs` (settings modal view and tab callback wiring), and `overlays.rs` (modal dialogs, context menus, and bottom command box).
     - Maintained `render_reader` in `src/main.rs` as the direct top-level reader branch adapter for `Render for Yomu`.
     - Reduced `src/main.rs` to ~1,250 lines, focused on application startup, root `Render` implementation (frame timing, texture GC, shutdown barrier), and root event listeners.
     - Full test suite passing (241 total active tests: 106 lib, 135 bin).
3. **Phase 3: Scalability & Performance:**
   - **Phase 3.1 (Completed):** Eliminated synchronous filesystem operations from UI render paths:
     - Replaced per-book `b.path.exists()` in `render_two_pane_shell` with cached `self.downloads_count` computed on the background executor (`refresh_downloads_count`) with monotonic `downloads_seq` sequence tracking and explicit invalidation upon imports, removals, clear all data, and navigation.
     - Replaced synchronous `props.cover_path.exists()` in `ui::details::render_manga_details` with cached `cover_available: bool` in `MangaDetailsProps` driven by background executor checks (`check_details_cover`) with stale-result protection on book change (`details_cover_seq` and `selected_book_id` match).
     - Added 4 UI integration regression tests in `src/tests.rs` (245 active tests passing: 106 lib, 139 bin).
   - **Phase 3.2 (Completed):** Eliminated quadratic $O(N^2)$ book lookups and per-frame string allocations in library rendering (`src/app/views/library.rs`):
     - Replaced `order.iter().filter_map(|id| self.library.get(id))` in `Yomu::render_all_manga_view` with direct contiguous iteration over `self.library.books()` in ungrouped mode.
     - For open series groups, directly borrowed `group.books` from `directory_groups()` without intermediate ID vector allocation or repeated linear scans.
     - Added UI integration regression test in `src/tests.rs` (246 active tests passing: 106 lib, 140 bin).
   - **Phase 3.3A (Completed):** Eliminated synchronous filesystem checks and path re-allocation from render-time cover paths:
     - Precomputed base cover directories in `DataDirs`, replacing multi-join `root.join("covers").join(size.dir_name()).join(format!("{id}.jpg"))` with a single join on pre-resolved base paths.
     - Removed synchronous `.exists()` disk `stat` syscalls from `DataDirs::cover_for_size()`.
     - Preserved legacy unsized cover compatibility via automatic startup migration in `DataDirs::at()` and in-memory fallback ID tracking (`legacy_fallback_ids`).
     - Added unit and UI tests for sized, legacy, missing, and fallback cover handling (250 active tests passing: 109 lib, 141 bin).
   - **Phase 3.3B (Completed):** Reduced heap allocations in library card and list-row rendering:
     - Replaced redundant `book.id.clone()` in card and row event handlers with `Arc<str>` reference counts (`Arc::from(book.id.as_str())`), halving book ID string allocations per card and row.
     - Decoupled folder card/row right-click handlers from member ID vector cloning, extracting `open_group_context_menu_by_key` to resolve group IDs on demand only when the context menu is opened.
     - Replaced `HashSet<String>` with borrowed `HashSet<&str>` across `render_all_manga_view` and `render_continue_reading_view`, eliminating hundreds of transient heap string clones per frame.
     - Avoided redundant `name: String` allocation in `directory_groups()` for duplicate members of existing directory buckets.
     - Full test suite passing (250 active tests: 109 lib, 141 bin).
   - **Phase 3.3C (Completed):** Eliminated reader frame-loop allocations, linear lookups, and redundant string cloning:
     - Cached `book_title: SharedString` on `Reader` at initialization, eliminating per-frame $O(N)$ linear scans of `self.library` and title string allocations during reading.
     - Replaced `keep: HashSet<usize>` with scalar range `RangeInclusive<usize>` checks, eliminating per-frame `HashSet` allocations in both single-page and continuous modes.
     - Optimized `pagecache::plan_eviction_with_keep` to allocate the `remaining` collection only when `total > budget`, achieving zero heap allocations during steady-state cache checks.
     - Replaced `wanted_set: HashSet<usize>` with direct slice iteration (`!wanted.iter().any(...)`), eliminating `HashSet` creation on background prefetch cancellation checks.
     - Removed redundant `.to_string()` calls in `reader_header`, consuming `SharedString` directly into GPUI layout elements.
     - Eliminated repeated `reader.pages.clone()` from `sync_reader` and `request_page`, removing 7–10 atomic reference count increments/decrements per frame.
     - Reused `reader.entries_scratch` buffer in `sync_reader` to avoid allocating `Vec<Entry>` per frame.
     - Full test suite passing (255 active tests: 112 lib, 143 bin).
   - Implement $O(1)$ lookup indexing for `Library` with standard library collections.
   - Eliminate deep library cloning on background autosave.
    - **Phase 4.1 (Completed):** Implemented persistent library sorting for All Manga and Downloads:
      - Modes: Title (A–Z / Z–A natural sort), Date Added (Newest / Oldest with backward-compatible mtime backfill), Last Read (recency), Reading Progress (Highest / Lowest), and Default import order.
      - Toolbar dropdown menu with checkmarks, section separators, and contextual direction labels.
      - Settings persistence with lenient fallback and full preservation of underlying library order.
      - Full test suite passing (266 active tests: 119 lib, 147 bin).
    - **Phase 4.2 (Completed):** Implemented native dual-page spread reading mode:
      - Defined `PageLayout::Single` and `PageLayout::Double` with responsive portrait fallback (`is_dual_spread_effective`).
      - Defined `ReadingDirection::RightToLeft` (Manga) and `ReadingDirection::LeftToRight` (Western comics) with directional arrow, click zone, and slide travel inversion.
      - Added `first_page_is_cover` toggle to isolate page 0 before pairs begin.
      - Implemented common-height spread scaling preserving individual page aspect ratios and `SPREAD_GAP = 2.0`.
      - Spread-aware navigation (`turn_by` advances/retreats by spread), prefetching (prioritizes visible spread pages at `current_tier`), and eviction keep-window protection.
      - Header layout dropdown button (`reader-layout-toggle`) and Settings Reader tab controls.
      - Full test suite passing (276 active tests: 125 lib, 151 bin).
    - **Phase 4.2.1 (Completed):** Implemented mixed-source spread support:
      - Supported 3 page types in a single archive: normal portrait page, two-image spread, and single-image spread.
      - Automatic detection of extra-wide single-image spreads using aspect ratio threshold (`DEFAULT_SPREAD_RATIO_THRESHOLD = 1.25`, configurable in Settings) and context awareness (portrait vs landscape comic detection).
      - Lightweight dimension probing without full decompression via `imaging::probe_dimensions` and `archive::probe_page_dimensions`.
      - Manual spread classification overrides per page (`Auto`, `ForceSingle`, `ForceSpread`) persisted in `Book.spread_overrides` using stable archive entry identifiers, with persistent dimension cache in `Book.page_dimensions`.
      - Spread planning with precomputed `SpreadPlan` cached on `Reader` ensuring $O(1)$ lookups and zero per-frame allocations during reading.
      - Single-image spreads occupy an entire spread alone and are never paired with adjacent pages; normal pages pair into two-image spreads; and isolated normal pages remain alone. Zero pages skipped, duplicated, or re-ordered.
      - Reader header layout dropdown integration with current page override menu, Settings Reader tab ratio threshold control, and keybinding `'s'` to cycle overrides in reader mode.
      - Full test suite passing (**289 active tests**: 131 lib, 158 bin).
4. **Phase 4.3: Restrained UI Motion Transitions:**
   - Subtle, interruptible desktop motion that strictly adheres to `disable_animations`.
