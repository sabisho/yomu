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

### 4. Core Domain GUI-Independence
- **Goal:** Ensure `src/lib.rs` is strictly decoupled from GPUI.
- **Remediation:**
  - Move `src/scroll.rs` (`PlatformScrollController`) into `src/ui/scroll.rs`.
  - Retain pure mathematical kinetic decay in `src/motion.rs`.

### 5. Decomposition of `src/main.rs`
- **Goal:** Reduce `src/main.rs` (5,426 lines) to a clean binary entry point (< 100 lines).
- **Proposed Module Structure:**
  - `src/app/state.rs`: `Yomu` entity definition, navigation history, and view router.
  - `src/app/reader_controller.rs`: Async decode queue, prefetch coordination, texture drop lifecycle.
  - `src/app/keybindings.rs`: Keyboard event routing, shortcuts, and Vim action dispatch.
  - `src/app/desktop.rs`: Client-side decorations, app icon, and desktop entry installation.
  - `src/app/persistence.rs`: Debounced background atomic autosave scheduler.

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
