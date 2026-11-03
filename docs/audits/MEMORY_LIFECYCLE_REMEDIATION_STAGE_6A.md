# Yomu — Stage 6A: Memory Lifecycle & GPU Resource Remediation Report

**Date:** 2026-11-04  
**Author:** Antigravity (Advanced Agentic Pair Programmer)  
**Status:** Completed & Verified  
**Baseline Test Matrix:** 346 active tests passing (145 lib unit tests, 201 binary UI tests, 13 ignored benchmarks; 359 total)  
**Toolchain:** Safe Rust 2024 (`rustc 1.99.0`), GPUI Kit 0.8.1 (`gpui-pre = 0.3.10`), NixOS/devenv  
**Safety Mandate:** `#![forbid(unsafe_code)]` preserved across all crates; zero unsafe Rust introduced.

---

## 1. Executive Summary

Stage 6A executes a targeted remediation of the primary memory lifecycle defects identified in the deep memory and SSD I/O audit (`docs/audits/MEMORY_LEAK_AND_SSD_IO_AUDIT.md`):

1. **Test-Count Reconciliation:** Fully diagnosed and resolved the 10-test discrepancy between Stage 4 (342 tests) and Stage 5 (332 tests). Recovered all missing regression tests and benchmarks accidentally truncated during the previous audit, establishing a verified baseline of 342 active tests before making functional changes.
2. **Finding M1 (Reader Resource Cleanup & Task Cancellation):** Remediated the direct book-switching lifecycle in `src/app/reader_controller.rs`, `src/app/persistence.rs`, and `src/main.rs`. Replaced direct reader assignment with `cleanup_current_reader()`, which actively signals all outstanding background decode cancellation tokens (`cancellations`, `probe_cancel`), drains previously decoded CPU images and GPU texture handles via `reader.into_images()`, and schedules their release through `drain_garbage()` using `window.drop_image()`. Prevents background executor task leaks, CPU heap retention, and GPU texture atlas fragmentation across rapid book switching and application resets.
3. **Finding M2 (Bounded Cover Asset Retention & GPU Tile Eviction):** Audited the official GPUI 0.3.10 asset architecture and resolved the unbounded cover cache problem without forking GPUI or introducing external dependencies. Implemented `BoundedCoverCache` in `src/ui/cover_cache.rs` using standard library data structures (`std::collections::{HashMap, VecDeque}`) implementing the native `gpui_kit::ImageCache` trait with an LRU eviction policy capped at 256 covers (~138 MB limit). Integrated it into the application root shell via `gpui_kit::image_cache()`, automatically intercepting all child `img()` cover elements across the virtual library grid and sidebar. When capacity is exceeded, oldest items are evicted, dropping CPU heap buffers and explicitly calling `cx.drop_image()` to release GPU atlas tiles.
4. **Validation:** Introduced 4 dedicated regression tests verifying reader cancellation, race condition immunity, cache clearing, and cover cache LRU capacity enforcement. The complete test suite now passes with **346 active tests** with zero warnings under `clippy -D warnings` and clean `cargo fmt`.

---

## 2. Test-Count Discrepancy Reconciliation & Resolution

### A. Root Cause Analysis
During Stage 5, the test baseline was reported as 332 active tests, whereas Phase 8 had concluded with 342 active tests. An investigation of Git history and transcript logs revealed that in step 231 of the previous session, an `edit_file` tool call targeting the end of `src/tests.rs` replaced an anchor string but inadvertently omitted 10 active regression tests and 9 benchmark functions that had been appended in Phases 6, 7, and 8.

### B. Inventory of Lost Tests
The truncated region contained the following active regression tests:
1. `test_cancelled_task_does_not_pollute_failed_and_leaves_page_requestable` (Phase 6 cancellation safety)
2. `test_sustained_long_reading_session_stability` (Phase 6 long-session soak test)
3. `batch_import_coalesces_saves_and_persists_on_finish` (Phase 7 persistence durability)
4. `shutdown_flushes_pending_debounced_save` (Phase 7 shutdown flush guarantee)
5. `batch_import_persists_on_cancellation` (Phase 7 cooperative import cancellation)
6. `test_batch_import_large_collection_durability_and_flush` (Phase 7 large collection test)
7. `test_virtual_grid_row_construction_and_partially_filled_rows` (Phase 8 virtual grid row layout)
8. `test_virtual_grid_filtering_and_sorting_invalidation` (Phase 8 virtual grid cache invalidation)
9. `test_virtual_grid_group_expansion_and_collapse` (Phase 8 group accordion mechanics)
10. `test_virtual_grid_list_mode_row_partitioning` (Phase 8 list mode partitioning)

Additionally, 9 opt-in benchmarks (`perf_end_to_end_page_pipeline`, `perf_cache_hit_vs_miss_transition`, `perf_real_world_presentation_and_navigation_breakdown`, `perf_stage3_*`, `perf_stage4_*`) were missing.

### C. Resolution & Verification
All 10 regression tests and 9 benchmarks were recovered from transcript logs, updated to current domain API signatures (e.g. `layout::display_size`), and re-integrated into `src/tests.rs`.
Executing `cargo test --all-targets` verified that all 342 active tests passed cleanly before beginning Stage 6A engineering.

---

## 3. M1 — Reader Lifecycle & Texture Cleanup Analysis & Remediation

### A. Technical Investigation of the Defect
In Yomu's architecture, when a user is reading a comic and selects another comic (either via library navigation, recent reading list, or direct file open), `Yomu::open_book()` was previously executing:
```rust
// Legacy code in open_book():
self.reader = Some(Reader::new(&book, &settings));
```
This direct assignment immediately overwrote `self.reader`. This created two critical resource management flaws:
1. **Uncancelled Background Decode Tasks:** Any in-flight background worker threads decoding pages from the previous archive continued executing in `cx.background_executor()`. When completed, their completion callbacks (`page_decoded`) were dispatched to the UI thread. Although `page_decoded` checks `reader.book_id == book_id`, the CPU and I/O cycles of decoding high-resolution pages were wasted, and memory allocated for the aborted pages remained alive until the thread finished.
2. **GPU Texture Atlas Leak:** In GPUI, a `RenderImage` on the CPU only allocates a GPU sprite atlas tile when painted. Dropping a `RenderImage` on the Rust CPU heap does *not* notify GPUI's platform atlas to release the GPU texture tile unless `window.drop_image(data)` is explicitly invoked. Because `reader.into_images()` was never called on the old reader during direct switching, previously painted pages remained pinned in the GPU atlas indefinitely until application exit.

### B. Implemented Remediation
1. **Centralized Reader Teardown:** Implemented `cleanup_current_reader(&mut self)` in `src/app/reader_controller.rs`:
   ```rust
   pub(crate) fn cleanup_current_reader(&mut self) {
       if let Some(reader) = self.reader.take() {
           if let Some(ref cancel) = reader.probe_cancel {
               cancel.store(true, Ordering::Relaxed);
           }
           for token in reader.cancellations.values() {
               token.store(true, Ordering::Relaxed);
           }
           self.garbage.extend(reader.into_images());
       }
   }
   ```
2. **Immediate Texture Atlas Draining:** Added `drain_garbage(&mut self, window: &mut Window)` which loops through `self.garbage.drain(..)` and invokes `window.drop_image(image)`, freeing atlas tiles immediately.
3. **Wired into All Lifecycle Transitions:**
   - `open_book(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>)`: calls `self.cleanup_current_reader(); self.drain_garbage(window);` prior to constructing the new `Reader`.
   - `close_book(&mut self, window: &mut Window, cx: &mut Context<Self>)`: calls `self.cleanup_current_reader(); self.drain_garbage(window);`.
   - `clear_all_data(&mut self, cx: &mut Context<Self>)` in `src/main.rs`: replaces `self.reader = None;` with `self.cleanup_current_reader();`.
   - `request_quit(&mut self, cx: &mut Context<Self>)` in `src/app/persistence.rs`: calls `self.cleanup_current_reader();` ensuring all background decoders abort promptly on exit.
4. **Race Condition Safety:** If an in-flight background task completes after `cleanup_current_reader()`, `page_decoded()` compares the task's `book_id` against `self.reader.book_id`. Because the IDs mismatch, the incoming image is dropped immediately without polluting `reader.images`, `reader.pending`, or `reader.failed`.

---

## 4. M2 — GPUI Cover Asset Retention Analysis & Bounded Eviction Implementation

### A. Investigation of GPUI Asset Architecture
An inspection of `gpui-pre = 0.3.10` source code revealed how `img(cover_path)` works:
1. In `src/elements/img.rs`, the element checks `window.image_cache_stack.last()`. If an image cache provider is active on the window, it delegates loading to `cache.load(&resource, window, cx)`.
2. If no image cache is active, it falls back to `window.use_asset::<ImgResourceLoader>(&resource, cx)`.
3. GPUI's built-in `RetainAllImageCache` stores every loaded image in an unbounded `HashMap<u64, ImageCacheItem>` without an eviction policy.
4. `ImageCacheItem` owns a `CachedLoad<ImgResourceLoader>`, which holds both the CPU-side decoded pixel buffer (`Arc<RenderImage>`) and triggers GPU texture atlas upload when painted.
5. In a 10,000-book library, scrolling through the entire collection using the default loader would retain thousands of decoded cover textures in memory without upper bounds.

### B. Implementation of `BoundedCoverCache`
To solve this cleanly within official GPUI Kit conventions without custom webviews, hacks, or fork dependencies:
1. Created `src/ui/cover_cache.rs` implementing `BoundedCoverCache` using standard library collections:
   - `items: HashMap<u64, ImageCacheItem>` for $O(1)$ item lookup.
   - `order: VecDeque<u64>` for $O(1)$ LRU order tracking.
   - `capacity: usize` configured to 256 covers (clamped to a minimum of 16).
2. **LRU Eviction Semantics:**
   - On cache hit: The accessed key is moved to the back of `order` (most recently used).
   - On cache miss: If `items.len() >= capacity`, the least recently used key is popped from `order.pop_front()`. The corresponding `ImageCacheItem` is removed from `items`. If an image had already finished loading, `cx.drop_image(image, Some(window))` is invoked to reclaim GPU sprite atlas space, and the CPU buffer is dropped.
   - When the entity is released (e.g. on shutdown), `cx.observe_release` drains all remaining items and drops their GPU textures.
3. **Application Root Shell Integration:**
   - Added `cover_cache: Entity<BoundedCoverCache>` to `struct Yomu` in `src/app/state.rs`.
   - In `src/app/views/shell.rs`, wrapped `render_two_pane_shell` with:
     ```rust
     gpui_kit::image_cache(self.cover_cache.clone())
         .size_full()
         .flex()
         .flex_col()
         .child(...)
     ```
   - Because `image_cache()` pushes the cache provider onto the window's image cache stack during layout and prepaint, all descendant `img(cover_path)` calls in `LibraryCard`, `ContinueReadingCard`, and `DetailsView` automatically route through `BoundedCoverCache` with zero modifications to view components.

---

## 5. Empirical Profiling & Verification Results

### A. Direct Book Switch (Scenario B)
To verify M1 remediation, the 50 direct book switches in Scenario B were tested:
- **Before Remediation:** Rapidly switching across 50 books left uncancelled background decode tasks running in the executor. Decoded pages from abandoned books remained in the GPU texture atlas until reader exit, elevating peak RSS to 68.34 MB and causing CPU churn.
- **After Remediation:** On every `open_book()`, `cleanup_current_reader()` immediately cancels all outstanding tasks and moves old page textures to `garbage`, where `window.drop_image()` frees them. Memory footprint remains stable, and executor worker threads terminate promptly upon cancellation.

### B. Cover Cache Eviction (Scenario D)
To verify M2 remediation:
- **Before Remediation:** In an unbounded asset loader, scrolling 10,000 covers would allocate approximately $\approx 5.4$ GB of decoded BGRA8 cover textures.
- **After Remediation:** `BoundedCoverCache` enforces a strict 256-cover limit ($\approx 138$ MB at 300x450 BGRA8). Once 256 covers are reached, each newly displayed card evicts the oldest off-screen card from memory, dropping both CPU pixel buffers and freeing GPU atlas tiles.

---

## 6. Verification Matrix & Test Inventory

The complete validation suite was executed inside the NixOS / devenv environment:

```bash
# 1. Format check
devenv shell cargo fmt --all -- --check
# Result: Exit 0 (clean formatting)

# 2. Type check
devenv shell cargo check --all-targets
# Result: Exit 0 (clean compilation)

# 3. Clippy lint check with warnings as errors
devenv shell cargo clippy --all-targets --all-features -- -D warnings
# Result: Exit 0 (zero warnings)

# 4. Full test suite across all targets
devenv shell cargo test --all-targets
# Result: Exit 0 (346 passed, 0 failed, 13 ignored benchmarks)
```

### Dedicated Regression Tests Added
1. `test_direct_book_switch_cleans_up_reader_and_cancels_tasks` (`src/tests.rs:7700`):
   - Verifies that `open_book()` from Book A to Book B cancels Book A's in-flight decode tokens, drains old images to garbage, and flushes them via `window.drop_image()`.
2. `test_direct_book_switch_in_flight_completion_does_not_pollute_new_reader` (`src/tests.rs:7750`):
   - Verifies that if a stale background decode task for Book A completes after Book B is opened, the late callback is safely discarded and Book B's reader state remains unpolluted.
3. `test_reader_close_and_clear_all_data_cleans_up_resources` (`src/tests.rs:7805`):
   - Verifies that `clear_all_data()` cancels in-flight reader tasks, tears down the reader, and empties the cover cache.
4. `test_bounded_cover_cache_capacity_and_lru_eviction` (`src/tests.rs:7852`):
   - Verifies that `BoundedCoverCache` enforces its maximum capacity constraint (256/16), correctly tracks item count on insertion, and cleanly releases resources on `clear_without_window()`.

---

## 7. Modified Files & Responsibilities

| File Path | Nature of Change | Responsibilities Changed |
|---|---|---|
| `src/ui/cover_cache.rs` | **New file** | Implemented `BoundedCoverCache` (LRU eviction, capacity bounding, GPU texture freeing via `cx.drop_image`). |
| `src/ui/mod.rs` | Modified | Exported `pub mod cover_cache;`. |
| `src/app/state.rs` | Modified | Added `cover_cache: Entity<BoundedCoverCache>` field to `struct Yomu` and initialized it in `Yomu::new`. |
| `src/app/reader_controller.rs` | Modified | Added `cleanup_current_reader()` and `drain_garbage()`; updated `open_book()` and `close_book()` to clean up prior readers. |
| `src/app/persistence.rs` | Modified | Added `self.cleanup_current_reader();` to `request_quit()` for prompt background task cancellation on shutdown. |
| `src/app/views/shell.rs` | Modified | Wrapped `render_two_pane_shell` with `gpui_kit::image_cache(self.cover_cache.clone())`. |
| `src/main.rs` | Modified | Updated `clear_all_data()` to call `self.cleanup_current_reader()` and clear `self.cover_cache`. |
| `src/tests.rs` | Modified | Restored 10 truncated tests and 9 benchmarks from Stage 5; added 4 new Stage 6A regression tests (total 346 active tests). |
| `AGENTS.md` / `GEMINI.md` | Modified | Updated active test baseline count from 342 to 346. |
| `docs/agent-handoff.md` | Modified | Updated test baseline and logged Stage 6A completion. |

---

## 8. Architectural Invariants & Residual Technical Debt

1. **GPU Texture Atlas Mechanics:** In GPUI Kit / Metal / Vulkan / Direct3D backends, calling `window.drop_image()` removes the sub-texture allocation from the GPU sprite atlas. This frees atlas tile space for reuse by other pages or covers, preventing the atlas from growing without bound.
2. **Wayland Display Reporting:** As required by Section 5 of `AGENTS.md`, headless simulated test execution validates state, layout, and event dispatch. Visual appearance on physical Wayland compositors remains **UNVERIFIED** via automated tests.
3. **Next Recommended Tasks (Stage 6B):**
   - **Autosave SSD Write Amplification (Finding S1):** Implement dirty checking on page turn events to avoid unnecessary saves when progress does not change, and evaluate a lightweight progress sidecar file for reading state to eliminate whole-library JSON serialization on every page flip.
   - **CBR/RAR Header Seek Optimization (Finding S2):** Cache entry byte offsets during the initial dimension probe pass to enable $O(1)$ page seeks instead of linear $O(N)$ header re-scans in RAR archives.
