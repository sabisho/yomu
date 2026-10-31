# Product & Engineering Roadmap

This document outlines prioritized future work, planned features, and tracked technical debt for Yomu. Items are prioritized based on stability, architectural cleanliness, performance, and user impact.

---

## 🎯 Phase 1: Critical Fixes & Reliability (Completed)

### 1. Prefetch Task Cancellation Recovery
- **Goal:** Prevent cancelled prefetch tasks from marking pages as permanently failed.
- **Status:** Completed.
  - Passing `cancel_token: Arc<AtomicBool>` to `page_decoded` and verifying identity via `Arc::ptr_eq`.
  - Cancellation aborts no longer write to `reader.failed`. Cancelled pages remain requestable on navigation.
  - Added regression unit and UI integration tests verifying cancellation, retry, race resilience, and genuine error handling.

### 2. UI-Thread I/O Elimination
- **Goal:** Remove synchronous filesystem reads from active render and animation loops.
- **Status:** Completed.
  - Cached `pagecache::system_memory_budget()` output via `std::sync::OnceLock<u64>`.
  - Zero disk reads on render, scroll, and animation paths; host memory budget cached once.

### 3. Archive Buffer Allocation Bounds
- **Goal:** Guard against archive allocation bombs in `archive::read_zip_page`.
- **Status:** Completed.
  - Capped initial `Vec::with_capacity` to `INITIAL_PAGE_BUF_CAPACITY` (8 MiB) while streaming up to `MAX_PAGE_BYTES` (256 MiB).
  - Preserves legitimate large pages and rejects oversized entries without masking errors.

---

## 🏗️ Phase 2: Architectural Modularization

### 4. Core Domain GUI-Independence (Phase 2A — Completed)
- **Goal:** Ensure `src/lib.rs` is strictly decoupled from GPUI.
- **Status:** Completed.
  - Relocated `src/scroll.rs` (`PlatformScrollController`) into `src/ui/scroll.rs`.
  - Removed `pub mod scroll;` from `src/lib.rs`. The core domain library is now 100% GUI-independent with zero GPUI dependencies.
  - Retained pure mathematical kinetic decay in `src/motion.rs`.

### 5. Decomposition of `src/main.rs` (Phases 2B, 2C, 2D.1, 2D.2 & 2D.3 — Completed)
- **Goal:** Establish `src/app/` and decompose monolithic orchestration logic out of `src/main.rs`.
- **Status:** In Progress (Phases 2B, 2C, 2D.1, 2D.2, and 2D.3 Completed).
  - **Phase 2B:** Created `src/app/state.rs` extracting `struct Yomu`, `NavDestination`, `ReaderScrollHandle`, `Side`, `ImportEvent`, timing constants, lifecycle initialization, settings helpers, and navigation routing.
  - **Phase 2C:** Created `src/app/reader_controller.rs` extracting reader lifecycle (`open_book`, `close_book`, `pages_listed`), page navigation (`goto_page`, `jump_to`, `turn_by`, `step`), zoom/fit/continuous/webcomics modes, reader gesture handlers (`on_wheel`, `on_pinch`, `drag_start`, `drag_move`, `drag_end`), background decode task queue, cancellation token management, and cache eviction synchronization (`sync_reader`, `request_page`, `page_decoded`).
  - **Phase 2D.1:** Created `src/app/keybindings.rs` extracting keyboard shortcut dispatch (`on_key`), action execution (`apply`), library shortcuts (`on_library_key`), modal overlay shortcuts (`on_overlay_key`), and Vim command-line editing/theme preview synchronization (`in_cmdline`, `cmdline_text`, `replace_cmdline`, `cancel_cmdline`, `sync_theme_preview`, `restore_theme_preview`).
  - **Phase 2D.2:** Created `src/app/persistence.rs` extracting application-level persistence coordination (`save_library`, `save_library_debounced`, `flush_save_background`, `request_quit`, non-blocking zero-I/O `impl Drop for Yomu`), non-blocking asynchronous dispatch across all UI handlers, graceful shutdown flush via `request_quit` and `on_window_should_close`, error recovery dialog with `ForceQuit`, and monotonic sequencing via `save_seq`, lock-free `last_saved_seq: Arc<AtomicU64>`, and background `write_lock: Arc<Mutex<()>>`.
  - **Phase 2D.3:** Created `src/app/desktop.rs` extracting XDG desktop entry generation (`desktop_entry_content`), application and hicolor icon installation across resolutions (`HICOLOR_ICON_SIZES`, `LOGO_PNG`, `app_icon`), and desktop environment integration installer (`install_desktop_integration_to`, `install_desktop_integration`).
  - **Phase 2E.1:** Relocated reader visual presentation responsibilities (`render_reader`, `page_layer`, `reader_header`, `reader_progress_bar`, `lateral_turners`, `ReaderScrollHandle`) from `src/main.rs` into `src/ui/reader.rs`, establishing pure presentation props and callbacks while reducing `src/main.rs` to a concise adapter.
  - **Phase 2E.2:** Created `src/app/actions.rs` extracting menu generation (`open_book_menu`, `open_context_menu`, `open_group_context_menu`, `open_library_menu`), menu interaction tracking (`menu_hover`), action dispatch (`run_book_action`, `perform_book_action`, `run_library_menu_action`), confirmation routing (`confirm_accept`), series dialog handling (`open_series_dialog`, `save_series_dialog`), and book/selection removal workflows (`request_remove_books`, `remove_books`, `request_remove_selected`, `remove_selected`, `remove_book`).
  - **Next Subsystems:** Remaining Phase 2E structural cleanup and window lifecycle orchestration.

---

## ⚡ Phase 3: Performance & Scalability

### 6. Constant-Time Library Item Lookups
- **Goal:** Eliminate $O(N^2)$ linear comparisons during library rendering.
- **Remediation:**
  - Index `Library` items using standard library `HashMap<String, usize>` or ordered index without introducing unnecessary crates.

### 7. Background Persistence Zero-Copy Snapshots
- **Goal:** Eliminate deep-cloning entire `Library` vectors during debounced autosave.
- **Remediation:**
  - Pass lightweight serialization diffs or immutable snapshots to background write threads.

### 8. Dependency & Vendor Hygiene
- **Goal:** Reduce repository footprint and simplify upstream maintenance.
- **Remediation:**
  - Remove dead dependency `memmap2` from `Cargo.toml`.
  - Resolve the `Selectable` trait re-export cleanly and assess removing `vendor/gpui-component`.

---

## 🎨 Phase 4: UI/UX Refinement & Restrained Animations

### 9. Restrained UI Motion Transitions
- **Goal:** Introduce subtle, interruptible desktop motion that strictly adheres to `disable_animations`.
- **Candidates (Starting Guidelines):**
  - Sidebar collapse/expand: ~160–180 ms cubic ease-out width transition.
  - Settings navigation: ~120–140 ms subtle opacity cross-fade.
  - Modal overlay exit: ~120–140 ms fade and scale-down before unmounting.

---

## 📖 Phase 5: Feature Expansion

### 10. Dual-Page Spread Reading Mode
- **Goal:** Display two portrait pages side-by-side to replicate physical manga reading.
- **Behavior:**
  - Automatic single-page fallback for wide landscape pages (spread covers).
  - Respect reading direction (Right-to-Left pairs right page first; Left-to-Right pairs left page first).
  - Toggle via reader key `d` and `:spread [on|off]` command, with persistence in `Settings`.

### 11. Library Sorting Options
- **Goal:** Allow users to sort library cards and list rows beyond the default import order.
- **Criteria:**
  - Sort by: Title (alphabetical/natural), Last Read (recency), Date Added (import timestamp), Page Count, Reading Progress.
  - Direction: Ascending / Descending toggle.
  - Persist active sort preference in `library.json` without altering source file order.

### 12. CB7 / 7z Archive Support
- **Goal:** Support `.cb7` and `.7z` comic archives alongside CBZ, CBR, ZIP, and RAR.

---

## 📋 Completed Features (Archive)

- ✅ **Framework Migration to GPUI Kit 0.8.1 & gpui-pre 0.3.10**
- ✅ **Auto-Hiding Native Scrollbars & Platform Scroll Bounce**
- ✅ **Official GPUI Kit Icon Buttons & ButtonGroup Segmented Controls**
- ✅ **macOS HIG Two-Pane Redesign** (Persistent 240px sidebar, unified toolbar, responsive content views)
- ✅ **Incremental Modularization** (Consolidated library rendering, overlays, command box, help modal, history view)
- ✅ **Kinetic Momentum Scrolling** (`PlatformScrollController` with touch catch, stillness fallback, and reduced motion override)
- ✅ **Safe Desktop Integration** (Explicit CLI installation flags `--install-desktop` / `--install-icons`)
