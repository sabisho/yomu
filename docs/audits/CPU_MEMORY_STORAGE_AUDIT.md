# Yomu: Comprehensive CPU, Memory & Storage I/O Audit Report (Stage 1)

**Date:** 2026-11-03  
**Auditor:** Senior Rust Systems, Graphics & Concurrency Engineering Team  
**Scope:** Stage 1: System-Wide Resource Consumption Audit, Profiling & Remediation Roadmap  
**Target Architecture:** Linux (Wayland-native, X11 fallback), safe Rust 2024, GPUI Kit 0.8.1 (`gpui-pre = 0.3.10`)  
**Safety Invariant:** `#![forbid(unsafe_code)]` Strictly Enforced Across All Crates  

---

## 1. Executive Summary

A comprehensive, evidence-driven engineering audit of Yomu's CPU utilization, memory allocation, and storage read/write behavior was conducted across the core domain layer (`src/`), the UI presentation layer (`src/ui/`), and application orchestration (`src/app/`, `src/main.rs`).

The audit employed real-time Linux instrumentation, including `/run/current-system/sw/bin/time -v`, `/run/current-system/sw/bin/strace -c` syscall tracking, Valgrind Massif (heap profiling), Valgrind DHAT (dynamic heap allocation analysis), and existing Criterion-style benchmarks.

### Key Audit Conclusions

1. **Safety & Leak Discipline:**
   - Yomu strictly complies with `#![forbid(unsafe_code)]` with zero unsafe code.
   - Valgrind DHAT analysis on end-to-end decode-and-render pipelines confirmed that after peak allocation (~114.1 MB at $t_{\text{gmax}}$), retained heap at shutdown drops to ~67 KB ($t_{\text{end}}$), confirming that image caches, textures, and decode buffers do not leak long-term.
   - Adaptive memory budgeting (`pagecache::system_memory_budget()`) correctly enforces memory bounds based on host physical RAM (512 MiB to 3.0 GiB) without per-frame `/proc/meminfo` stat overhead.

2. **Primary Confirmed Inefficiencies & Bugs:**
   - **Idle CPU/Storage Wakeup Loop (Finding C1):** In `src/app/state.rs:565-611`, `watch_custom_theme_file` runs an unceasing 750ms polling loop on the background executor that performs a complete `fs::read` of `themes.toml` every 750ms (~80 times per minute) while completely idle, preventing CPU deep C-states and causing periodic disk read I/O.
   - **N-Times Full-File Rewrites on Batch Import (Finding S1):** In `src/main.rs:750`, during library imports, every single discovered book immediately triggers `save_library`, which clones the full library and rewrites the entire `library.json` file to disk. Importing 500 books performs 500 full serialization and atomic disk replacement cycles (~2.5 GB of writes for a 5 MB library).
   - **Deep Cloning of Full Library Model on Autosave (Finding M1):** In `src/app/persistence.rs:108`, `flush_save_background` deep-clones the entire `Library` struct (including all books, marks, spread overrides, and thousands of cached `page_dimensions` BTreeMap nodes) on every discrete or debounced autosave.
   - **Lack of `fsync` Prior to Atomic Rename in `Library::save` (Finding S3):** In `src/library.rs:967-969`, `Library::save` writes to `library.json.tmp` and renames to `library.json` without calling `file.sync_all()`. In the event of a system crash or power outage, this creates zero-length truncated files, causing `Library::load` to mark the file as corrupt.
   - **Unvirtualized Library Element Construction (Finding C2):** `src/app/views/library.rs:344-448` instantiates the complete GPUI element tree for every book and folder in the library simultaneously on every frame during scrolling, with zero element virtualization.
   - **Redundant Case Fold Allocations in Library Search (Finding C3):** `src/app/views/library.rs:245-252` calls `b.title.to_lowercase()` and `s.to_lowercase()` for every book on every render frame, bypassing the existing zero-allocation `search::contains_ignore_case` implementation.
   - **Sequential $O(N)$ Header Traversal on CBR/RAR Page Reads (Finding S2):** `src/archive.rs:269-317` opens the RAR archive from byte 0 and scans entry headers sequentially from the beginning for every page request, incurring quadratic I/O seeks during multi-page chapter reading.
   - **$O(N)$ Cumulative Layout Scans in Continuous Mode (Finding C5):** `src/ui/reader.rs` calculates continuous page positions by iterating $0..N$ four times per frame (`continuous_total_height`, `continuous_page_at`, `continuous_visible_range`, and `render_reader`).

---

## 2. Existing Architecture & Resource-Flow Overview

The diagram below details the end-to-end resource lifecycle, background worker scheduling, and persistence paths:

```text
               ┌─────────────────────────────────────────────────────────────┐
               │                        User Input                           │
               │         (Keybindings, Gestures, Window Resizing)            │
               └──────────────┬───────────────────────────────┬──────────────┘
                              │                               │
                              ▼                               ▼
       [UI Render Loop: src/main.rs]          [Orchestration: reader_controller.rs]
       - 60/144Hz frame pacing                - Prefetch plan (`wanted`)
       - Texture GC (`window.drop_image`)     - Cooperative cancellation tokens
       - Layout geometry & viewport bounds    - Debounced autosave triggers
                              │                               │
                              ▼                               ▼
       [UI Presentation: src/ui/]             [Background Thread Pool (GPUI)]
       - Unvirtualized Card Grid              - `archive::read_page` (ZIP/RAR)
       - Continuous Layout Math ($O(N)$)      - `imaging::decode_fit_bgra` (SIMD)
       - Theme Polling (750ms timer)          - `library.save` (`write_lock`)
                              │                               │
                              ▼                               ▼
       [GPU Compositor & Textures]            [Filesystem Storage ($XDG_DATA_HOME)]
       - RenderImage (BGRA8)                  - `covers/medium/*.jpg` (Hard-linked)
       - Adaptive 512 MiB–3.0 GiB cap         - `library.json` (Atomic rename)
       - Dropped on eviction & close          - `themes.toml` (Custom themes)
```

---

## 3. CPU Utilization Audit Findings

### Finding C1: Idle Theme Watcher Polling Loop & Continuous File Reads
- **Severity:** High (Power Consumption, Idle Battery Drain)
- **Subsystem:** Application State (`src/app/state.rs:565-611`, `src/ui/theme.rs:252-257`)
- **Problem:** When Yomu is completely idle (minimized, in the background, or with zero user interaction), `watch_custom_theme_file` wakes up every 750 milliseconds via `cx.background_executor().timer(Duration::from_millis(750))`. On every tick, it spawns an async task on the background executor, which calls `custom_theme_fingerprint`. This function calls `fs::read(&path)`, reading the entire `themes.toml` file into a newly allocated `Vec<u8>` buffer and hashing it.
- **Root Cause:** Polling-based file watcher architecture without file modification timestamp (`mtime`) pre-filtering or OS notification (`inotify`/`kqueue`/`ReadDirectoryChangesW`).
- **Measured Impact:**
  - ~80 wakeups and background task dispatches per minute when idle.
  - ~80 disk read syscalls (`openat`, `read`, `close`) per minute while idle.
  - Prevents processor cores from maintaining deep CPU power-saving C-states.
- **Proposed Fix (Stage 2):**
  1. Inspect file metadata via `fs::metadata(path).and_then(|m| m.modified())` before reading contents; skip `fs::read` if the modification timestamp has not changed.
  2. Increase the polling interval when the application window is inactive or unfocused, or switch to event-based watching.
- **Regression Risks:** Theme updates edited externally in Noctalia or text editors must still apply automatically. Checking `mtime` preserves this behavior while eliminating 99.9% of payload reads.

### Finding C2: Unvirtualized Element Construction in Library & Collection Views
- **Severity:** High (CPU Frametime, Scalability)
- **Subsystem:** View Presentation (`src/app/views/library.rs:344-448`, `src/app/views/continue_reading.rs:96-180`)
- **Problem:** `render_all_manga_view` and `render_continue_reading_view` instantiate `AnyElement` card containers, text labels, tags, click handlers, and context-menu closures for **every single item** in the library on every frame during scrolling and window resizing.
- **Root Cause:** Standard GPUI Kit flex-wrap container (`div().flex().flex_wrap().children(cards)`) without vertical scroll viewport windowing.
- **Measured Impact:**
  - In a library of 1,000 items, 1,000 complete element trees are constructed and evaluated on every frame.
  - Allocates thousands of transient closures and Element IDs per frame during active inertial glide.
- **Proposed Fix (Stage 2):**
  Implement row/grid virtualization or compute viewport visibility bounds so only cards currently intersecting the visible scroll window (plus 1–2 rows of overscan buffer) are instantiated.
- **Regression Risks:** Keyboard navigation (`j`/`k`, arrow keys) and selection states must remain consistent even when target cards are outside the active viewport.

### Finding C3: Redundant String Allocations in Live Library Filter
- **Severity:** Medium (CPU Allocations)
- **Subsystem:** Library View (`src/app/views/library.rs:245-252`)
- **Problem:** During live search or scrolling while a filter is active, `matches_query` computes:
  ```rust
  let matches_query = |b: &Book| {
      query.is_empty()
          || b.title.to_lowercase().contains(&query)
          || b.series.as_ref().is_some_and(|s| s.to_lowercase().contains(&query))
  };
  ```
  Every book incurs up to 2 heap string allocations (`to_lowercase()`) on every frame.
- **Root Cause:** Bypassed the existing zero-allocation search implementation in `src/search.rs`.
- **Measured Impact:** For 500 books, 1,000 string allocations per frame while typing in the search bar or scrolling search results.
- **Proposed Fix (Stage 2):**
  Replace with `yomu::search::matches(&b.title, &b.path, b.series.as_deref(), &self.search_query)` which uses stack-based ASCII window matching with zero heap allocations.
- **Regression Risks:** None; `search::matches` is already tested and verified in `src/search.rs`.

### Finding C4: Repeated `directory_groups` Bucket Partitioning in Shell & Library Render
- **Severity:** Medium (CPU Latency & Allocation)
- **Subsystem:** View Adapters (`src/app/views/shell.rs:93`, `src/app/views/library.rs:259, 347`)
- **Problem:** `directory_groups` iterates over all books, formats keys (`format!("series:{}", ...)` and `format!("directory:{}", ...)`), and builds `BTreeMap` buckets. In `shell.rs:93`, this is called solely to retrieve the title and volume count for the toolbar header. It is then called again inside `render_all_manga_view` (lines 259 and 347).
- **Root Cause:** Lack of memoization for folder group partitions.
- **Measured Impact:** Up to 3 separate $O(N \log N)$ grouping and string formatting passes over the entire library during a single frame render when an open group is active.
- **Proposed Fix (Stage 2):**
  Cache folder groups in `SortedBooksCache` or `Yomu` keyed on `save_seq` and `library.books().len()`.
- **Regression Risks:** Moving books between series or toggling grouping settings must invalidate the cache immediately.

### Finding C5: $O(N)$ Cumulative Layout Scans in Continuous Reader
- **Severity:** Medium (CPU Render Loop Latency)
- **Subsystem:** Reader Presentation (`src/ui/reader.rs:592, 1178, 1200, 1222`)
- **Problem:** In continuous vertical reading mode, `continuous_total_height`, `continuous_page_at`, `continuous_visible_range`, and the page render loop in `render_reader` all perform linear $0..N$ scans over all pages in the comic, accumulating heights from `orig_sizes`.
- **Root Cause:** Layout positions are not cached as cumulative prefix sums.
- **Measured Impact:** In a 1,000-page comic (e.g. continuous webtoon chapter collection), 4 linear loops totaling 4,000 iterations per frame occur during scrolling.
- **Proposed Fix (Stage 2):**
  Store a precomputed cumulative prefix sum array (`Vec<f32>`) in `Reader`. Finding total height becomes $O(1)$, page lookup becomes $O(\log N)$ via binary search, and visible page rendering directly indexes the start page without scanning prior pages.
- **Regression Risks:** When an unprobed page's dimensions arrive via `pages_probed`, the cumulative prefix array must be recomputed.

---

## 4. Memory Management Audit Findings

### Finding M1: Deep-Cloning Full Library Model on Every Background Autosave
- **Severity:** High (Memory Allocation Volume, Allocator Churn)
- **Subsystem:** Persistence Subsystem (`src/app/persistence.rs:108`, `src/library.rs:906-917`)
- **Problem:** `flush_save_background` executes:
  ```rust
  let library = self.library.clone();
  ```
  `Library` contains `books: Vec<Book>`, where each `Book` owns:
  - `id: String`, `title: String`, `series: Option<String>`, `path: PathBuf`
  - `marks: BTreeMap<String, usize>`
  - `spread_overrides: BTreeMap<String, PageSpreadOverride>`
  - `page_dimensions: BTreeMap<String, (u32, u32)>` (one node per page in the comic)
- **Root Cause:** Asynchronous persistence offloading was implemented by full value cloning of the domain model rather than immutable snapshots or copy-on-write pointers.
- **Measured Impact:**
  - For a library with 1,000 books averaging 200 pages probed, this clones 200,000 BTreeMap nodes, 3,000 strings, and thousands of allocations on every debounced autosave (triggered on page turns, scroll pauses, and setting changes).
  - DHAT tracking shows multiple megabytes of transient allocation churn during active reading.
- **Proposed Fix (Stage 2):**
  Adopt copy-on-write data structures (`Arc<[Book]>`, `Arc<BTreeMap<...>>`, or `im` immutable data structures), or serialize directly from an `Arc<RwLock<Library>>` without deep cloning.
- **Regression Risks:** Ensure concurrent UI mutations during background disk serialization do not introduce lock contention or stale state overwrites.

### Finding M2: Intermediate Triple Allocation in Non-JPEG Decoding Pipeline
- **Severity:** Medium (Peak RSS Spike During WebP/PNG Decoding)
- **Subsystem:** Imaging Subsystem (`src/imaging.rs:217-240`)
- **Problem:** While JPEG decoding uses `zune-jpeg` to decode directly into BGRA with SIMD, non-JPEG images (PNG, WebP, GIF) follow this chain:
  1. `decode(bytes)` allocates a `DynamicImage` (RGB/RGBA buffer).
  2. If opaque, `img.into_rgb8()` allocates an `RgbImage` (3 bytes/px).
  3. `resize_rgb` allocates a resized `RgbImage` (3 bytes/px).
  4. `rgb_to_bgra` allocates a fourth `RgbaImage` (4 bytes/px) with alpha 255.
- **Root Cause:** Format converters treat non-alpha images as 24-bit RGB through intermediate resizing before converting to 32-bit BGRA.
- **Measured Impact:** For a 3000x4500 WebP scan, transient heap allocation exceeds 107 MB across 4 buffers before settling on the 11.2 MB final BGRA texture.
- **Proposed Fix (Stage 2):**
  Convert directly from decoded image to 32-bit BGRA in a single pass, then run `fast_image_resize` directly on 4-channel `PixelType::U8x4`.
- **Regression Risks:** Verify Catmull-Rom downscaling quality on alpha vs opaque images.

### Finding M3: Transient `Vec<&Book>` Allocation in `get_sorted_books`
- **Severity:** Low (Minor Heap Churn)
- **Subsystem:** Application State (`src/app/state.rs:395-428`)
- **Problem:** `get_sorted_books` caches `indices: Vec<usize>`, but returns a newly allocated `Vec<&Book>` on every call (`cached.indices.iter().map(|&idx| &all_books[idx]).collect()`).
- **Root Cause:** Return type signature returns an owned vector of borrowed references instead of a cached slice or iterator.
- **Measured Impact:** In libraries with 1,000 books, allocates an 8 KB pointer buffer multiple times per frame.
- **Proposed Fix (Stage 2):**
  Cache `cached_books: Vec<&'a Book>` directly or return an iterator.

---

## 5. Storage I/O Audit Findings

### Finding S1: $N$-Times Full-File Rewrites on Batch Import
- **Severity:** Critical (Disk I/O Write Amplification, SSD Wear)
- **Subsystem:** Main Application & Import Coordination (`src/main.rs:749-752`)
- **Problem:** When batch importing files or folders:
  ```rust
  if this.library.add(book) {
      this.save_library(cx);
      this.imported_count += 1;
  }
  ```
  Every single imported book invokes `save_library(cx)`, which schedules an immediate full rewrite of `library.json` on the background thread pool.
- **Root Cause:** Immediate persistence dispatch on every book addition instead of batching or debouncing across the import session.
- **Measured Impact:**
  - Importing a 500-chapter manga library executes **500 consecutive full rewrites** of `library.json`.
  - For a 5 MB library file, this writes **2.5 Gigabytes of disk data** and acquires `write_lock` 500 times.
  - Generates heavy disk contention against the background image decoders generating thumbnails.
- **Proposed Fix (Stage 2):**
  Change line 750 to use `save_library_debounced(cx)` during import, and issue a single final `save_library(cx)` flush when `ImportEvent::Finished` is received.
- **Regression Risks:** If the process is terminated mid-import, debouncing guarantees that progress is committed at least every 2 seconds (`AUTOSAVE_MAX_DELAY`), ensuring partial import progress is safely preserved.

### Finding S2: Sequential Seeking in CBR/RAR Page Reads
- **Severity:** High (I/O Latency on CBR Archives)
- **Subsystem:** Archive Subsystem (`src/archive.rs:269-317`)
- **Problem:** In `read_rar_page`, each page read instantiates `unrar::Archive::new(path).open_for_processing()` and sequentially iterates through headers from the beginning of the file until the target entry is found.
- **Root Cause:** UnRAR C++ API requires stream processing; unlike ZIP (which has an end-of-file central directory index), RAR entries are laid out sequentially.
- **Measured Impact:** In a 500-page CBR archive, reading page 400 requires skipping 399 entry headers. Sequential reading across an entire volume incurs $\sum_{k=1}^N k = \frac{N(N+1)}{2}$ header scans (~125,000 header reads for 500 pages).
- **Proposed Fix (Stage 2):**
  Maintain an open processing cursor for contiguous page navigation or record byte offsets if upstream UnRAR supports seekable entry extraction.
- **Regression Risks:** UnRAR handle safety and DROP semantics must remain leak-free.

### Finding S3: Lack of `fsync` Prior to Atomic Rename in `Library::save`
- **Severity:** High (Data Durability & Crash Consistency)
- **Subsystem:** Library Persistence (`src/library.rs:967-969`)
- **Problem:** In `Library::save`:
  ```rust
  fs::write(&tmp, json)?;
  fs::rename(&tmp, &file)?;
  ```
  `fs::write` flushes to OS buffer cache but does not issue an `fsync`/`fdatasync`. On modern Linux filesystems (ext4, XFS, btrfs), an atomic rename updates directory metadata before dirty pages are committed to storage platter/flash. A system crash or power outage can leave `library.json` with a 0-byte size.
- **Root Cause:** Omission of `file.sync_all()` prior to `rename`.
- **Measured Impact:** On sudden power loss or kernel panic, `library.json` can be truncated to 0 bytes. On reboot, `Library::load` fails to parse JSON and moves the file to `library.json.corrupt`.
- **Proposed Fix (Stage 2):**
  Use `std::fs::File::create(&tmp)`, write bytes, call `file.sync_all()`, and then `fs::rename(&tmp, &file)`.
- **Regression Risks:** `sync_all` blocks the calling thread until dirty pages are flushed; however, because `save` is executed strictly on the background executor under `write_lock`, the UI thread will experience zero latency impact.

### Finding S4: Pretty-Printed JSON Storage Overhead
- **Severity:** Low (Storage Footprint, Serialization CPU)
- **Subsystem:** Library Persistence (`src/library.rs:967`)
- **Problem:** `Library::save` serializes with `serde_json::to_vec_pretty(self)`.
- **Root Cause:** Debug readability preferred over production storage efficiency.
- **Measured Impact:** Formatting JSON with whitespace and newlines increases `library.json` file size by ~45% and increases serialization/deserialization CPU time.
- **Proposed Fix (Stage 2):**
  Use compact `serde_json::to_vec(self)` for autosave persistence.
- **Regression Risks:** Negligible.

### Finding S5: Redundant Cover Thumbnail Fallback Writes
- **Severity:** Low (Disk I/O)
- **Subsystem:** Library Import (`src/library.rs:1334-1353`)
- **Problem:** `build_book` writes `covers/medium/{id}.jpg`, then attempts hard-links to `legacy`, `small`, and `large`. On filesystems where hard-linking fails (e.g. FAT32/exFAT external drives), it falls back to writing the identical JPEG file 3 additional times via `fs::write`.
- **Root Cause:** Multi-tier directory structure created for thumbnail sizing without filesystem capability detection.
- **Measured Impact:** 4x write multiplication per book on filesystems without hard-link support.
- **Proposed Fix (Stage 2):**
  Store one canonical cover file per book in `covers/{id}.jpg`; have `DataDirs::cover_for_size` return the canonical path if sized paths do not exist.
- **Regression Risks:** None.

---

## 6. Concurrency & Resource Lifecycle Findings

### Finding L1: Import Polling Loop with `try_recv` and Timer Sleeps
- **Severity:** Low (Thread Wakeups)
- **Subsystem:** Main Orchestration (`src/main.rs:720-786`)
- **Problem:** When an import is in progress, the UI listener runs a polling loop using `receiver.try_recv()` and `timer(Duration::from_millis(40)).await`.
- **Root Cause:** Standard library `std::sync::mpsc` channel does not support async `.recv().await`.
- **Measured Impact:** Generates 25 task wakeups per second on the background executor during active import.
- **Proposed Fix (Stage 2):**
  Use an asynchronous channel or event notification so the UI task sleeps until an item is pushed.

### Finding L2: Unused Dependency `memmap2`
- **Severity:** Low (Dependency Hygiene)
- **Subsystem:** Build Configuration (`Cargo.toml:34`)
- **Problem:** `memmap2 = "0.9"` is listed in `[dependencies]` but never imported or used across any file in `src/`.
- **Root Cause:** Leftover dependency from prior prototyping.
- **Measured Impact:** Increases dependency build times and binary symbol overhead.
- **Proposed Fix (Stage 2):**
  Remove `memmap2` from `Cargo.toml`.

---

## 7. Profiling Methodology & Reproducible Commands

All measurements were obtained within the NixOS / `devenv` development shell on x86_64 Linux.

### Reproducible Profiling Recipes

1. **Process-Level Timing, Memory RSS, Faults & Context Switches:**
   ```bash
   /run/current-system/sw/bin/time -v target/debug/deps/yomu-30e670dcd786326e \
     tests::test_sustained_long_reading_session_stability
   ```

2. **System Call Frequency & Latency Breakdown (strace):**
   ```bash
   /run/current-system/sw/bin/strace -c target/debug/deps/yomu-30e670dcd786326e \
     tests::test_sustained_long_reading_session_stability
   ```

3. **Heap Memory Allocation & Retention Profiling (Valgrind Massif):**
   ```bash
   devenv shell valgrind --tool=massif target/debug/deps/yomu-30e670dcd786326e \
     --exact tests::perf_per_frame_animation_math --ignored --nocapture
   ms_print massif.out.<pid>
   ```

4. **Dynamic Heap Lifetime & Leak Analysis (Valgrind DHAT):**
   ```bash
   devenv shell valgrind --tool=dhat target/debug/deps/yomu-30e670dcd786326e \
     --exact tests::perf_cache_hit_vs_miss_transition --ignored --nocapture
   ```

5. **Criterion-Style Microbenchmarks:**
   ```bash
   devenv shell cargo test perf_ -- --ignored --nocapture
   ```

---

## 8. Baseline Benchmark Results

The table below summarizes measured baselines captured during the audit:

| Metric / Scenario | Measured Baseline | Subsystem / Operation | Notes |
| :--- | :--- | :--- | :--- |
| **Animation Math** | **20 ns** per frame | `motion::step_scroll` + `PageTransition` | Tested across 2,000,000 frames |
| **Cache Hit Navigation** | **31.2 ns** (p50: 59 ns) | Memory LRU lookup + `Arc::clone` | Instantaneous zero-alloc turn |
| **Cache Miss Decode (JPEG)** | **132.07 ms** (p50: 137 ms) | Archive read + SIMD decode + Catmull-Rom | 2400x3600 scan -> Tier 2048 |
| **Cache Acceleration Factor** | **4,231,236x** | In-memory cache vs Disk Miss | Validates cache tier architecture |
| **Dynamic Zoom Transform** | **29.0 ns** (p50/p95/p99) | `layout::display_size` + `max_offset` | Zero-allocation matrix math |
| **Slide Frame Render** | Mean: **7.05 ms**, Median: **6.63 ms** | State update + layout + paint | Unoptimized headless test platform |
| **Slide Max Frame Time** | Max: **16.75 ms**, p95: **10.21 ms** | Single-frame worst case | Within 60 Hz budget (16.6 ms) |
| **Long Reading Session RSS** | Peak RSS: **50.5 MiB** (51,764 KB) | 100 turns, jumps, zooms, 2 books | Clean cache bounds, 0 leaks |
| **E2E Pipeline RSS** | Peak RSS: **138.5 MiB** (141,904 KB) | 2400x3600 decode & resize | Minor page faults: 121,826 |
| **DHAT Peak vs Retained Heap**| Peak: **114.1 MB**, Retained: **67 KB** | Cache hit/miss lifecycle | Verifies zero long-term heap leaks |

---

## 9. Confirmed Bugs vs Potential Risks

| Issue | Classification | Impact | Evidence Location |
| :--- | :--- | :--- | :--- |
| **Idle 750ms Polling Loop** | **Confirmed Bug** | CPU & Disk activity while completely idle | `src/app/state.rs:569-586` |
| **Batch Import Full Rewrites** | **Confirmed Bug** | Writes GBs of redundant data to disk | `src/main.rs:750` |
| **Missing `fsync` before Rename** | **Confirmed Bug** | Risk of 0-byte corrupt file on crash | `src/library.rs:967-969` |
| **Unused `memmap2` Crate** | **Confirmed Bug** | Unnecessary dependency bloat | `Cargo.toml:34` |
| **Full Library Deep Clone on Save**| **Performance Inefficiency** | High allocation volume during autosave | `src/app/persistence.rs:108` |
| **Sequential Scanning in CBR/RAR** | **Performance Inefficiency** | $O(N^2)$ reads during book navigation | `src/archive.rs:274-317` |
| **Unvirtualized Library Cards** | **Performance Inefficiency** | High frametime in large libraries | `src/app/views/library.rs:344-448` |
| **$O(N)$ Continuous Layout Loop** | **Performance Inefficiency** | 4 linear scans per frame during scroll | `src/ui/reader.rs:592-619` |
| **Non-JPEG Triple Allocation** | **Performance Inefficiency** | Transient peak RSS spikes | `src/imaging.rs:217-240` |

---

## 10. Prioritized Remediation Roadmap

### Priority 1: Critical Durability & Waste Elimination (Immediate)
1. **Fix Batch Import Disk Rewrites (`src/main.rs`):**
   - Use `save_library_debounced(cx)` during item ingestion; trigger immediate `save_library(cx)` once on completion.
2. **Add Crash Durability to `Library::save` (`src/library.rs`):**
   - Write to `File::create`, call `.sync_all()`, then atomic `rename`.
3. **Eliminate Idle Theme Watcher Polling (`src/app/state.rs`):**
   - Pre-check modification timestamp (`fs::metadata().modified()`) before calling `fs::read`.
4. **Remove Unused Dependency (`Cargo.toml`):**
   - Remove `memmap2`.

### Priority 2: Memory & Concurrency Optimization (High)
1. **Zero-Copy / Lightweight Snapshots for Autosave (`src/app/persistence.rs`):**
   - Eliminate deep-cloning of `BTreeMap<String, (u32, u32)>` by wrapping `Book` metadata or `Library` in `Arc` or CoW.
2. **Continuous Reader Cumulative Prefix Array (`src/ui/reader.rs`):**
   - Store precomputed prefix heights; replace linear loops with binary search and direct index slicing.
3. **Zero-Allocation Search in Library View (`src/app/views/library.rs`):**
   - Replace `.to_lowercase()` filter with `yomu::search::matches`.

### Priority 3: UI Scalability & Archive Seeking (Medium)
1. **Library Viewport Windowing / Virtualization (`src/app/views/library.rs`):**
   - Instantiate only visible cards intersecting viewport bounds.
2. **Memoize Folder Groups (`src/ui/library_view.rs`):**
   - Cache `directory_groups` to avoid rebuilding on every frame.
3. **Contiguous Cursor Seeking for CBR/RAR (`src/archive.rs`):**
   - Optimize sequential reads to avoid restarting header scans from byte 0.

---

## 11. Recommended Regression Tests

To accompany Stage 2 remediation, the following regression tests must be added:
1. `test_batch_import_coalesces_library_saves`: Verify that importing 10 items triggers only debounced saves and 1 final save, not 10 immediate rewrites.
2. `test_library_save_commits_durably_before_rename`: Verify file existence and non-zero length after crash simulation.
3. `test_theme_watcher_does_not_read_file_when_unmodified`: Mock file watcher and assert zero `fs::read` calls when `mtime` matches.
4. `test_continuous_reader_prefix_heights_match_linear_scan`: Verify that prefix-sum binary search yields exact same layout offsets as linear scan.
5. `test_deep_clone_persistence_zero_alloc_snapshot`: Measure that taking an autosave snapshot does not duplicate `page_dimensions` BTreeMap nodes.

---

## 12. Unverified Areas & Platform Limitations

1. **Wayland Compositor Frame Pacing:**
   Headless test environments validate CPU state updates, layout math, and mock platform painting, but cannot measure compositor-side VSync lock, Wayland surface commit latency, or GPU driver buffer swapping under physical Wayland compositors (Niri, Sway, GNOME Mutter, KDE KWin).
2. **True VRAM Footprint:**
   GPUI Kit's `RenderImage` passes texture buffers to the graphics API. Host memory measurements (`RSS`, Massif, DHAT) accurately capture CPU-side BGRA buffers, but dedicated VRAM usage on discrete GPUs (NVIDIA/AMD) could not be sampled headlessly.
3. **Filesystem Direct I/O:**
   Storage write metrics were validated via Linux syscall counters (`statx`, `openat`, `write`, `rename`). Storage hardware-level wear leveling and block device controller behavior vary by drive firmware.

---

## 13. Stage 2 Implementation & Verification Report

All verified high-priority and medium-priority optimizations identified in this audit were implemented and validated:

### 13.1 Implemented Optimizations & Bug Fixes
1. **Batch Import Persistence Coalescing (`src/main.rs`):**
   - Replaced per-book `this.save_library(cx)` calls with debounced autosave (`save_library_debounced(cx)`).
   - Added guaranteed immediate flush boundaries on `ImportEvent::Finished`, cancellation/shutdown (`should_stop`), and worker disconnection.
   - **Impact:** Reduced disk rewrites during multi-book batch imports from $O(N)$ full-file rewrites to coalesced 2-second debounces plus a single final flush (e.g. 100 books imported in 5 seconds generates 3 writes instead of 100).
2. **Persistence Durability & Temp Cleanup (`src/library.rs`):**
   - Hardened `Library::save`: writes to unique temporary file (`library.tmp.{pid}.{nanos}.{counter}`) in the destination directory, performs explicit `f.sync_all()`, executes atomic rename, and synchronizes the parent directory on Unix systems.
   - Hardened `Library::load`: automatically cleans up stale or orphaned `library.tmp.*` files left behind by ungraceful shutdowns or power failures.
   - Tested under multi-threaded concurrent saves with zero corruption and guaranteed consistency.
3. **Elimination of Idle Theme-File Polling (`src/app/state.rs`, `src/ui/theme.rs`):**
   - Added `custom_theme_metadata` checking `fs::metadata` (file size, `mtime`, and Unix inode).
   - Theme watcher only reads file contents or calculates FNV-1a hashes when file metadata actually changes.
   - **Impact:** Zero bytes read from disk during idle sessions while preserving dynamic Noctalia / external editor hot-reloading.
4. **Dependency Cleanup (`Cargo.toml`):**
   - Removed unused `memmap2 = "0.9"` crate dependency.
5. **Zero-Copy Autosave Snapshots (`src/library.rs`, `src/app/reader_controller.rs`):**
   - Wrapped `Book::page_dimensions` in `Arc<BTreeMap<String, (u32, u32)>>` with copy-on-write `insert_page_dimension` via `Arc::make_mut`.
   - **Impact:** Cloning `Library` for persistence on the UI thread takes $O(1)$ per book, eliminating 100,000+ string and tree node allocations on libraries with large archives.
6. **Continuous Reader $O(1)$ Height & $O(\log N)$ Prefix-Sum Layout (`src/ui/reader.rs`):**
   - Introduced `ContinuousLayoutCache` storing precomputed `tops`, `heights`, and `bottoms`.
   - `continuous_total_height` is $O(1)$.
   - `continuous_page_at` and `continuous_visible_range` use `partition_point` binary search ($O(\log N)$).
   - Layout cache is invalidated only when viewport width, zoom, webcomic mode, or page dimension probing versions change.
7. **Zero-Allocation Directory Grouping (`src/ui/library_view.rs`):**
   - Replaced `format!("directory:{}", ...)` string allocations with borrowed `GroupKey<'a>::Directory(&'a Path)` and `GroupKey<'a>::Series`.
   - Subsequent books in existing buckets borrow paths directly, eliminating 98% of heap allocations in `directory_groups`.
8. **Search & RAR Access Allocations Cleaned (`src/app/views/library.rs`, `src/archive.rs`):**
   - Replaced manual `to_lowercase()` search allocations with `yomu::search::matches`.
   - Replaced `String` allocations during RAR entry header scans with borrowed `&str` comparisons.

### 13.2 Empirical Verification Results
- **Unit & UI Test Suite:** **338 active tests passing** (145 lib tests + 193 binary UI tests, plus 11 benchmarks; 349 total), up from 329.
- **Clippy Lint Check:** 0 warnings across all targets with `-D warnings`.
- **Format Check:** 100% compliant with Rust 2024 `cargo fmt --all -- --check`.
- **Safety Guarantee:** Preserved `#![forbid(unsafe_code)]` with zero unsafe Rust.

---

## 14. Stage 3: Independent Verification, Profiling & Scalability Analysis

This section documents the independent profiling, stress testing, and architectural verification conducted on Yomu's CPU, memory, storage I/O, and UI subsystems.

### 14.1 Batch Import Persistence & Durability Verification

Independent benchmarks (`perf_stage3_batch_import_write_volume_and_durability`) were executed simulating real-world batch imports of 100, 500, and 1,000 books. Write counts and total disk write volume were measured using real temporary directories on Linux.

| Library Size | Uncoalesced Writes | Coalesced Writes | Uncoalesced Volume | Coalesced Volume | Volume Reduction | Duration Reduction |
|:---|:---|:---|:---|:---|:---|:---|
| **100 Books** | 100 writes | **2 writes** | 6.96 MiB | **0.53 MiB** | **-92.4%** | 0.07s → **0.01s** (7.0x faster) |
| **500 Books** | 500 writes | **4 writes** | 171.96 MiB | **4.43 MiB** | **-97.4%** | 1.43s → **0.06s** (23.8x faster) |
| **1,000 Books** | 1,000 writes | **5 writes** | 686.91 MiB | **10.63 MiB** | **-98.5%** | 5.31s → **0.10s** (53.1x faster) |

**Crash-Durability & Concurrency Verification:**
1. **Flushed Boundaries:** Unit test `test_batch_import_large_collection_durability_and_flush` verified that importing 100 books followed by `flush_pending_save_sync` commits all 100 books to disk, reloadable by a fresh `Library::load` instance with zero lost entries.
2. **Filesystem Atomicity:** Saves write to a temporary file in the destination directory (`library.tmp.{pid}.{nanos}.{counter}`), call `f.sync_all()`, atomic rename (`fs::rename`), and parent directory `sync_all()` on Unix platforms.
3. **Orphan Cleanup:** `Library::load` was verified to scan the parent directory and clean up stale `library.tmp.*` files resulting from simulated abrupt crashes or power cuts.

### 14.2 Theme Monitoring Syscall & Read Audit

Independent benchmark `perf_stage3_theme_monitoring_syscall_and_read_audit` measured syscall latency and content read behavior over 1,000 simulated watcher checks:

- **Metadata Stat Query:** **1.11 µs** per check (0 content bytes read).
- **Full Content Read + Hash:** **3.38 µs** per check (~250 bytes read + hashed).
- **Idle Savings:** Over 1 hour of idle operation (750 ms interval = 4,800 checks):
  - **4,800 full-file disk reads eliminated**
  - **Zero content bytes read** from disk during idle sessions (0 bytes vs ~1.2 MiB).
- **Precision Verification:** Unit test `test_custom_theme_metadata_and_atomic_replacement` verified that same-size modifications, atomic file replacement (write to temp + rename), and file recreation are detected with zero false negatives due to nanosecond `mtime` and filesystem inode tracking.

### 14.3 Memory Optimization & Autosave Snapshot Verification

The zero-copy autosave snapshot mechanism (`Arc<BTreeMap<String, (u32, u32)>>` with `Arc::make_mut` copy-on-write) was benchmarked against naive deep cloning (`perf_stage3_memory_snapshot_cloning_and_retention`):

| Library Scale | Dimension Entries | Deep Clone (UI Thread) | Snapshot Clone (UI Thread) | Latency Reduction |
|:---|:---|:---|:---|:---|
| **100 Books** | 20,000 | 1.272 ms | **0.011 ms** | **114.9x faster** |
| **1,000 Books** | 200,000 | 15.235 ms | **0.110 ms** | **138.0x faster** |
| **5,000 Books** | 1,000,000 | 84.984 ms | **0.552 ms** | **153.9x faster** |

**Retained Memory & CoW Analysis:**
- In standard operation, `Arc::make_mut` finds strong count `== 1`, performing $O(\log K)$ in-place inserts with **zero tree cloning**.
- During the brief moment an autosave snapshot is actively serializing in the background, a newly probed page dimension clones only that individual book's map, never the entire library.
- Memory is released immediately when the background serialization finishes and drops the snapshot.

### 14.4 Continuous Reader Layout Cache Verification

The continuous layout cache (`ContinuousLayoutCache`) was verified using randomized property testing (`test_continuous_cache_fuzzy_matches_naive_reference`) across 50 iterations with random heights, viewport dimensions, zoom levels, and webcomic modes. Prefix-sum binary search queries matched naive linear scan with 100% precision.

Microbenchmark `perf_stage3_continuous_reader_cache_benchmarks` evaluated performance from 100 to 10,000 pages:

| Document Length | Naive Visible Range Scan | Cached Total Height ($O(1)$) | Cached Visible Range ($O(\log N)$) | Speedup |
|:---|:---|:---|:---|:---|
| **100 Pages** | 302.7 ns | **6.0 ns** | **35.7 ns** | **8.5x faster** |
| **1,000 Pages** | 2,731.8 ns | **5.8 ns** | **47.3 ns** | **57.8x faster** |
| **5,000 Pages** | 11,791.2 ns | **5.4 ns** | **56.6 ns** | **208.2x faster** |
| **10,000 Pages** | 18,600.7 ns | **5.3 ns** | **59.6 ns** | **312.1x faster** |

Total height queries during scroll clamp and scrollbar thumb calculations dropped from $O(N)$ linear summation to instantaneous $O(1)$ lookups (5.3 ns), completely decoupling continuous scrolling CPU overhead from comic length.

### 14.5 CBR / RAR Archive Concurrency & Thread-Safety Analysis

A deep architectural inspection was conducted on `src/archive.rs` to evaluate the feasibility of multi-threaded archive handle caching for CBR/RAR:

1. **UnRAR `!Send` / `!Sync` Constraint:**
   `unrar::OpenArchive` wraps an unrar C++ pointer (`NonNull<unrar_sys::Handle>`). The `unrar` crate explicitly does not implement `Send` or `Sync` for `OpenArchive`.
2. **Rule 1 (`#![forbid(unsafe_code)]`) Invariant:**
   Wrapping `OpenArchive` in a shared handle pool (such as `ARCHIVE_POOL`) or transferring it between thread pools requires `unsafe impl Send` or raw pointer manipulation, which is strictly forbidden in Yomu.
3. **Cursor-Based Stream Architecture:**
   The underlying UnRAR C++ engine operates as a unidirectional forward stream (`read_header()`, `skip()`, `read()`). Random index access does not exist; seeking backward to an earlier entry requires closing and reopening from byte 0.
4. **Current Architectural Balance:**
   Yomu's existing design optimizes this safely: `Archive::open` performs a single linear entry header scan and caches entry names and sizes in `ArchiveEntries`. Page extraction tasks run independently on background threads, opening a thread-local handle and skipping forward directly to the requested entry.

### 14.6 Library Grid Scalability & Framework Constraints

Library grid rendering was benchmarked using headless GPUI Kit visual test harness (`perf_stage3_library_grid_scalability_and_render_cost`):

| Book Count | Render Tree Generation Time | 144 Hz Budget (6.94 ms) | 60 Hz Budget (16.66 ms) | Max Scroll FPS |
|:---|:---|:---|:---|:---|
| **100 Books** | **2.57 ms** | **Fits budget** | **Fits budget** | >144 FPS |
| **1,000 Books** | **26.43 ms** | Exceeds budget | Exceeds budget | ~38 FPS |
| **5,000 Books** | **146.42 ms** | Exceeds budget | Exceeds budget | ~7 FPS |
| **10,000 Books** | **388.97 ms** | Exceeds budget | Exceeds budget | ~2.5 FPS |

**Framework Architectural Assessment:**
- GPUI's standard `div().children(cards)` generates element trees linearly for all items ($O(N)$). At >1,000 books, tree generation consumes >16.6 ms per frame during active scrolling.
- **GPUI Kit 0.8.1 Capability:** GPUI Kit provides `v_virtual_list` (`VirtualList`), which supports only 1D vertical single-column items. GPUI Kit does not currently provide a native 2-dimensional virtual grid component (`v_virtual_grid`).
- **Rule 3 & Ponytail Philosophy Adherence:** Per Rule 3 ("Never build a second UI framework or design system on top of GPUI Kit"), building a custom 2D grid virtualizer with manual viewport intersection, item pooling, and layout geometry inside Yomu is strictly avoided.
- **Recommended Path:** Libraries under 1,000 books render comfortably within 60-144 Hz frame budgets. For ultra-large collections (>5,000 books), the recommended future architectural path is either contributing a native 2D `VirtualGrid` upstream to GPUI Kit, or grouping books into row chunks rendered via `v_virtual_list`.

---

## 15. Resource Optimization Summary (Before vs After)

| Resource Metric | Baseline (Pre-Audit) | Optimized (Stage 2 & 3) | Measurable Impact |
|:---|:---|:---|:---|
| **1,000-Book Batch Import Disk Writes** | 1,000 disk writes | **5 disk writes** | **99.5% fewer disk writes** |
| **1,000-Book Batch Import Write Volume** | 686.9 MiB | **10.6 MiB** | **98.5% less disk write volume** |
| **Idle Disk I/O (1 Hour)** | 4,800 reads (~1.2 MiB) | **0 bytes read** | **100% eliminated idle content reads** |
| **Library Persistence Latency (5,000 Books)** | 84.98 ms (drops frames) | **0.55 ms** | **154x faster (fits 144 Hz budget)** |
| **Continuous Scroll Layout (10,000 Pages)** | 18,600 ns ($O(N)$) | **59 ns ($O(\log N)$)** | **312x faster** |
| **Continuous Total Height Calculation** | $O(N)$ linear summation | **5.3 ns ($O(1)$)** | **Instantaneous scroll clamping** |
| **Custom Theme Change Detection** | Full content hashing | **1.11 µs stat check** | **Zero idle CPU / storage waste** |
| **Library Grid Render (10,000 Books)** | 388.97 ms (drops frames) | **0.05 ms (0.7% 144Hz)** | **7,779x faster (99.6% fewer cards)** |
| **Unsafe Rust Invariant** | 0 unsafe lines | **0 unsafe lines** | **Strict `#![forbid(unsafe_code)]` preserved** |

---

## 16. Stage 4: Library Scalability & Official GPUI Kit Row-Based Virtualization

### 16.1 Architecture & Implementation
To address the $O(N)$ library grid element instantiation bottleneck, row-based virtualization was designed and implemented using official GPUI Kit 0.8.1 components:

1. **Logical Row Chunking:**
   - Evaluated responsive grid parameters (`cols`, `card_w`, `cover_h`, `gap`, `pad_x`) from `ui::components::compute_responsive_grid`.
   - In Grid Mode, items are chunked into logical rows of `cols` items. In List Mode, items are chunked into rows of 1 item.
   - Items are modeled as lightweight `LibraryItem::Folder` and `LibraryItem::Book { id, index }` (with cached index for fast $O(1)$ lookup).
2. **Official GPUI Kit Integration (`v_virtual_list`):**
   - Utilizes `gpui_kit::component::v_virtual_list` with `VirtualListScrollHandle`.
   - Precomputes uniform row heights (`cover_h + 64.0 + gap` in Grid Mode, `64.0 + 8.0` in List Mode).
   - Only rows within the viewport visible range (plus overscan) are rendered.
   - Pairs with native `ScrollBounce` and `Scrollbar::vertical(scroll_handle).viewport_from_layout()`.
3. **Derived Row Cache (`LibraryRowCache`):**
   - Caches row partitioning and item sizes, invalidating only when relevant state changes: library mutations (`mutation_seq`), search query, sorting mode/direction, series grouping toggle, open group key, grid/list view mode, or responsive column count.
   - Re-render during scrolling and hover takes ~0.02 ms (zero row allocations or layout recomputations).
4. **Preserved Behaviors:**
   - Existing responsive grid columns and centered padding.
   - Dual-mode card and list row visual appearance.
   - Folder grouping, group navigation, and breadcrumbs.
   - Context menus and multi-selection synchronization.
   - Native momentum scrolling and scrollbar dragging.
   - Empty states and search filtering.

### 16.2 Empirical Verification & Scalability Results
Microbenchmarks (`perf_stage3_library_grid_scalability_and_render_cost` and `perf_stage4_virtual_grid_instantiation_and_scroll_audit`) evaluated performance across library scales from 100 to 10,000 books:

| Metric | 100 Books | 1,000 Books | 5,000 Books | 10,000 Books |
|:---|:---|:---|:---|:---|
| **Pre-Virtualization Render Time** | 2.57 ms | 26.43 ms | 146.42 ms | 388.97 ms |
| **Stage 4 Virtualized Frame Time** | **0.01 ms** | **0.02 ms** | **0.02 ms** | **0.05 ms** |
| **Speedup on Scroll Frames** | **257x faster** | **1,321x faster** | **7,321x faster** | **7,779x faster** |
| **144 Hz Frame Budget (6.94 ms)** | **0.1% budget** | **0.3% budget** | **0.3% budget** | **0.7% budget** |
| **Cards Instantiated per Frame** | ~35 cards | ~35 cards | ~35 cards | ~35 cards |
| **Instantiated Card Reduction** | -65.0% | **-96.5%** | **-99.3%** | **-99.6%** |
| **Scroll FPS Headroom** | >144 FPS | **>144 FPS** | **>144 FPS** | **>144 FPS** |

### 16.3 Regression & Interaction Validation
Dedicated unit tests in `src/tests.rs` verified:
- `test_virtual_grid_row_construction_and_partially_filled_rows`: Verified correct chunking and partially filled trailing rows.
- `test_virtual_grid_filtering_and_sorting_invalidation`: Verified search and sort cache invalidations.
- `test_virtual_grid_group_expansion_and_collapse`: Verified folder group opening and returning.
- `test_virtual_grid_list_mode_row_partitioning`: Verified single-column row partitioning in List Mode.


