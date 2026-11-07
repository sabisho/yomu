# Agent Handoff

This document is the active operational snapshot for coding agents working on Yomu. Read it after [AGENTS.md](../AGENTS.md) and [GEMINI.md](../GEMINI.md). For structural invariants and module boundaries, see [docs/architecture.md](architecture.md); for user-facing features and keybindings, see [README.md](../README.md).

---

## 1. Current Implementation Status

Yomu is a high-performance desktop manga and comic reader built in safe Rust 2024 using **GPUI Kit 0.9.0** (`gpui-fast = 0.1.13` runtime via feature `gpui-fast`) under `#![forbid(unsafe_code)]`.

- **Primary Target:** Linux (Wayland-native with X11 fallback; Nix/devenv native packaging).
- **Secondary Targets:** macOS and Windows compatibility maintained through platform abstraction layers.
- **UI Architecture:** Modern macOS HIG two-pane design (persistent 240px sidebar, context-aware 52px toolbar, responsive content views, edge-to-edge immersive reader with native `ScrollbarMode::Scrolling` auto-hide scrollbars and boundary scroll bounce).
- **Test Baseline:** **391 active tests** passing (171 core library unit tests in `src/lib.rs`, 220 binary-target UI tests in `src/main.rs`, plus 16 opt-in performance benchmarks; 407 total).
- **Clean Toolchain:** Zero compiler errors, zero warnings under `devenv shell cargo check --all-targets` and `devenv shell cargo clippy --all-targets --all-features -- -D warnings`, fully formatted under `devenv shell cargo fmt --all -- --check`.

---

## 2. Completed Engineering Milestones

### Phase 1: Critical Reliability & Performance Foundations (Completed)
- **Prefetch Cancellation Defect Resolved:** Differentiated worker task aborts from decode failures in `request_page` and `page_decoded`; tasks aborted during rapid scrubbing are safely dropped without polluting `reader.failed`. Cancelled pages remain immediately requestable on subsequent navigation.
- **UI-Thread Synchronous I/O Eliminated:** Cached `pagecache::system_memory_budget()` output via `std::sync::OnceLock<u64>` (512 MiB to 3.0 GiB adaptive policy with 1.0 GiB fallback), eliminating per-frame `/proc/meminfo` reads.
- **Archive Buffer Allocation Bounded:** Capped initial buffer allocation in `archive::read_zip_page` to `INITIAL_PAGE_BUF_CAPACITY` (8 MiB) while streaming entries up to `MAX_PAGE_BYTES` (256 MiB) to guard against zip bombs.

### Phase 2: Architectural Modularization (Completed)
- **Phase 2A (Core Domain Decoupling):** Relocated `src/scroll.rs` to `src/ui/scroll.rs`, fully decoupling `src/lib.rs` from GPUI and presentation dependencies.
- **Phase 2B (Root State Extraction):** Created `src/app/state.rs` extracting `struct Yomu`, `NavDestination`, navigation router, and lifecycle initialization.
- **Phase 2C (Reader Controller Extraction):** Created `src/app/reader_controller.rs` handling reader lifecycle, async decode queue, prefetch cancellation safety, and gestures.
- **Phase 2D.1 (Keybindings Extraction):** Created `src/app/keybindings.rs` handling keyboard shortcut event dispatch (`on_key`), Vim actions (`apply`), overlay keys (`on_overlay_key`), and library keys (`on_library_key`).
- **Phase 2D.2 (Persistence Coordination & Graceful Shutdown):** Created `src/app/persistence.rs` extracting non-blocking async persistence dispatch, lock-free monotonic sequencing (`Arc<AtomicU64>`), background `write_lock: Arc<Mutex<()>>`, cooperative import cancellation, and 4-tier graceful shutdown isolation (`request_quit`, native pointer occlusion barrier, error recovery dialog with `ForceQuit`).
- **Phase 2D.3 (Desktop Integration):** Created `src/app/desktop.rs` handling XDG desktop entry generation and multi-resolution hicolor icon installation (`--install-desktop`).
- **Phase 2E.1 (Reader Presentation Relocation):** Moved reader visual presentation (`render_reader`, `page_layer`, `reader_header`, `lateral_turners`, `ReaderScrollHandle`) into `src/ui/reader.rs` with pure `ReaderProps`.
- **Phase 2E.2 (Actions & Context Menus):** Created `src/app/actions.rs` encapsulating context menu generation, action routing, confirmations, series dialogs, and removal workflows.
- **Phase 2E.3 (View Adapters Decomposition):** Extracted `src/app/views/` (`shell.rs`, `library.rs`, `continue_reading.rs`, `collections.rs`, `history.rs`, `details.rs`, `settings.rs`), reducing `src/main.rs` to ~1,330 lines focused on entry, window setup, texture GC, and root `Render`.

### Phase 3: Performance & Scalability (Completed)
- **Phase 3.1 (Filesystem Checks Elimination):** Replaced per-book `.exists()` in shell rendering with cached `downloads_count` computed on background executor; replaced synchronous details cover check with async `cover_available: bool`.
- **Phase 3.2 (Quadratic Lookups Elimination):** Eliminated $O(N^2)$ linear scans in `render_all_manga_view`, iterating directly over `self.library.books()` and borrowing `group.books` from `directory_groups()`.
- **Phase 3.3A (Cover Path Optimization):** Precomputed base cover directories in `DataDirs`, removed synchronous `.exists()` stat calls from `cover_for_size()`, and added automatic legacy cover migration.
- **Phase 3.3B (Library Allocation Reduction):** Replaced `book.id.clone()` in card/row closures with `Arc<str>`, eliminated bulk ID vector cloning in folder rows, and switched to borrowed `HashSet<&str>`.
- **Phase 3.3C (Reader Frame-Loop Allocation Reduction):** Cached `book_title: SharedString` on `Reader`, replaced per-frame `keep: HashSet` with scalar `RangeInclusive<usize>`, deferred eviction allocations, and reused `reader.entries_scratch` buffer.

### Phase 4: UI/UX Refinement, Layouts & Restrained Motion (Completed)
- **Phase 4.1 (Library Sorting):** Implemented persistent library sorting for All Manga and Downloads (Title natural sort, Date Added, Last Read, Reading Progress, Default order) via toolbar dropdown.
- **Phase 4.2 & 4.2.1 (Dual-Page Spread Mode & Mixed-Source Support):** Implemented native dual-page spread layout with automatic detection of single-image double-page spreads via aspect ratio threshold (`1.25`), lightweight dimension probing, manual overrides (`Auto`, `ForceSingle`, `ForceSpread`), precomputed `SpreadPlan` with zero per-frame allocations, and cover isolation toggle.
- **Phase 4.3 (Restrained Motion Transitions & Reader Polish):** Introduced subtle, interruptible desktop motion (`EffectTransition`, 120–140 ms ease-out) under a permanent animation policy (`cx.set_reduce_motion(false)`). Hardened reader layout clamping, empty archive handling, `.jfif` format support, responsive feedback toasts, and Vim layout commands (`d`, `s`, `:double`, `:single`, `:layout`).

### Phase 5: CBR/RAR Security, Hardening & Probing Optimization (Completed)
- **UnRAR C++ Audit:** Verified `unrar_sys 0.5.8` bundles official Eugene Roshal UnRAR 7.1.0 (May 2024), containing all upstream security fixes (CVE-2023-38831, CVE-2023-40477). Handle lifecycle managed via RAII `RARCloseArchive` drop semantics; operations strictly isolated to background executors.
- **Path Traversal Protection (`is_safe_archive_entry_name`):** Added centralized validator rejecting relative path components (`../`, `../../`), root paths (`/etc/passwd`), Windows drive letters and ADS (`C:\`, `foo:stream`), null bytes (`\0`), and hidden metadata across both CBZ and CBR pipelines.
- **Decompression Bomb & Encryption Hardening:** Added pre-decompression validation against `unpacked_size > MAX_PAGE_BYTES` (256 MiB), post-decompression streaming bounds checking, and clean detection of encrypted/password-protected entries.
- **$O(N^2) \to O(N)$ Probing Optimization:** Refactored RAR `probe_pages_dimensions` from $N$ repetitive archive reopenings to a single sequential extraction pass using a hash set of requested pages, reducing file opens from $N \to 1$ and entry advances from quadratic to linear.
- **Dedicated Regression Tests:** Added 5 new unit tests verifying safe entry names, traversal rejection, decompression bomb rejection, corrupted archive resilience, and single-pass probing with cooperative cancellation.

### Phase 6: Stage 2 CPU, Memory & Storage Optimization (Completed)
- **Batch Import Persistence Coalescing (`src/main.rs`):** Replaced per-book immediate serialization with debounced autosave (`save_library_debounced`), flushing reliably on batch finish, cancellation, shutdown, and worker disconnect. Eliminates hundreds of redundant full-file writes.
- **Crash-Resilient Persistence & Leftover Cleanup (`src/library.rs`):** Implemented temporary file isolation (`library.tmp.{pid}.{nanos}.{counter}`), `sync_all()`, atomic rename, Unix parent directory sync, and automated stale temp file cleanup on startup. Verified under concurrent multi-threaded saves.
- **Idle Theme Polling Elimination (`src/app/state.rs`, `src/ui/theme.rs`):** Replaced continuous full-file reads and hashing with metadata change detection (`len`, `mtime`, Unix `inode`), dropping idle read I/O to 0 bytes while preserving hot-reloading.
- **Zero-Copy Autosave Snapshots (`src/library.rs`, `src/app/reader_controller.rs`):** Wrapped `Book::page_dimensions` in `Arc<BTreeMap<String, (u32, u32)>>` with copy-on-write `insert_page_dimension`, making UI-thread library cloning $O(1)$ per book and eliminating 100,000+ map node clones.
- **Continuous Reader $O(1)$ Height & $O(\log N)$ Prefix Array (`src/ui/reader.rs`):** Replaced linear height accumulation loops with `ContinuousLayoutCache` prefix-sums and `partition_point` binary search, reducing scroll layout work by 99.7%.
- **Zero-Allocation Grouping & Search Filtering (`src/ui/library_view.rs`, `src/app/views/library.rs`, `src/archive.rs`):** Borrowed folder paths in `directory_groups`, eliminated redundant string allocations in search matching (`search::matches`), and optimized RAR entry name scans.
- **Dependency Hygiene (`Cargo.toml`):** Removed unused `memmap2 = "0.9"`.
- **7 New Regression Tests Added:** 3 unit tests in `src/library.rs` (durability, temp cleanup, failure preservation, concurrent saves), 3 UI integration tests in `src/tests.rs` (batch import coalescing, cancellation persistence, shutdown flush), and 1 theme metadata test in `src/ui/theme.rs`.

### Phase 7: Stage 3 Independent Verification & Scalability Profiling (Completed)
- **Persistence Verification & Durability Stress:** Benchmarked batch import write volumes across 100, 500, and 1,000 books, measuring 98.0% to 99.5% fewer disk writes and 92.4% to 98.5% reduced write volume. Verified crash durability, atomic rename, Unix parent directory sync, and automated cleanup of orphaned `library.tmp.*` files. Added large-collection durability flush regression test (`test_batch_import_large_collection_durability_and_flush`).
- **Theme Monitoring Syscall Audit:** Benchmarked stat queries (1.11 µs) vs content reads (3.38 µs). Confirmed 4,800 reads eliminated per idle hour (0 bytes read vs ~1.2 MiB). Added test coverage for same-size file updates and atomic replacement.
- **Memory Snapshot Latency Profiling:** Measured snapshot cloning speedup across 100 to 5,000 books, demonstrating 114.9x to 153.9x lower UI-thread latency (0.55 ms vs 84.98 ms for 5,000 books) and safe copy-on-write memory reclamation.
- **Continuous Reader Layout Invalidation & Fuzzy Verification:** Implemented 50-iteration randomized fuzzy comparison between `ContinuousLayoutCache` and naive linear accumulation, verifying 100% precision across mixed dimensions and zoom levels. Benchmarked layout queries across 100 to 10,000 pages (8.5x to 312x faster, with $O(1)$ total height taking 5.3 ns).
- **CBR/RAR Archive Safety & Concurrency Analysis:** Investigated UnRAR C++ backend handle constraints. Confirmed `unrar::OpenArchive` contains raw pointer `NonNull<unrar_sys::Handle>` which is strictly `!Send` and `!Sync`. Documented why shared handle pools are prohibited under `#![forbid(unsafe_code)]` and validated Yomu's single-pass entry header probing architecture.
- **Library Grid Scalability Benchmark:** Benchmarked render tree generation across 100 to 10,000 books (2.57 ms at 100 books to 388.97 ms at 10,000 books). Documented the then-current GPUI Kit 0.8.1 `v_virtual_list` limitation and recommendations for row virtualization under Rule 3.

### Phase 8: Stage 4 Library Scalability & Official GPUI Kit Row-Based Virtualization (Completed)
- **Official GPUI Kit Virtualization (`v_virtual_list`):** Implemented row-based 2D grid virtualization using `gpui_kit::component::v_virtual_list` and `VirtualListScrollHandle` without creating a custom second UI framework. Precomputed uniform row heights and paired with native `ScrollBounce` and `Scrollbar`.
- **Derived Row Partition Cache (`LibraryRowCache`):** Caches logical rows and item sizes with automatic invalidation on library mutations (`mutation_seq`), search query changes, sort options, folder expansion/collapse, view mode toggles, and responsive column rebalancing.
- **Measurable Scalability Breakthrough:** Reduced element construction time for 10,000 books from 388.97 ms to **0.05 ms (7,779x faster)**. Per-frame card instantiation dropped from 10,000 cards to **~35 cards (99.6% reduction)**, taking just 0.02 ms per scroll frame (0.3% of 144 Hz frame budget).
- **4 Dedicated Regression Tests Added:** Verified row chunking and partially filled rows (`test_virtual_grid_row_construction_and_partially_filled_rows`), search/sort invalidation (`test_virtual_grid_filtering_and_sorting_invalidation`), group expansion/collapse (`test_virtual_grid_group_expansion_and_collapse`), and list mode row partitioning (`test_virtual_grid_list_mode_row_partitioning`).

### Phase 9: Stage 5 Deep Memory Leak, Memory Management & SSD I/O Audit (Completed)
- **Deep Memory Lifecycle & Leak Audit:** Conducted exhaustive ownership tracing, task cancellation auditing, and dynamic heap profiling using Valgrind DHAT, Valgrind Massif, and Linux `/proc/self` telemetry. Measured peak live heap during active reading of just **3.96 MB** (4,218 blocks), with scratch decode/SIMD buffers immediately deallocated upon task completion.
- **Direct Book Switching Texture & Task Defect Isolated (Finding M1):** Identified that `open_book()` in `src/app/reader_controller.rs` dropped `self.reader` in-place without enqueuing previous `RenderImage` textures to `self.garbage` or signalling task cancellation tokens, causing GPU textures to remain allocated in the GPUI sprite atlas until application shutdown.
- **Unbounded GPUI Cover Asset Retention Isolated (Finding M2):** Identified that cover thumbnails rendered via `img(cover_path)` route into GPUI-pre 0.3.10 (at the time) asset cache without an LRU eviction policy or capacity cap, retaining ~5.4 GB of uncompressed textures across a 10,000-book library.
- **Autosave SSD Write Amplification Quantified (Finding S1):** Measured that updating reading progress by 8 bytes triggers full library JSON serialization, writing **16.58 MB** to disk with two `sync_all()` barriers for a 10,000-book library (**2,173,616x write amplification**; 1.66 GB written across 100 page turns).
- **CBR/RAR Quadratic Header Read Traversal Isolated (Finding S2):** Confirmed that reading a 200-page CBR chapter triggers $\approx 20,100$ sequential entry header reads and 200 archive open/close cycles due to `unrar::OpenArchive` being `!Send`.
- **Comprehensive Audit Deliverable Produced:** Authored [`docs/audits/MEMORY_LEAK_AND_SSD_IO_AUDIT.md`](audits/MEMORY_LEAK_AND_SSD_IO_AUDIT.md) providing full empirical findings, stress scenario tables, and a severity-ranked Stage 6 remediation roadmap.

### Phase 10: Stage 6A Memory Lifecycle & GPU Resource Remediation (Completed)
- **Test Inventory Reconciled:** Restored 10 regression tests and 9 benchmarks inadvertently truncated in Stage 5, elevating the verified test suite to 342 active tests before making functional changes.
- **Reader Lifecycle & GPU Texture Cleanup (Finding M1 Remediated):** Centralized reader teardown in `cleanup_current_reader()`, actively cancelling in-flight decode tasks, draining `RenderImage` instances via `reader.into_images()`, and scheduling immediate GPU sprite atlas release via `window.drop_image()` in `drain_garbage()`. Wired into `open_book`, `close_book`, `clear_all_data`, and `request_quit`.
- **Bounded Cover Asset Cache (Finding M2 Remediated):** Implemented `BoundedCoverCache` in `src/ui/cover_cache.rs` using standard library LRU tracking, implementing the native `gpui_kit::ImageCache` trait with a 256-cover capacity cap (~138 MB limit). Wrapped the two-pane shell via `gpui_kit::image_cache()`, automatically bounding cover texture memory and explicitly calling `cx.drop_image()` upon eviction.
- **Regression Test Coverage:** Added 4 focused regression tests verifying reader cancellation, late completion race condition immunity, cache clearing, and LRU capacity enforcement (346 active tests).
- **Comprehensive Technical Report:** Authored [`docs/audits/MEMORY_LIFECYCLE_REMEDIATION_STAGE_6A.md`](audits/MEMORY_LIFECYCLE_REMEDIATION_STAGE_6A.md).

### Phase 11: Stage 6B Persistence Architecture & SSD Write Amplification Remediation (Completed)
- **Dedicated Progress Sidecar (`progress.json`):** Created `src/progress.rs` implementing `ProgressStore` (v1) to decouple frequent reading-progress updates (`current_page`, `opened`, `last_read`) from static library metadata (`library.json`).
- **Transparent In-Memory Overlay & Automatic Migration:** `Library::load(&dirs)` automatically overlays `progress.json` records onto in-memory `Book` models on startup, preserving existing UI queries and zero-loss migration from older `library.json` files without progress sidecars.
- **Dual Dirty-State Detection:** Added monotonic `mutation_seq` tracking to both `Library` and `ProgressStore`. Background flush tasks skip serialization and disk I/O when state is unchanged.
- **Decoupled Debounced Autosave:** Routed active reading progress mutations to `save_progress_debounced()` (500 ms debounce, 2000 ms max delay bound). `library.json` is never written during active reading sessions.
- **Empirical Scalability Breakthrough:** In a 10,000-book collection, reduced single-save payload from 17.07 MB to 1.06 KB (99.994% reduction), reduced 100 page turns writes from 1.67 GB to 2.12 KB (99.99987% reduction), reduced CPU serialization time from 76.06 ms to 9.07 µs (8,386x speedup), reduced disk latency from 169.83 ms to 3.95 ms (43x speedup), and lowered write amplification from 2,184,324x to 272x.
- **Regression Test Coverage:** Added 6 dedicated regression tests and 2 performance benchmarks in `src/tests.rs` (353 active tests passing).
- **Comprehensive Technical Report:** Authored [`docs/audits/PERSISTENCE_AND_SSD_IO_REMEDIATION_STAGE_6B.md`](audits/PERSISTENCE_AND_SSD_IO_REMEDIATION_STAGE_6B.md).

### Phase 12: Stage 6C CBR/RAR Archive Access & I/O Optimization (Completed)
- **Worker-Confined Session Architecture (`RarSession`):** Confined `unrar::OpenArchive` to an OS worker thread spawned per active CBR/RAR book reader in `src/archive.rs`. Communicates over safe standard library channels (`std::sync::mpsc`), keeping `RarSession` `Send + Sync` in safe Rust without unsafe code or raw pointer wrappers.
- **Eliminated Quadratic Header Traversal:** Monotonically advances the UnRAR stream on forward page turns and prefetch queries, achieving $O(1)$ constant-time page access (~33.9 µs/page) without reopening the file. Tracks known entry positions and executes lazy rewind on backward navigation.
- **Empirical Verification (200-Page Archive):** Reduced total sequential extraction time from 230.66 ms to **6.79 ms (34x speedup)**, average time per page from 1.15 ms to **33.9 µs (34x speedup)**, archive opens from 200 to **1 (99.5% reduction)**, and header scans from 20,100 to **200 (99.0% reduction)**. Last-page extraction dropped from 2.08 ms to **29.3 µs (71x speedup)**.
- **Deterministic RAII Teardown:** Dropping `Reader::rar_session` in `cleanup_current_reader()` immediately disconnects the channel, causes the worker thread to close the UnRAR handle, and cleanly joins the thread.
- **Regression Test Coverage:** Added 6 unit tests and benchmarks in `src/archive.rs` and 2 UI tests in `src/tests.rs` (361 active tests passing).
- **Comprehensive Technical Report:** Authored [`docs/audits/RAR_ARCHIVE_ACCESS_OPTIMIZATION_STAGE_6C.md`](audits/RAR_ARCHIVE_ACCESS_OPTIMIZATION_STAGE_6C.md).

### Phase 13: Stage 7 Animation, Scrolling & Rendering Performance Optimization (Completed)
- **Library Grid Scroll Frame Invalidation Bypass:** Reordered `render_all_manga_view` to validate `LibraryRowCache` *before* sorting and query filtering. When scrolling or gliding through the library, sorting and `matches_query` passes are completely bypassed, achieving flat **0.010 ms** frame latency across 10,000 books (18,146x faster than recomputing).
- **Library Element ID Formatting Elimination:** Replaced runtime `format!` strings on book and folder cards with zero-allocation composite tuple IDs (`(ElementId::from("manga-item"), book.id.as_str())`), eliminating ~30,000 transient string allocations per second during 144 Hz scrolling.
- **Reader Frame Render Loop Allocation Elimination (`sync_reader`):** Added `wanted_scratch: Vec<(usize, u32)>` on `Reader` and stack chunking (`[(usize, u32); 32]`) for prefetch dispatch, completely eliminating heap allocations in the reader's frame tick.
- **Continuous Layout Cache In-Place Buffer Reuse & $O(1)$ Settle:** Updated `ensure_continuous_cache` to reuse `tops`, `heights`, and `bottoms` vector capacity in-place during zoom/resize; optimized `settle` to read arrival offsets directly from cache in sub-microsecond time.
- **Page Counter String Caching (`CachedPageCounter`):** Cached formatted counter string on `Reader`, reducing page string resolution to 3.9 ns with zero allocations during scrolling, zooming, and panning within the same page.
- **Appearance & Texture Drop Guards:** Optimized `sync_appearance` to early-return when theme mode is manual and opacity is set; guarded texture drop sweeps with `!self.garbage.is_empty()`.
- **Regression Test Coverage:** Added 3 dedicated tests and a benchmark in `src/tests.rs` (366 active tests passing).
- **Comprehensive Technical Report:** Authored [`docs/audits/ANIMATION_SCROLLING_RENDERING_OPTIMIZATION_STAGE_7.md`](audits/ANIMATION_SCROLLING_RENDERING_OPTIMIZATION_STAGE_7.md).

### Phase 14: Stage 8 Production-Readiness & Stability Audit (Completed)
- **Independent Full-Codebase Verification:** Audited all subsystems for memory lifecycle, GPU atlas deallocation, persistence durability, archive security, large-library reliability (10,000 books), and concurrency safety under safe Rust 2024 (`#![forbid(unsafe_code)]`).
- **Validation Matrix Execution:** Confirmed 100% pass rate across formatting (`cargo fmt`), strict clippy lints (`-D warnings`), 366 active tests, and 18 performance benchmarks.
- **Resource Discipline Verified:** Confirmed zero texture leaks in GPU atlas backends (`WgpuAtlasTextures`, `DirectXAtlasTextures`, `MetalAtlasTextures`) via `window.drop_image`, zero file descriptor leaks, and sub-1MB memory drift across 100 reader cycles.
- **Identified Production Risks:** Catalogued 1 Major issue (untrusted RAR archive decompression bomb vulnerability due to `unrar` 0.5.8 unbounded `ReadToVec` C++ callback), 2 Moderate issues (Windows atomic rename retry backoff, vendored UnRAR non-OSI license restriction), and 1 Minor issue (bounded cover cache counts entries rather than total bytes).
- **Release Qualification Verdict:** Classified Yomu as **CONDITIONALLY READY** (qualified for Release Candidate distribution; full release verified upon CBR decompression bomb patch and interactive desktop testing).
- **Comprehensive Audit Report:** Authored [`docs/audits/PRODUCTION_READINESS_AUDIT_STAGE_8.md`](audits/PRODUCTION_READINESS_AUDIT_STAGE_8.md).

### Phase 15: Stage 8A CBR/RAR Decompression-Bomb Protection & Extraction Hardening (Completed)
- **Vendored & Patched `unrar` Wrapper (`vendor/unrar`):** Audited native UnRAR 7.1.0 C++ engine and confirmed returning `-1` from `UCM_PROCESSDATA` cleanly executes `ErrHandler.Exit(RARX_USERBREAK)` and `ERAR_UNKNOWN` without cross-language panics or unwinding. Vendored `unrar` 0.5.8 via Cargo patch and added `BoundedBuffer` with `ReadBoundedToVec`.
- **Incremental Byte Accounting & Early Native Abort:** Tracks cumulative extracted bytes across incoming chunks with integer overflow checking (`checked_add`). If limit (`MAX_PAGE_BYTES = 128 MiB`) is exceeded, extraction aborts immediately inside native C++ via `-1` return code, and buffer memory is immediately freed via `clear()` and `shrink_to_fit()`.
- **Centralized Integration in Core Domain (`src/archive.rs`):** Wired `header.read_bounded(MAX_PAGE_BYTES as usize)` across `read_rar_page`, `RarSession` worker loop, and `probe_pages_dimensions`.
- **Zero Performance Degradation:** Sequential extraction overhead is undetectable (36.3 µs/page remediated vs 35.4 µs/page baseline).
- **6 Unit & Adversarial Tests Added:** Verified exact limit matching, 1-byte overflow abort, byte-spoofed local header abort, session worker recovery after extraction failure, repeated consecutive failures resilience, and integer overflow boundary conditions.
- **Comprehensive Technical Report:** Authored [`docs/audits/RAR_DECOMPRESSION_HARDENING_STAGE_8A.md`](audits/RAR_DECOMPRESSION_HARDENING_STAGE_8A.md).

### Phase 16: Stage 8B Windows Persistence Hardening & Dependency / Licensing Audit (Completed)
- **Windows Atomic Persistence Hardening (`src/library.rs`, `src/progress.rs`):** Implemented `atomic_replace` and `atomic_replace_with_retry` with exponential backoff (10 ms, 25 ms, 50 ms) targeting transient Windows lock errors (`ERROR_ACCESS_DENIED`, `ERROR_SHARING_VIOLATION`, `ERROR_LOCK_VIOLATION`) caused by background virus scanners or indexing services.
- **Fail-Safe Atomicity & Invariant Preservation:** The destination file is never deleted prior to replacement. Permanent errors abort immediately on attempt 0 with zero sleep delay. Leftover temporary files from prior crashes are automatically cleaned up on startup.
- **UnRAR Licensing & Distribution Audit:** Completed legal analysis of Alexander Roshal's UnRAR 7.1.0 freeware license and the Rust `unrar` wrapper. Verified full compliance for GitHub Releases, Flatpak, AppImage, macOS, and Windows MSI. Documented Debian/Fedora non-free repository requirements.
- **Bounded Cover Cache Review (`src/ui/cover_cache.rs`):** Verified that the 256-entry LRU cache reliably bounds memory to 138–297 MB across pre-scaled thumbnails without incurring blocking synchronous UI thread I/O or asynchronous loader race conditions.
- **Dependency Security Audit (`devenv.nix`, `.cargo/audit.toml`):** Added `pkgs.cargo-audit` to `devenv.nix` resolving the prior exit code 101. Audited all 829 crate dependencies against RustSec database. Confirmed that 4 reported advisories in `zip` 2.6.1 apply exclusively to `ZipArchive::extract` to disk (never used by Yomu). Configured `.cargo/audit.toml`, achieving a clean exit code 0 scan.
- **8 Dedicated Unit Tests Added:** Added tests covering first-time file creation, existing file replacement, mock transient retry success, retry exhaustion, permanent error fast-abort, and stale temporary file cleanup (378 active tests).
- **Comprehensive Technical Report:** Authored [`docs/audits/PRE_RELEASE_HARDENING_STAGE_8B.md`](audits/PRE_RELEASE_HARDENING_STAGE_8B.md).

### Phase 17: Stage 8D Comprehensive Memory Retention & Leak Investigation (Completed)
- **Application Heap Leak Disproven:** Evaluated live application memory lifecycle with Valgrind DHAT dynamic heap profiling. Verified that upon reader closure, total live heap allocations across the entire application were only 3.43 MiB (dominated by GPUI font glyph caches and UI render scratchpads), with zero bytes of decoded images or decompressed page buffers remaining.
- **glibc Allocator Arena Retention Identified:** Smaps analysis revealed that elevated RSS post-reader was held in seven 64 MiB (`0x4000000`-aligned) secondary thread arenas managed by glibc's `ptmalloc3` allocator, which failed to unmap or release free subheaps back to the OS.
- **Comprehensive Technical Report:** Authored [`docs/audits/MEMORY_RETENTION_INVESTIGATION_STAGE_8D.md`](audits/MEMORY_RETENTION_INVESTIGATION_STAGE_8D.md).

### Phase 18: Stage 8E Allocator Diagnosis & Comparative Benchmarking (Completed)
- **glibc Trimming Inefficiency Proven:** Built dynamic diagnostic helper (`diag_glibc_helper.so`) intercepting `SIGUSR1`. Proved that while `malloc_trim(0)` in glibc 2.44 reclaimed 25.3 MiB from Arena 0 (main `sbrk`), it reclaimed exactly 0 bytes from secondary arenas (Arenas 1–7) because tiny residual allocations pinned the subheap boundaries.
- **Three-Way Allocator Benchmark Matrix (Tests A–G):** Compared `glibc 2.44`, `tikv-jemallocator 0.6.1`, and `mimalloc 0.1.52` across real Wayland GUI sessions. Jemalloc demonstrated a 44% lower peak RSS (359.8M vs 640.4M), 46% lower post-close retention (344.5M vs 640.5M), zero memory creep across repeated cycles, and the fastest average page turn latency (162.7 ms).
- **Comprehensive Technical Report:** Authored [`docs/audits/ALLOCATOR_DIAGNOSIS_AND_COMPARISON_STAGE_8E.md`](audits/ALLOCATOR_DIAGNOSIS_AND_COMPARISON_STAGE_8E.md).

### Phase 19: Stage 8F Production Jemalloc Integration & Memory Remediation (Completed)
- **Target-Specific Global Allocator Integration:** Configured `tikv-jemallocator = "0.6.1"` strictly for Linux GNU targets (`cfg(all(target_os = "linux", target_env = "gnu"))`) in `Cargo.toml` and `src/main.rs`. Windows and macOS deliberately retain their platform-native system allocators.
- **NixOS Build Fortification Fix:** Configured `[profile.dev.package.tikv-jemalloc-sys]` and `[profile.test.package.tikv-jemalloc-sys]` with `opt-level = 2` to satisfy NixOS's `-D_FORTIFY_SOURCE` requirement across all profiles without modifying Nix packages or linker wrappers.
- **Native Symbol & Allocator Coverage Verification:** Verified embedded `_rjem_` symbols and `madvise(MADV_DONTNEED)` purge handling. Verified that 100% of Rust heap allocations (page vectors, SIMD resampler buffers, cover caches, GPUI views) route through jemalloc.
- **Production Wayland Benchmark Verification (Tests A–G):** Peak reading RSS dropped from 640.4 MiB to 484.9 MiB (-155.5 MiB / -24.3%); 300s post-close retention dropped from 640.5 MiB to 473.5 MiB (-167.0 MiB / -26.1%); direct switch memory dropped from 559.0 MiB to 423.5 MiB (-135.5 MiB / -24.2%); average page navigation latency remained identical at 164.6 ms (zero regression). All 380 active tests pass.
- **Comprehensive Technical Report:** Authored [`docs/audits/PRODUCTION_JEMALLOC_INTEGRATION_STAGE_8F.md`](audits/PRODUCTION_JEMALLOC_INTEGRATION_STAGE_8F.md).

### Phase 20: Stage 8F.1 Jemalloc Benchmark Reconciliation & Memory Stability Verification (Completed)
- **Stage 8E vs 8F Discrepancy Reconciled:** Side-by-side evaluation of the Stage 8E diagnostic binary (`yomu-jemalloc`) vs Stage 8F production binary (`target/release/yomu`) under identical clean-state conditions revealed virtually identical memory footprints (Peak: 412.0M vs 430.2M; Post-Close: 406.7M vs 427.7M). Proved that prior ~125 MiB variance was an artifact of test harness sequencing in Stage 8E and reading window sampling timing.
- **Extended 15-Cycle Memory Plateau Proven:** Conducted a 15-cycle consecutive reader soak test. Post-close RSS fluctuated strictly within a 395–445 MiB envelope with an average of 419.0 MiB. Cycle 15 closed RSS was 413.0 MiB (23.7 MiB lower than Cycle 1). Settled at 412.9 MiB across a 180s extended post-soak idle window, conclusively proving zero unbounded memory growth.
- **High-Sample Latency Profiling:** Evaluated 30 page-turn events, measuring Mean: 167.0 ms, Median: 165.8 ms, P95: 176.2 ms, P99: 177.7 ms with standard deviation <5 ms.
- **Comprehensive Technical Report:** Authored [`docs/audits/JEMALLOC_RECONCILIATION_STAGE_8F1.md`](audits/JEMALLOC_RECONCILIATION_STAGE_8F1.md).

### Phase 21: Library Import UX, Folder Grouping & UI Regression Fixes (Completed)
- **Dual Import Actions on Home Screen:** Replaced single generic import button on the empty Home/Continue Reading screen and Library empty state with two distinct, visually balanced actions: "Import Files" (`IconName::FilePlus`, primary) and "Import Folder" (`IconName::FolderPlus`, outline). Wired to native file and folder pickers with full cancellation safety.
- **Automatic Folder Volume Grouping:** Enhanced archive discovery in `src/library.rs` (`Candidate::Archive`, `Candidate::Folder`) to detect parent folder context. Volumes imported directly from a folder are assigned to a series group named after the folder (`Book.series`), while container directories with nested manga subfolders preserve their individual subfolder groupings without flattening. Single-volume series groups are supported in `src/ui/library_view.rs`. Re-importing retains existing metadata and avoids duplication.
- **Restored Toolbar Groups/Volumes Toggle:** Restored the 28px ghost grouping toggle button on the Library view toolbar (`show_grouping`). Bound to `is_group_series` with dynamic iconography (`IconName::Folder` vs `IconName::BookOpen`) and visual active state indication (`.selected(props.is_group_series)`). Controls presentation only without mutating underlying book data.
- **Fixed Settings About Icon Blue Tint:** Resolved blue color cast on the embedded Yomu logo (`assets/logo.png`) in `src/app/desktop.rs` by swapping Red and Blue color channels (`px.swap(0, 2)`) to match GPUI's native BGRA byte texture requirements.
- **6 New Regression Tests Added:** 3 unit tests (`test_app_logo_image_bgra_channel_order_and_no_blue_tint`, `test_folder_import_automatic_grouping_and_nested_structure`, `test_folder_import_empty_and_unsupported_files`) and 3 UI integration tests (`test_empty_library_import_actions_distinguishable_and_cancel_safe`, `test_library_toolbar_grouping_toggle_presentation_and_persistence`, `test_folder_import_automatic_grouping_and_reimport_behavior`) elevating the test suite to 386 active tests passing.
- **Comprehensive Technical Report:** Authored [`docs/audits/LIBRARY_IMPORT_GROUPING_AND_UI_REGRESSION_FIXES.md`](audits/LIBRARY_IMPORT_GROUPING_AND_UI_REGRESSION_FIXES.md).

### Phase 22: Library Selection State, Responsive Grid & Toolbar Consistency Refinement (Completed)
- **Group-Card Selection Derived from Members:** Added `GroupSelectionState` (`Unselected`, `PartiallySelected`, `FullySelected`) and `compute_group_selection_state` in `src/ui/library_view.rs`. Added `member_ids` to `LibraryItem::Folder`. Rendered `IconName::Check` checkmark badge for full selection and `IconName::Minus` indeterminate badge for partial selection. Preserved selection across entering and exiting groups.
- **Responsive Grid Space Utilization & Padding:** Refined `compute_responsive_grid` in `src/ui/components.rs` with rounded column sizing (`cols = ((avail_w + gap) / slot_w).round().max(1.0)`), bounded padding (`pad_x` 16–24px), restrained scaling within ±12%, and dynamic cover height strictly maintaining 1.5 aspect ratio. Eliminated dead horizontal space (116px outer margin).
- **Consistent Segmented Toolbar Mode Control:** Replaced isolated toggle button in `src/ui/toolbar.rs` with native GPUI Kit 0.8.1 (at the time) `ButtonGroup::new("grouping-mode-segmented").compact()` containing `mode-groups` (`IconName::Folder`) and `mode-volumes` (`IconName::BookOpen`) with `.selected(...)` styling and 28px height matching adjacent sort and view-mode controls.
- **4 New/Updated Tests Added:** Added 1 core library unit test (`test_select_many_and_deselect_many` in `src/selection.rs`), 3 binary UI integration tests (`test_group_card_selection_states_and_ctrl_a`, `test_responsive_grid_space_utilization_and_padding`, `test_toolbar_grouping_segmented_control_states` in `src/tests.rs`), and updated `entering_and_exiting_group_preserves_selection`. Elevated the test baseline to 390 active tests at that milestone.
- **Wayland Visual Verification:** Captured native Wayland screenshots demonstrating Ctrl+A full selection (`library_selection_ctrl_a_groups.png`), partial selection with indeterminate minus badge (`library_selection_partial_group.png`), responsive grid spacing (`library_groups_mode.png`), and active segmented control states (`crop_toolbar_groups.png` and `crop_toolbar.png`).
- **Comprehensive Technical Report:** Authored [`docs/audits/LIBRARY_SELECTION_GRID_AND_TOOLBAR_REFINEMENT.md`](audits/LIBRARY_SELECTION_GRID_AND_TOOLBAR_REFINEMENT.md).

---

## 3. Active Known Limitations & Technical Debt

1. **Wayland Compositor Visual Verification:**
   - Headless simulated GPUI tests validate layout, interaction, focus, and state transitions, but do not validate visual appearance (translucent surfaces, backdrop blur, compositor shadows) under physical Wayland compositors (e.g., Niri, Sway, GNOME Mutter, KDE KWin).
   - *Rule for agents:* Never claim visual verification on Wayland unless manually inspected on a real desktop session.
2. **Cross-Platform Momentum Scrolling:**
   - Linux and Windows rely on `PlatformScrollController`'s kinetic inertia glide loop because GPUI does not yet synthesize momentum events on non-macOS platforms.
   - *Action required:* Monitor official GPUI Kit releases for native upstream cross-platform momentum support.
3. **Physical Multi-Platform Testing:**
   - Linux tests run inside NixOS/devenv. Windows and macOS testing requires physical testing on native host environments.

---

## 4. Next Recommended Engineering Tasks

Refer to [docs/audits/PRE_RELEASE_HARDENING_STAGE_8B.md](audits/PRE_RELEASE_HARDENING_STAGE_8B.md) for the Stage 8B report, [docs/audits/RAR_DECOMPRESSION_HARDENING_STAGE_8A.md](audits/RAR_DECOMPRESSION_HARDENING_STAGE_8A.md) for Stage 8A, [docs/audits/PRODUCTION_READINESS_AUDIT_STAGE_8.md](audits/PRODUCTION_READINESS_AUDIT_STAGE_8.md) for Stage 8, and [ROADMAP.md](../ROADMAP.md):

1. **Stage 8C — Multi-Platform Packaging & Desktop Qualification:**
   - Verify AppImage, Flatpak manifest, Nix flake, and Windows MSI package recipes.
   - Perform interactive smoke testing on Wayland (Niri/Hyprland/GNOME Mutter), Windows 11, and macOS.
2. **CB7 / 7z Archive Support (Phase 7):**
   - Support `.cb7` and `.7z` archives using a pure Rust LZMA/7z decoder.
   - Maintain the standard archive interface and background thread isolation.

---

## 5. How to Inspect, Continue Work, and Report Results

### Inspecting the Codebase
- Use `src/lib.rs` and `src/AGENTS.md` for core domain models and algorithms.
- Use `src/ui/` and `src/ui/AGENTS.md` for reusable visual components and presentation screens.
- Use `src/app/` and `src/app/AGENTS.md` for state, routing, persistence, and event dispatch.
- Always check installed GPUI Kit versions in `Cargo.lock` before designing UI changes.

### Validating Changes
Run the complete validation matrix inside the development shell:

```bash
devenv shell cargo fmt --all -- --check
devenv shell cargo check --all-targets
devenv shell cargo clippy --all-targets --all-features -- -D warnings
devenv shell cargo test --all-targets
devenv shell cargo audit
```

### Reporting Standards
When completing a task, provide a concise, factual report:
1. Exact files modified, created, or deleted.
2. Architectural responsibilities changed.
3. Exact command exit codes and test counts (391 active tests).
4. Status of manual visual checks (explicitly report Wayland appearance as UNVERIFIED if tested headlessly).
