# Stage 8: Production-Readiness & Stability Audit Report

**Date:** 2026-11-04  
**Auditor:** Antigravity / DeepMind Pair Programming Agent  
**Target:** Yomu 26.10.0 (`gpui-kit = 0.8.1`, `gpui-pre = 0.3.10`, Safe Rust 2024, `#![forbid(unsafe_code)]`)  
**Commit Inspected:** `62d4df9` (Clean tree)  
**Final Release Verdict:** **CONDITIONALLY READY**

---

## 1. Executive Summary

A comprehensive, evidence-driven production-readiness and stability audit was conducted on **Yomu**, an offline, keyboard-first desktop manga and comic reader built with GPUI and GPUI Kit 0.8.1.

Across previous optimization stages (Stages 1 through 7), Yomu eliminated:
- $O(N^2)$ quadratic scanning during CBR/RAR archive reading via `RarSession`.
- Full-library re-serialization on reading progress updates via the `progress.json` sidecar.
- Frame-rate degradation in large collections via pre-validated virtual grid row caching (`0.011 ms` cached frame render at 10,000 books).
- Memory leakage through explicit texture garbage collection (`Window::drop_image`), cooperative task cancellation tokens, and bounded cover LRU caching.

This Stage 8 audit independently verified:
- **Zero Unsafe Code:** Confirmed `#![forbid(unsafe_code)]` at all library and binary roots.
- **Validation Matrix:** 100% pass rate across formatting (`cargo fmt`), strict clippy lints (`-D warnings`), 366 active tests, and 18 performance benchmarks.
- **Data Integrity & Durability:** Crash-safe atomic persistence (`tempfile` + `write_all` + `sync_all` + atomic `rename` + parent directory `sync_all` on Unix) for both `library.json` and `progress.json`.
- **Zero Leaked Textures:** Atlas regions in `WgpuAtlasTextures`, `DirectXAtlasTextures`, and `MetalAtlasTextures` are verified to be deallocated upon `window.drop_image()`.

### Key Limitations & Release Risks
1. **Untrusted RAR Header Decompression Bomb (MAJOR):** While declared `unpacked_size` is checked against `MAX_PAGE_BYTES` (128 MB) prior to decompression, the upstream `unrar` 0.5.8 crate uses an unbounded `ReadToVec` callback. A crafted RAR archive that misstates its header unpacked size will decompress unbounded data until host memory exhaustion (OOM crash) before Yomu's post-read check can intercept it.
2. **Interactive Platform Verification (MODERATE):** Headless test harnesses (`gpui_kit::TestAppContext`) validate state transitions, event handling, and data flows, but physical compositor transparency, multi-monitor scaling, and Windows file-locking retry semantics remain visually unverified on physical hardware.

---

## 2. Repository & Environment Details

| Component | Specification |
| :--- | :--- |
| **Rust Version** | `rustc 1.99.0 (b940084d7 2026-09-28)` (Rust 2024 Edition) |
| **MSRV** | Rust 1.92 |
| **Frameworks** | `gpui-kit = "=0.8.1"`, `gpui-pre = "0.3.10"` |
| **Host OS** | Linux 6.12 (NixOS / `devenv` reproducible environment) |
| **Graphics Runtime** | GPUI WGPU/Blade (Wayland / X11 fallback on Linux; DirectX 12 on Windows; Metal on macOS) |
| **Primary Code Targets** | `src/lib.rs` (Domain Core), `src/main.rs` (Binary & UI Presentation) |
| **User Protection Invariant**| `problems.md` unedited; user manga files never modified or deleted |

---

## 3. Actual Test & Benchmark Inventory

The automated test suite was executed across all targets using `cargo test --all-targets`:

```
running unittests src/lib.rs: 154 passed; 0 failed; 2 ignored (benchmarks)
running unittests src/main.rs: 212 passed; 0 failed; 16 ignored (benchmarks)
Total Active Tests: 366 passed; 0 failed (100% pass rate)
Total Performance Benchmarks: 18 ignored (opt-in via --ignored)
Total Test Cases: 384
```

### Complete Benchmark Inventory (18 Ignored Targets)
1. `archive::tests::perf_stage6c_cbr_baseline`
2. `archive::tests::perf_stage6c_cbr_remediated`
3. `tests::perf_cache_hit_vs_miss_transition`
4. `tests::perf_real_world_presentation_and_navigation_breakdown`
5. `tests::perf_virtual_scroll_card_rebuild_cost`
6. `tests::perf_virtual_grid_render_latency_stress`
7. `tests::perf_render_book_card_instantiation_latency`
8. `tests::perf_search_filtering_latency_and_throughput`
9. `tests::perf_stage3_continuous_reader_cache_benchmarks`
10. `tests::perf_stage3_theme_metadata_polling_benchmark`
11. `tests::perf_stage3_library_grid_scalability_and_render_cost`
12. `tests::perf_stage4_virtual_grid_instantiation_and_scroll_audit`
13. `tests::perf_stage5_deep_memory_and_ssd_io_audit`
14. `tests::perf_stage6b_persistence_baseline`
15. `tests::perf_stage6b_remediated_progress_persistence`
16. `tests::perf_stage7_animation_scrolling_rendering_breakdown`
17. `tests::perf_continuous_layout_scaling`
18. `tests::perf_manga_screentone_psnr`

All 18 benchmarks compile, pass, and reproduce baseline/remediated metrics when invoked explicitly.

---

## 4. Memory Management & Resource Lifecycle Findings

### A. CPU Memory & Buffer Management
- **Zero Per-Frame Heap Clones:** Stage 7 eliminated composite string allocations in card rendering (`(ElementId::from("manga-item"), book.id.as_str())`), continuous layout vectors (`clear()` + in-place reuse in `ContinuousLayoutCache`), and page counter string formatting (`CachedPageCounter` backed by stack-inlined `SmolStr`).
- **Memory Drift in Long Sessions:** Verified via `perf_stage5_deep_memory_and_ssd_io_audit`:
  - 100 repeated book open/navigate/close cycles: Baseline RSS `52.21 MB` $\rightarrow$ Post-workload RSS `53.07 MB` (net drift: `+0.86 MB`, fully stabilized within OS allocator arena).
  - Open file descriptors: Initial = 78, Post = 78 (zero file descriptor leaks).
- **Buffer Reuse:** Archive decompression buffers reuse `wanted_scratch` and pre-allocated vectors up to `INITIAL_PAGE_BUF_CAPACITY` (8 MB), avoiding heap fragmentation during sustained reading.

### B. GPU Memory & Texture Lifecycle
- **Atlas Deallocation Verification:** Direct source inspection of `gpui-pre-wgpu-0.3.10/src/wgpu_atlas.rs` confirmed that `Window::drop_image()` invokes `WgpuAtlasTextures::remove()`, executing `texture.allocator.deallocate(tile.tile_id.into())` and returning the texture ID to `storage[kind].free_list`.
- **Texture Drain Hooks:** `Yomu::render()` executes `self.drain_garbage(window)` both before and after event processing, guaranteeing that evicted textures do not outlive the frame in which they were dropped.
- **Bounded Cover Cache:** `BoundedCoverCache` restricts memory to 256 entries. Because thumbnail covers are pre-scaled on disk (Small: 360x540, Medium: 440x660, Large: 540x810 BGRA8), memory consumption is bounded between `197 MB` and `448 MB`.

---

## 5. Persistence, Crash Recovery & Data Integrity Findings

### A. Dual Persistence Model (`library.json` + `progress.json`)
- **Isolation of Concerns:** Book additions, metadata edits, and layout settings persist to `library.json` (format version 3). Page turns, opened status, and `last_read` timestamps persist to `progress.json` (schema version 1).
- **Write Amplification Reduction:** Verified via `perf_stage6b_remediated_progress_persistence`:
  - For a 10,000-book collection, 100 rapid page turns generate only `2.12 KB` of disk writes (debounced to 2 atomic flushes), compared to `1,658.34 MB` under legacy full-library writes (**99.99987% reduction**).
- **Atomic File Replacement:** Both persistence paths execute:
  1. In-memory serialization via `serde_json::to_vec_pretty`.
  2. Writing to a PID- and timestamp-unique temporary file (`progress.tmp.<pid>.<time>.<counter>`).
  3. `f.sync_all()` ensuring physical drive write.
  4. Explicit handle drop before `fs::rename`.
  5. Atomic rename replacing the destination.
  6. `sync_all()` on parent directory on Unix systems.

### B. Crash & Corruption Resilience
- **Corrupt File Quarantine:** If `progress.json` is corrupted or truncated, `ProgressStore::load()` automatically moves it to `progress.json.corrupt`, logs a recovery message, harvests baseline progress from `library.json`, and initializes a fresh store without crashing.
- **Orphan Cleanup:** `load()` scans the parent directory and unlinks any leftover `progress.tmp.*` or `library.tmp.*` files from prior system crashes.
- **Shutdown Persistence Guarantee:** `window.on_window_should_close` intercepts window close events, sets `is_quitting = true`, and runs an async flush of both stores. Only upon completion does it call `cx.quit()`. If saving fails, it presents a modal `ConfirmDialog::ForceQuit` asking whether to discard changes.

---

## 6. Archive Handling & Security Findings

### A. Safe Path Traversal Guards
- `is_safe_archive_entry_name()` enforces strict sanitization on all archive entries:
  - Rejects empty names, absolute paths (`/` and `\`), null bytes (`\0`), and Windows alternate data streams / drive prefixes (`:`).
  - Splitting on `/` and `\` rejects any component that is `.`, `..`, starts with `.`, or equals `__MACOSX`.
- No archive extraction to disk: archive pages are decoded directly in memory.

### B. Decompression Bombs & Resource Limits
- **CBZ (Zip):** Hard pre-allocation limit:
  ```rust
  let bytes_read = entry.take(MAX_PAGE_BYTES + 1).read_to_end(&mut buf)?;
  if bytes_read > MAX_PAGE_BYTES as usize { bail!("unreasonably large"); }
  ```
  `take()` enforces a strict 128 MB streaming cut-off regardless of declared header values.
- **CBR (RAR) Vulnerability (MAJOR):**
  - In `src/archive.rs`, `header.entry().unpacked_size > MAX_PAGE_BYTES` is validated before extraction.
  - However, upstream `unrar::OpenArchive::read()` relies on `ReadToVec::process_data(&mut vec, chunk)` in C++ unrar callback, which performs unbounded `vec.extend_from_slice(chunk)`.
  - If a malicious RAR archive spoofed its header unpacked size to `< 128 MB` while containing a multi-gigabyte compression bomb, the worker thread will allocate uncontrollably until OS OOM abort occurs before line 431 (`data.len() > MAX_PAGE_BYTES`) is reached.

### C. Worker Confinement & Sequential Seeking (`RarSession`)
- `unrar::OpenArchive` is confined to a dedicated OS thread (`yomu-rar-worker`).
- Moving forward sequentially maintains the open handle ($O(1)$ page traversal).
- Moving backward or random access recognizes that `target_is_behind` and reopens the archive from the start ($O(N)$ traversal), correctly preserving decompression state for solid RAR archives.
- Worker threads join on drop; channels are bounded (32 items) with non-blocking reply sends, preventing deadlocks during task cancellation.

---

## 7. Concurrency, Cancellation & Thread Safety

- **Cooperative Cancellation:** Every page decode task receives an `Arc<AtomicBool>`. If the user turns the page or closes the book, `token.store(true, Ordering::Relaxed)` is set. Background decoders check this token before disk reading, after archive extraction, and before image resampling.
- **Stale Page Rejection:** `page_decoded()` compares the task's token pointer against `reader.cancellations.get(&ix)`. If a new book was opened or a newer decode was requested, stale image data is dropped immediately without updating the UI.
- **UI Thread Safety:** Synchronous disk operations (`list_pages`, `read_page`, `decode_fit_bgra`, `save`) run strictly on `cx.background_executor()` or dedicated worker threads. The UI render thread executes zero blocking filesystem calls.

---

## 8. Rendering, Scrolling & Virtualization Performance

- **Virtual Grid Partitioning:** Library cards and list rows are virtualized using GPUI Kit's `v_virtual_list`. Only visible rows plus a 1-row overscan are instantiated.
- **Scroll Cache Hit Rate:**
  - 1,000 books: `0.012 ms` per frame.
  - 5,000 books: `0.012 ms` per frame.
  - 10,000 books: `0.011 ms` per frame (609x headroom under 144 Hz frame budget).
- **Continuous Mode Physics:** Offset settlement is computed in $O(1)$ from `tops[page]` and `heights[page]` in `46 ns`, eliminating layout stuttering during rapid scrolling.

---

## 9. Dependencies & Supply Chain Security

| Crate | Version | License | Security / Audit Status |
| :--- | :--- | :--- | :--- |
| `gpui-kit` | `0.8.1` | Apache-2.0 / MIT | Clean, vendored `gpui-component` patched locally |
| `gpui-pre` | `0.3.10` | Apache-2.0 / MIT | Clean, official Zed GPUI runtime |
| `fast_image_resize`| `6` | MIT / Apache-2.0 | Pure Rust, SIMD-accelerated, no C bindings |
| `image` | `0.25` | MIT / Apache-2.0 | Default features disabled, pure-Rust decoders |
| `zune-jpeg` | `0.5` | MIT / Apache-2.0 / Zlib | Fast SIMD JPEG decoder |
| `zip` | `2` | MIT | Pure Rust Zip archive handling |
| `unrar` / `unrar_sys`| `0.5.8` | MIT / Apache-2.0 (wrapper) | **Alexander Roshal Freeware License** for vendored C++ UnRAR |

### Licensing Notice for Linux Distributions
The vendored UnRAR C++ code prohibits using the source code to re-create the RAR compression algorithm. This makes UnRAR non-OSI compliant, restricting Yomu from being packaged into Fedora "free" or Debian "main" repositories without building against a free alternative or placing Yomu into "non-free".

---

## 10. Cross-Platform Compatibility Matrix

| Platform | Verification Status | Notes |
| :--- | :--- | :--- |
| **Linux (Wayland)** | **VERIFIED (Headless) / UNVERIFIED (Compositor Visuals)** | Safe Wayland-native windowing. Headless automated tests verify state and events. Compositor transparency and shadows require interactive desktop verification. |
| **Linux (X11)** | **VERIFIED (Headless)** | Fallback supported by GPUI X11 backend. |
| **Windows** | **CODE-INSPECTED / STATICALLY VERIFIED** | Resources compiled via `embed-resource`. Potential transient file locking on `fs::rename` under aggressive antivirus scanners. |
| **macOS** | **CODE-INSPECTED / STATICALLY VERIFIED** | Retina/HiDPI and Metal atlas supported via `gpui-pre-apple`. Momentum scrolling and native menus configured. |

---

## 11. Severity-Ranked Issue Register

### Major Issues

#### ISSUE-8-01: Untrusted RAR Archive Decompression Bomb Exhaustion
- **Severity:** **MAJOR**
- **Location:** `src/archive.rs:422-438`, `src/archive.rs:571-579`
- **Description:** `unrar::OpenArchive::read()` relies on upstream `ReadToVec`, which appends chunks to `Vec<u8>` inside the C++ callback without checking length. If an untrusted RAR archive specifies an entry whose declared header `unpacked_size` is small, but whose compressed stream expands to gigabytes, the process will allocate memory uncontrollably and crash due to OOM before Yomu's post-read check runs.
- **Impact:** Denial of service / application crash when opening maliciously crafted CBR archives.
- **Recommended Remediation:** Fork or patch `unrar` to pass a maximum output byte limit into the decompression callback, or validate stream expansion via a bounded custom callback.

---

### Moderate Issues

#### ISSUE-8-02: Lack of Atomic Rename Retry Loop on Windows
- **Severity:** **MODERATE**
- **Location:** `src/library.rs:1046`, `src/progress.rs:257`
- **Description:** On Windows, `std::fs::rename` can fail with `ERROR_ACCESS_DENIED` or `ERROR_SHARING_VIOLATION` if an antivirus scanner or Windows Search indexer temporarily opens the newly closed `.tmp` file.
- **Impact:** Occasional save failures on Windows systems running third-party antivirus software.
- **Recommended Remediation:** Introduce a small retry loop (e.g. 3 attempts with 20 ms exponential backoff) around `fs::rename` on Windows targets.

#### ISSUE-8-03: Vendored UnRAR License Non-OSI Restriction
- **Severity:** **MODERATE**
- **Location:** `Cargo.toml:35` (`unrar = "0.5.8"`)
- **Description:** UnRAR's license restricts reverse-engineering the compression algorithm, which prevents inclusion in pure open-source distribution repositories (Debian main, Fedora official).
- **Impact:** Packaging limitation for official Linux distribution maintainers.
- **Recommended Remediation:** Document distribution guidelines clearly in `README.md` and `docs/development.md`. In future phases, consider optional pure-Rust 7z/LZMA or libarchive backends.

---

### Minor Issues

#### ISSUE-8-04: BoundedCoverCache Bounds Entry Count Rather Than Total Bytes
- **Severity:** **MINOR**
- **Location:** `src/ui/cover_cache.rs:16-23`
- **Description:** The cache limits entries to `256` items rather than tracking total raw texture bytes. While thumbnail covers are clamped to predictable dimensions on disk, full-resolution images passed directly could theoretically increase memory footprint.
- **Impact:** Minor memory variability on low-RAM systems (e.g. 2 GB devices).
- **Recommended Remediation:** Track total allocated texture bytes in `BoundedCoverCache` alongside entry count.

---

## 12. Release Blockers

There are **zero critical release blockers** that cause data loss, file corruption, or UI deadlocks in standard user operation:
- User comic collections on disk are never altered or deleted.
- Reading progress is completely decoupled from full library writes.
- 100% of the 366 unit and UI tests pass.
- `#![forbid(unsafe_code)]` is strictly enforced.

---

## 13. Recommended Remediation Order

1. **Phase 8A (Pre-Release Polish):** Implement bounded streaming or size verification for `unrar` page extraction to eliminate decompression bomb vulnerability.
2. **Phase 8B (Windows Reliability):** Add transient retry backoff to atomic file renames on Windows.
3. **Phase 8C (Packaging & Verification):** Perform physical interactive verification on Wayland (Niri/Hyprland/GNOME) and package release binaries (AppImage, Flatpak, Windows MSI, macOS DMG).

---

## 14. Final Release Verdict

### **CONDITIONALLY READY**

**Verdict Statement:**
Yomu demonstrates production-grade performance, memory discipline, crash recovery, and data durability. All 366 regression tests and 18 performance benchmarks pass cleanly. Yomu is qualified for **Release Candidate (RC)** distribution to early adopters, pending remediation of the CBR decompression-bomb vector (`ISSUE-8-01`) and completion of interactive Wayland/Windows hardware testing.
