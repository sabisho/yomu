# Agent Handoff

This document is the current operational snapshot for coding agents working on Yomu. Read it after [AGENTS.md](../AGENTS.md). For structural invariants and module boundaries, see [docs/architecture.md](architecture.md); for user-facing features and keybindings, see [README.md](../README.md).

---

## 1. Current Implementation Status

Yomu is a high-performance desktop manga and comic reader built in Rust 2024 using **GPUI Kit 0.8.1** (`gpui-pre = 0.3.10` runtime). It operates with `#![forbid(unsafe_code)]` in safe Rust.

- **Primary Target:** Linux (Wayland-native with X11 fallback; Nix/devenv native packaging).
- **Secondary Targets:** macOS and Windows compatibility maintained through abstraction layers.
- **UI Architecture:** Modern macOS HIG two-pane design (persistent 240px sidebar, context-aware 52px toolbar, responsive content views, edge-to-edge immersive reader with native `ScrollbarMode::Scrolling` auto-hide scrollbars and boundary scroll bounce).
- **Test Baseline:** **202 active tests** passing (100 core library unit tests in `src/lib.rs`, 102 binary-target UI tests in `src/main.rs`, plus 3 opt-in performance benchmarks; 205 total).
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

1. **Phase 1: Critical Fixes & Reliability:**
   - Resolve prefetch cancellation defect in `page_decoded`.
   - Cache `/proc/meminfo` memory budget via `OnceLock`.
   - Bound initial allocation capacity in `archive::read_zip_page`.
2. **Phase 2: Architectural Modularization:**
   - Relocate `scroll.rs` to `src/ui/scroll.rs`.
   - Extract `src/app/` subsystems from `src/main.rs`.
3. **Phase 3: Scalability & Performance:**
   - Implement $O(1)$ lookup indexing for `Library` with standard library collections.
   - Eliminate deep library cloning on background autosave.
4. **Phase 4: Dual-Page Spread Reading Mode:**
   - Side-by-side portrait page display with landscape spread auto-fallback.
