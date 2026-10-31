# Agent Handoff

This document is the current operational snapshot for coding agents working on Yomu. Read it after [AGENTS.md](../AGENTS.md). For structural invariants and module boundaries, see [docs/architecture.md](architecture.md); for user-facing features and keybindings, see [README.md](../README.md).

---

## 1. Current Implementation Status

Yomu is a high-performance desktop manga and comic reader built in Rust 2024 using **GPUI Kit 0.8.1** (`gpui-pre = 0.3.10` runtime). It operates with `#![forbid(unsafe_code)]` in safe Rust.

- **Primary Target:** Linux (Wayland-native with X11 fallback; Nix/devenv native packaging).
- **Secondary Targets:** macOS and Windows compatibility maintained through abstraction layers.
- **UI Architecture:** Modern macOS HIG two-pane design (persistent 240px sidebar, context-aware 52px toolbar, responsive content views, edge-to-edge immersive reader with native `ScrollbarMode::Scrolling` auto-hide scrollbars and boundary scroll bounce).
- **Test Baseline:** **239 active tests** passing (106 core library unit tests in `src/lib.rs`, 133 binary-target UI tests in `src/main.rs`, plus 3 opt-in performance benchmarks; 242 total).
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
   - **Phase 2E (Next):** Remaining Phase 2E structural cleanup and window lifecycle orchestration.
3. **Phase 3: Scalability & Performance:**
   - Implement $O(1)$ lookup indexing for `Library` with standard library collections.
   - Eliminate deep library cloning on background autosave.
4. **Phase 4: Dual-Page Spread Reading Mode:**
   - Side-by-side portrait page display with landscape spread auto-fallback.
