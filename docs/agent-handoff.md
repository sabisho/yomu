# Agent Handoff

This document is the active operational snapshot for coding agents working on Yomu. Read it after [AGENTS.md](../AGENTS.md) and [GEMINI.md](../GEMINI.md). For structural invariants and module boundaries, see [docs/architecture.md](architecture.md); for user-facing features and keybindings, see [README.md](../README.md).

---

## 1. Current Implementation Status

Yomu is a high-performance desktop manga and comic reader built in safe Rust 2024 using **GPUI Kit 0.8.1** (`gpui-pre = 0.3.10` runtime) under `#![forbid(unsafe_code)]`.

- **Primary Target:** Linux (Wayland-native with X11 fallback; Nix/devenv native packaging).
- **Secondary Targets:** macOS and Windows compatibility maintained through platform abstraction layers.
- **UI Architecture:** Modern macOS HIG two-pane design (persistent 240px sidebar, context-aware 52px toolbar, responsive content views, edge-to-edge immersive reader with native `ScrollbarMode::Scrolling` auto-hide scrollbars and boundary scroll bounce).
- **Test Baseline:** **308 active tests** passing (130 core library unit tests in `src/lib.rs`, 178 binary-target UI tests in `src/main.rs`, plus 3 opt-in performance benchmarks; 311 total).
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

---

## 3. Active Known Limitations & Technical Debt

1. **Wayland Compositor Visual Verification:**
   - Headless simulated GPUI tests validate layout, interaction, focus, and state transitions, but do not validate visual appearance (translucent surfaces, backdrop blur, compositor shadows) under physical Wayland compositors (e.g., Niri, Sway, GNOME Mutter, KDE KWin).
   - *Rule for agents:* Never claim visual verification on Wayland unless manually inspected on a real desktop session.
2. **Test File Size:**
   - `src/tests.rs` (~3,600 lines) contains 178 binary UI tests. Future maintenance may modularize test categories (e.g., navigation, persistence, reader, settings) without weakening coverage.
3. **Cross-Platform Momentum Scrolling:**
   - Linux and Windows rely on `PlatformScrollController`'s kinetic inertia glide loop because GPUI does not yet synthesize momentum events on non-macOS platforms.
   - *Action required:* Monitor official GPUI Kit releases for native upstream cross-platform momentum support.

---

## 4. Next Recommended Engineering Tasks

Refer to [ROADMAP.md](../ROADMAP.md) for full descriptions:

1. **Background Persistence Zero-Copy Snapshots (Phase 3.3):**
   - Eliminate deep cloning of entire `Library` vectors during debounced autosave by using lightweight serialization diffs or immutable snapshots.
2. **Dependency & Vendor Hygiene:**
   - Remove unused dependency `memmap2` from `Cargo.toml`.
   - Resolve the `Selectable` trait re-export cleanly and assess removing `vendor/gpui-component`.
3. **CB7 / 7z Archive Support (Phase 5):**
   - Support `.cb7` and `.7z` comic archives alongside CBZ, CBR, ZIP, and RAR using a pure Rust 7z reader.

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
```

### Reporting Standards
When completing a task, provide a concise, factual report:
1. Exact files modified, created, or deleted.
2. Architectural responsibilities changed.
3. Exact command exit codes and test counts (308 active tests).
4. Status of manual visual checks (explicitly report Wayland appearance as UNVERIFIED if tested headlessly).
