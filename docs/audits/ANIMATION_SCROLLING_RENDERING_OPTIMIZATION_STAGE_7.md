# Stage 7 — Animation, Scrolling & Rendering Performance Optimization Report

**Status:** Completed & Validated  
**Date:** November 2026  
**Target Environment:** Linux (Wayland / X11), macOS, Windows; safe Rust 2024 (`#![forbid(unsafe_code)]`) on GPUI Kit 0.8.1 (`gpui-pre = 0.3.10`).  
**Test Baseline:** **366 active tests** passing (154 lib unit tests, 212 binary/UI tests; 18 ignored benchmarks; 384 total).

---

## 1. Executive Summary

Stage 7 conducted an exhaustive, empirical performance investigation across Yomu's animations, scrolling, rendering pipelines, and general UI responsiveness. Prior stages addressed algorithmic bottlenecks in image scaling, archive concurrency, library scalability, memory leaks, persistence write amplification, and RAR sequential scanning. Stage 7 focused on the high-frequency UI thread render loop and compositor frame pacing:

1. **Library Virtual Grid Scroll Invalidation Bypass:**
   - *Problem:* In `render_all_manga_view`, `self.get_sorted_books()` was invoked and `matches_query` filtering was executed over all books on *every single animation/scroll frame*, allocating a `Vec<&Book>` before checking `library_row_cache.is_valid()`. During fast scrolling or inertial glide, the cache was valid, meaning the full sorting and query pass was completely redundant and immediately discarded. For a 10,000-book library, this imposed ~185 ms of CPU overhead per frame if invalidated.
   - *Optimization:* Re-ordered rendering pipeline to validate `LibraryRowCache` *prior* to book filtering. When `is_cached` is true, the layout directly renders from cached rows in **0.010 ms** (18,146x faster than recomputing).
2. **Library Element ID Formatting Elimination:**
   - *Problem:* `render_book_card`, `render_book_list_row`, `render_folder_card`, and `render_folder_list_row` constructed element IDs via runtime `format!("manga-item-{}", id)`, `format!("manga-cover-{}", id)`, and `format!("folder-{}", key)`. Across 60 visible cards at 144 Hz, this generated over 30,000 transient string allocations per second.
   - *Optimization:* Adopted native composite tuple IDs `(ElementId::from("manga-item"), book.id.as_str())`, `(ElementId::from("manga-cover"), book.id.as_str())`, and `(ElementId::from("folder"), group_key)`, completely eliminating per-frame string heap allocations.
3. **Reader Frame Render Loop Allocation Elimination (`sync_reader`):**
   - *Problem:* `sync_reader` allocated a heap `Vec<usize>` (`wanted_indices`) and a heap `Vec<(usize, u32)>` (`wanted`) on every render tick (up to 144 times/sec).
   - *Optimization:* Equipped `Reader` with `wanted_scratch: Vec<(usize, u32)>` for in-place buffer reuse, coupled with stack array chunking (`[(usize, u32); 32]`) for dispatching `request_page`. Zero heap allocations per frame.
4. **Continuous Reader Layout Cache In-Place Buffer Reuse & $O(1)$ Settle:**
   - *Problem:* On viewport width or zoom adjustments, `ensure_continuous_cache` deallocated and reallocated three separate vectors (`tops`, `heights`, `bottoms`). Additionally, `settle` executed an $O(N)$ linear loop re-summing page heights on arrivals.
   - *Optimization:* `ensure_continuous_cache` now retains vector capacity across resize/zoom via `.clear()` and `.reserve()`. `settle` extracts arrival geometry directly from cached `tops[page]` and `heights[page]` in $O(1)$ time (<50 ns).
5. **Page Counter String Caching (`CachedPageCounter`):**
   - *Problem:* `current_page_str()` formatted a new string (e.g. `"1 / 50"`) on every frame render, even when scrolling or panning within the same page.
   - *Optimization:* Added `CachedPageCounter` on `Reader` storing `(primary, secondary, total, formatted)`. Consecutive calls on the same page return the cached `SharedString` via fast stack clone in **3.9 ns** with 0 heap allocations.
6. **Main Render Loop Appearance & Texture Drop Guards:**
   - *Problem:* `self.sync_appearance()` queried `window.appearance()` on every frame even when manual theme mode was selected and opaque appearance was already applied. `self.garbage.drain(..)` was called unconditionally twice per frame.
   - *Optimization:* Added early-return guards skipping `window.appearance()` when theme mode is not `System` and appearance is already opaque. Guarded texture drop passes with `!self.garbage.is_empty()`.

---

## 2. Empirical Benchmark Verification

The performance test suite (`perf_stage7_animation_scrolling_rendering_breakdown`) was executed under `devenv shell` on release/bench profile:

### A. Library Grid Scrolling (144 Hz Frame Budget: 6.94 ms)

| Library Size | Recomputed Invalidation Frame | Cached Scroll Frame (144 Hz Glide) | Speedup Factor | Headroom vs 144 Hz Budget |
|:---|:---|:---|:---|:---|
| **1,000 Books** | 2.063 ms | **0.012 ms** | **166.2x** | 559.1x headroom |
| **5,000 Books** | 40.353 ms (frame drop) | **0.010 ms** | **3,956.6x** | 680.5x headroom |
| **10,000 Books** | 185.563 ms (severe stutter) | **0.010 ms** | **18,146.2x** | 678.7x headroom |

*Conclusion:* In-library scrolling stays flat at **~0.010 ms** regardless of collection size, consuming <0.15% of the 144 Hz frame budget.

### B. Continuous Reader Layout & Settle Latency

| Document Size | Settle Offset & Visible Page Partition | Page Counter String (Cached) | Total Per-Frame Math | Frame Budget Consumed |
|:---|:---|:---|:---|:---|
| **1,000 Pages** | 48.0 ns (0.048 µs) | 4.0 ns | **0.052 µs** | <0.001% of 144 Hz |
| **5,000 Pages** | 45.0 ns (0.045 µs) | 3.9 ns | **0.049 µs** | <0.001% of 144 Hz |
| **10,000 Pages** | 50.1 ns (0.050 µs) | 3.9 ns | **0.054 µs** | <0.001% of 144 Hz |

*Conclusion:* Continuous reader scrolling and zooming execute in sub-microsecond time with zero heap allocations on steady-state frames.

---

## 3. Detailed Technical Modifications

### `src/ui/reader.rs`
- **`CachedPageCounter`:** Added a structured cache struct storing `primary`, `secondary`, `total`, and `formatted: SharedString`.
- **`current_page_str`:** Reads `page_str_cache`. If `primary`, `secondary`, and `total` match, returns `cached.formatted.clone()` without string formatting.
- **`wanted_scratch`:** Added a preallocated `Vec<(usize, u32)>` field to `Reader` for zero-allocation prefetch calculations.
- **`ensure_continuous_cache`:** Updated cache regeneration to reuse preallocated `tops`, `heights`, and `bottoms` vectors in-place instead of re-instantiating new vectors.
- **`settle`:** Replaced naive linear loop with $O(1)$ lookups into `cache.tops[page]` and `cache.heights[page]`. Scoped `ensure_continuous_cache` borrows cleanly around offset mutations.

### `src/app/views/library.rs`
- **`render_empty_library_state`:** Factored out unified empty state element construction.
- **`render_all_manga_view`:** Reordered cache validation to precede book list sorting and search filtering. When `cache.is_valid(...)` is true, row layout generation is skipped entirely.
- **Composite Element IDs:** Replaced string format allocations in `render_book_card`, `render_book_list_row`, `render_folder_card`, and `render_folder_list_row` with tuple `ElementId` representations (`(ElementId::from("manga-item"), book.id.as_str())`).

### `src/app/reader_controller.rs`
- **`sync_reader`:** Populates `reader.wanted_scratch` in-place, eliminating temporary `wanted_indices` and `wanted` vector allocations. Dispatches page prefetch requests via stack-allocated array chunks (`[(usize, u32); 32]`).
- **`ReaderSyncOutcome`:** Streamlined to return only `animating` and `progress_changed`.

### `src/main.rs`
- **`sync_appearance`:** Added early return guard when theme mode is not `System` and opaque appearance has already been applied, avoiding redundant `window.appearance()` calls per frame.
- **Garbage Texture Draining:** Wrapped `self.garbage.drain(..)` with `!self.garbage.is_empty()` checks.

### `src/app/views/shell.rs`
- **`EffectTransition` ID:** Replaced `format!("main-view-fade-{}", view_key)` with native composite tuple `(ElementId::from("main-view-fade"), view_key)`.

### `src/tests.rs`
- Added unit tests:
  - `test_stage7_reader_page_counter_string_caching`
  - `test_stage7_continuous_layout_in_place_buffer_reuse`
  - `test_stage7_library_scroll_render_tree_cache_hit_bypass`
- Added performance benchmark:
  - `perf_stage7_animation_scrolling_rendering_breakdown`

---

## 4. Full Validation Matrix

All validation commands executed in the devenv shell:

1. **Format Check:**
   ```bash
   devenv shell cargo fmt --all -- --check
   ```
   *Exit code:* 0 (clean).
2. **Type Check Across All Targets:**
   ```bash
   devenv shell cargo check --all-targets
   ```
   *Exit code:* 0 (clean).
3. **Strict Clippy Lint Check:**
   ```bash
   devenv shell cargo clippy --all-targets --all-features -- -D warnings
   ```
   *Exit code:* 0 (clean, 0 warnings).
4. **Full Test Suite Across All Targets:**
   ```bash
   devenv shell cargo test --all-targets
   ```
   *Exit code:* 0.
   - Unit tests (`src/lib.rs`): **154 passed**, 0 failed, 2 ignored.
   - Main / UI tests (`src/main.rs`): **212 passed**, 0 failed, 16 ignored.
   - Total active tests: **366 active tests** (384 total including benchmarks).
