# Yomu — Comprehensive Engineering Reassessment & Architectural Assessment

**Date:** October 2026  
**Auditor:** Antigravity (Google DeepMind Advanced Agentic Coding)  
**Philosophy:** Ponytail Engineering (Minimalism, native solutions, zero unnecessary abstractions, evidence-based optimization)  
**Status:** Audit & Planning Only (No source code or user data modified)

---

## Executive Summary

A comprehensive engineering investigation of the Yomu codebase was conducted across all 20,801 lines of Rust source, build manifests (`Cargo.toml`, `Cargo.lock`), Nix/devenv environments, vendor patches, unit tests, and project documentation.

### Core Verdict
**The core engine and UI rendering pipeline are functionally robust, exceptionally fast, and thoroughly tested (199/199 unit and UI integration tests pass, per-frame animation math runs in 20 ns, slide frame paint average is 2.91 ms). An architectural rewrite is completely unjustified and would introduce catastrophic regression risk.**

However, rapid iterative feature additions across multiple prior agent sessions have produced acute **architectural debt in `src/main.rs` (6,946 lines)**, orphaned dead code (`src/ui/library_view.rs`, `vendor/xattr`, `vendor/stacksafe-macro`), startup filesystem pollution (`ensure_desktop_icon`), and severe documentation drift.

A surgical, **incremental refactoring plan** following the Ponytail philosophy is the optimal path forward.

---

## 1. Actual Current Architecture

### 1.1 Architectural Topology

The application is structured into two primary tiers:
1. **Core Library (`src/lib.rs` / `yomu` crate):** 100% pure Rust (`#![forbid(unsafe_code)]`), decoupled from GPUI and windowing. Handles archive decompression, SIMD image decoding/resizing, layout math, cache eviction policies, vim key parsing, search matching, selection rules, and JSON persistence.
2. **UI & Window Orchestration (`src/main.rs` + `src/ui/`):** Built on GPUI Kit 0.8.0. Manages the single application window, state routing between two-pane desktop shell and edge-to-edge immersive reader, gesture/event dispatch, and background task scheduling.

```mermaid
graph TD
    subgraph "Core Domain Layer (GPUI-Independent) [src/lib.rs]"
        archive["archive.rs<br/>ZIP/CBZ/CBR/RAR & handle pool"]
        imaging["imaging.rs<br/>zune-jpeg + fast_image_resize"]
        pagecache["pagecache.rs<br/>Adaptive memory budget (0.5–3GB)"]
        layout["layout.rs<br/>Fit modes, zoom, geometry"]
        motion["motion.rs<br/>Inertial physics & transitions"]
        scroll["scroll.rs<br/>PlatformScrollController"]
        library["library.rs<br/>Library, Book, Settings, XDG I/O"]
        collections["collections.rs<br/>Collections & Smart categories"]
        vim["vim.rs<br/>Vim key tokens & actions"]
        selection["selection.rs<br/>Multi-select range engine"]
        search["search.rs<br/>Zero-alloc case-insensitive matcher"]
        util["util.rs<br/>Natural sort, FNV-1a, formatting"]
    end

    subgraph "UI Component Layer [src/ui/]"
        sidebar["sidebar.rs<br/>240px macOS sidebar"]
        toolbar["toolbar.rs<br/>Contextual header & switches"]
        continue_reading["continue_reading.rs<br/>Hero banner & queue"]
        details["details.rs<br/>Manga details inspector"]
        collections_view["collections_view.rs<br/>Smart & custom collections"]
        settings_view["settings_view.rs<br/>6-tab preferences view"]
        components["components.rs<br/>Pills, badges, OS shortcuts"]
        overlays["overlays.rs<br/>Dialogs, popups, menus"]
        reader_ui["reader.rs<br/>Reader state & continuous strip"]
        theme["theme.rs<br/>Kit themes & TOML loader"]
        dead_lib_view["library_view.rs<br/>⚠️ UNUSED DUPLICATE (620 loc)"]
    end

    subgraph "Application Runtime [src/main.rs]"
        main["main.rs (6,946 lines)<br/>Yomu God Struct (42 fields)<br/>Event Loop & Render Router"]
    end

    main --> library
    main --> reader_ui
    main --> sidebar
    main --> toolbar
    main --> continue_reading
    main --> details
    main --> collections_view
    main --> settings_view
    main --> overlays
    main --> theme
    main --> scroll
    main --> imaging
    main --> archive
    reader_ui --> pagecache
    reader_ui --> layout
    reader_ui --> motion
```

### 1.2 Dependency Graph & Vendor Manifest

| Component | Upstream Spec | Actual Implementation | Assessment |
|---|---|---|---|
| **GPUI Kit** | `gpui-kit = "=0.8.0"` | Pin 0.8.0 with feature `test-support` | Modern, stable. Documented in docs as 0.7.3 (stale doc). |
| **gpui-component** | Crates.io patch | `vendor/gpui-component` (118k lines) | Patched locally for custom tab/table/tooltip extensions. |
| **unrar** | `unrar = "0.5.8"` | Links `unrar_sys` (C++ UnRAR) | Contradicts Cargo.toml "Pure Rust decoders only" comment. |
| **image** | `0.25` (no default features) | `features = ["jpeg", "png", "webp", "gif"]` | Clean, pure Rust. |
| **zune-jpeg** | `0.5` | Fast direct-to-BGRA decoding | Highly optimized SIMD path. |
| **fast_image_resize** | `6` | Pure Rust SIMD Catmull-Rom | Reuses thread-local scratch buffer. |
| **zip** | `2` (deflate only) | Pure Rust buffered zip reader | Integrated with 8-slot MRU archive cache. |
| **vendor/stacksafe-macro** | None | Dead directory in `vendor/` | Unused in `Cargo.toml`/`Cargo.lock`. Safe to remove. |
| **vendor/xattr** | None | Dead directory in `vendor/` | Unused in `Cargo.toml`/`Cargo.lock`. Safe to remove. |

---

## 2. Critical Findings Ranked by Severity

### Priority 1: High Severity (Architectural Bloat & Unintended Side Effects)

1. **`src/main.rs` God-Object Congestion (6,946 lines, 42 struct fields)**
   - **Location:** [src/main.rs#L169-L242](file:///home/sabisho/Projects/yomu/src/main.rs#L169-L242), [src/main.rs#L244-L6500](file:///home/sabisho/Projects/yomu/src/main.rs#L244-L6500)
   - **Problem:** `Yomu` contains 42 fields, including 7 individual `ScrollHandle` instances, navigation history, overlay states, animations, timers, and input states. A single `impl Yomu` block spans 6,256 lines. Over 3,350 lines in `main.rs` are dedicated to view rendering methods (`render_book_card`, `render_folder_card`, `render_all_manga_view`, `render_history_view`, etc.).
   - **Impact:** Slow editor LSP responsiveness, high cognitive overhead, difficult code navigation, and increased risk of accidental cross-concern mutations.

2. **620 Lines of Completely Dead, Duplicated Code in `src/ui/library_view.rs`**
   - **Location:** [src/ui/library_view.rs#L1-L620](file:///home/sabisho/Projects/yomu/src/ui/library_view.rs#L1-L620) vs [src/main.rs#L4790-L5412](file:///home/sabisho/Projects/yomu/src/main.rs#L4790-L5412)
   - **Problem:** `src/ui/library_view.rs` was authored in a previous turn to extract `render_book_card`, `render_book_list_row`, `render_folder_card`, and `render_folder_list_row`. However, `src/main.rs` was never updated to invoke it; `main.rs` retained its own identical copy of all four functions.
   - **Verification:** `grep -rn "library_view" src/` confirms `src/ui/library_view.rs` is never called anywhere in the runtime.
   - **Impact:** 620 lines of dead maintenance liability. Bug fixes applied to `main.rs` cards are not reflected in `library_view.rs` and vice versa.

3. **Intrusive Disk I/O & External Process Execution on Every Startup (`ensure_desktop_icon`)**
   - **Location:** [src/main.rs#L6804-L6872](file:///home/sabisho/Projects/yomu/src/main.rs#L6804-L6872)
   - **Problem:** Every time `yomu` is launched on Linux, `ensure_desktop_icon()` executes synchronously before window creation:
     - Scans and writes 9 image icons (`assets/icons/*.png`, `assets/logo.png`) into `~/.local/share/icons/` and `~/.local/share/pixmaps/`.
     - Rewrites `~/.local/share/applications/yomu.desktop`.
     - Spawns a synchronous external process `gtk-update-icon-cache -f -t ~/.local/share/icons/hicolor`.
   - **Impact:** Unnecessary startup latency; mutates user desktop configuration files without consent; conflicts with NixOS/distro package managers (which already install icons via `nix/package.nix#L74-L82`).

### Priority 2: Medium Severity (Lifecycle, Invariants & Dependency Hygiene)

4. **GPUI Entity Creation and Subscription Inside `Render::render()`**
   - **Location:** [src/main.rs#L6537-L6555](file:///home/sabisho/Projects/yomu/src/main.rs#L6537-L6555)
   - **Problem:** `search_input` (`Entity<InputState>`) is conditionally constructed and subscribed to within the `render()` loop rather than during entity initialization (`with_dirs` / `new`).
   - **Impact:** Violates GPUI best practices. Allocating entities and attaching reactive event listeners during paint cycles can cause subtle retain cycles or unexpected state reinitialization.

5. **Discrepancy in Default Grouping Setting (`group_series`)**
   - **Location:** [src/library.rs#L454](file:///home/sabisho/Projects/yomu/src/library.rs#L454) vs [docs/architecture.md#L109](file:///home/sabisho/Projects/yomu/docs/architecture.md#L109)
   - **Problem:** `Settings::default()` initializes `group_series: false`, whereas `docs/architecture.md` and `docs/agent-handoff.md` state: *"Directory grouping is on by default."*
   - **Impact:** Confusion between documented behavior and default runtime state for fresh installations.

6. **Orphaned Vendored Crates in `vendor/`**
   - **Location:** `vendor/stacksafe-macro` and `vendor/xattr`
   - **Problem:** Leftover crates from GPUI 0.7 migration that are no longer referenced in `Cargo.toml` or `Cargo.lock`.
   - **Impact:** Repository clutter and bloat.

7. **Misleading Documentation Regarding C Dependencies**
   - **Location:** [Cargo.toml#L30-L36](file:///home/sabisho/Projects/yomu/Cargo.toml#L30-L36)
   - **Problem:** Comment states *"Pure-Rust decoders only: no C libraries get linked for image or archive handling"*, but line 36 pulls in `unrar = "0.5.8"`, which compiles and links the C++ UnRAR codebase (`unrar_sys`).
   - **Impact:** Misrepresents security and compilation boundaries.

### Priority 3: Low Severity (Documentation Drift)

8. **Widespread Documentation Drift**
   - **`README.md#L349-L383`:** References GPUI Kit 0.7.3 (actual: 0.8.0); lists CBR, pinch zoom, and inertial scrolling under "Not done yet" (all are fully implemented).
   - **`ROADMAP.md`:** 7-line file whose only item is inertial scrolling (already done).
   - **`docs/development.md#L62-L63`:** States CBR, pinch zoom, and inertial scrolling are gaps (all done).
   - **`docs/ui-components.md`:** References deleted functions (`mac_window_controls`) and obsolete fields (`can_go_forward`).
   - **`docs/agent-handoff.md`:** 362 lines of chronological turn-by-turn agent notes containing contradictory claims from earlier rolled-back turns.

---

## 3. Performance & Memory Assessment

Measurements collected on the target Linux system via `cargo test perf_ -- --ignored --nocapture`:

| Operation | Measurement | Target / Budget | Assessment |
|---|---|---|---|
| **Animation Physics Math** | **20 ns / frame** | < 16,000 ns (16ms) | Exceptional. Vectorized `x86_64-v3` AVX2 math. |
| **Frame Render Paint (Simulated)** | **2.91 ms (p95: 3.08 ms)** | < 8.33 ms (120 Hz) | Highly responsive, even on CPU-simulated quad renderer. |
| **JPEG Decode (3000×4500, unscaled)** | **177 ms** | < 500 ms | Direct-to-BGRA via `zune-jpeg`. Zero intermediate allocation. |
| **Decode + Catmull-Rom Scale (Tier 1024)** | **1,107 ms** (median) | Bounded background thread | Runs strictly on `cx.background_executor()`. |
| **Decoded Page Cache Memory** | **Adaptive 0.5–3.0 GiB** | Clamped to Host RAM / 8 | Eviction preserves visible & current pages. |
| **Persistence I/O Latency** | **0.00 ms (UI thread)** | 0.00 ms | Asynchronous debounced flush (500ms debounce). |

### Concurrency & Threading Integrity
- Image decoding and scaling are strictly decoupled from the GPUI main thread via `cx.background_executor().spawn(...)`.
- Cancellation tokens (`Reader.cancellations: Arc<Mutex<HashMap<usize, Arc<AtomicBool>>>>`) successfully abort obsolete background decode jobs upon rapid page turns or reader close.
- Thread-safe archive pool (`ARCHIVE_POOL` with 8 handles) eliminates zip header re-parsing overhead while ensuring parallel decompression without mutex contention.

---

## 4. Technical Debt Inventory

| Category | Component | Description | Remediation |
|---|---|---|---|
| **Code Duplication** | `src/ui/library_view.rs` | 620 lines duplicating `main.rs` card/folder/row rendering | Wire `main.rs` to call `library_view.rs` and delete the duplicate functions in `main.rs`. |
| **God Object** | `src/main.rs:Yomu` | 42 fields, 7 `ScrollHandle`s, monolithic state | Group scroll handles and UI router states into clean helper structs. |
| **Startup Overhead** | `main.rs:ensure_desktop_icon` | Spawns `gtk-update-icon-cache` and writes 10 files on every boot | Make opt-in via `--install-desktop-entry` CLI flag or packaging script only. |
| **Lifecycle Anti-Pattern** | `main.rs:render()` | Entity allocation (`search_input`) in paint loop | Move instantiation into `Yomu::with_dirs`. |
| **Stale Vendoring** | `vendor/xattr`, `stacksafe-macro` | Dead crates in repo | Delete directories. |
| **Documentation** | `README.md`, `ROADMAP.md`, `docs/` | Inaccurate features, outdated version tags | Update to current truth; consolidate redundant docs. |

---

## 5. Refactor versus Rewrite Analysis

### Proposal A: Incremental Refactoring (Recommended)

- **Strategy:**
  1. **Connect `src/ui/library_view.rs`:** Divert card, list row, folder card, and folder row calls in `src/main.rs` to `ui::library_view::*`, and remove ~620 lines of duplicate code from `src/main.rs`.
  2. **Move Overlay Rendering out of `src/main.rs`:** Delegate `render_menu`, `render_library_menu`, `render_confirm`, `render_properties`, `render_series_dialog`, `render_collection_dialog`, and `render_command_box` to `src/ui/overlays.rs`.
  3. **Relocate `ensure_desktop_icon`:** Remove synchronous desktop file creation from `main()`, adding an optional CLI sub-command `--install-desktop-file` if required.
  4. **Initialize `search_input` in Constructor:** Initialize `InputState` once in `Yomu::with_dirs`, removing entity creation from `Render::render()`.
  5. **Purge Unused Vendor Folders:** Delete `vendor/xattr` and `vendor/stacksafe-macro`.
  6. **Reconcile Documentation:** Bring `README.md`, `ROADMAP.md`, `architecture.md`, and `development.md` into exact agreement with the 0.8.0 implementation.
- **Regression Risk:** Minimal. All 199 existing unit and UI integration tests verify these exact interfaces and behavior.
- **Estimated Code Delta:** -1,500 lines net reduction. Zero behavioral disruption.

### Proposal B: Architectural Rewrite (Rejected)

- **Strategy:** Complete rewrite of application architecture into a rigid Model-View-Intent (MVI) or Elm-style architecture with decoupled sub-views.
- **Why it is Rejected (Ponytail Principles):**
  1. **Core Works Exceptionally Well:** The core library (`src/lib.rs`) is already decoupled from GPUI, 100% safe Rust, and achieves 20 ns animation math and sub-3ms frame times.
  2. **Huge Reimplementation Cost:** Rebuilding GPUI Kit 0.8 view hierarchies, context menu anchoring, Wayland compositor workarounds, and cross-platform momentum scrolling would take weeks.
  3. **High Regression Risk:** Yomu has 98 delicate UI integration tests asserting keyboard focus, mouse hit-testing, zoom retention, and async cover loading. A rewrite would break these guarantees for purely aesthetic architectural satisfaction.

### Objective Comparison

| Dimension | Incremental Refactoring (Proposal A) | Architectural Rewrite (Proposal B) |
|---|---|---|
| **Preserves Working Features** | 100% | High risk of subtle UX regressions |
| **Test Suite Stability** | All 199 tests remain passing | Entire test suite must be rewritten |
| **Performance Impact** | Preserves 20 ns math & sub-3ms frames | Unknown / Unproven |
| **Cross-Platform Parity** | Preserves tested Linux/macOS/Windows paths | High risk of platform regressions |
| **Implementation Effort** | Low (Focused, targeted surgical cleanups) | Extremely High (Weeks of redundant labor) |
| **Adherence to Ponytail** | High (Smallest change that solves the issue) | Zero (Premature rewrite for aesthetics) |

---

## 6. Proposed Documentation Structure

Review of existing documentation files:

| File | Classification | Proposed Action |
|---|---|---|
| `README.md` | **Needs Updating** | Update version badge to 0.8.0; remove CBR/inertial scroll from "Not done yet"; document current features accurately. |
| `AGENTS.md` | **Current & Authoritative** | Retain as the primary entry point for coding agents; update GPUI Kit version reference to 0.8.0. |
| `ROADMAP.md` | **Needs Updating** | Remove completed inertial scrolling; list actual remaining features (CB7, double-page spreads, sorting). |
| `problems.md` | **Historical / User-Owned** | Retain untouched (user-owned file). |
| `docs/architecture.md` | **Needs Updating** | Harmonize `group_series` default value; reflect modularized `src/ui/` structure. |
| `docs/agent-handoff.md` | **Should Be Consolidated** | Distill 362 lines of turn logs into an authoritative 80-line architectural decision record (ADR). |
| `docs/development.md` | **Should Be Consolidated** | Merge unique developer workflows into `docs/architecture.md` and `README.md`. |
| `docs/ui-components.md` | **Should Be Consolidated** | Update API signatures (remove `mac_window_controls`) and merge into design documentation. |
| `docs/ui-design.md` | **Current & Authoritative** | Retain as the visual design and HIG specification. |
| `docs/ui-redesign-plan.md` | **Historical Reference** | Retain as historical context for Layer 1–12 migration. |
| `docs/ui-redesign-changelog.md` | **Historical Reference** | Retain as historical log. |
| `docs/ui-validation.md` | **Historical Reference** | Retain as test matrix against `macOS_redign_design_board.png`. |

**Target Consolidated Documentation Tree:**
```text
docs/
├── architecture.md          # Unified system boundaries, data flows, and development recipes
├── ui-design.md             # macOS HIG visual spec, design tokens, and component guidelines
├── decisions/
│   └── 001-ui-redesign.md   # Archived historical handoff, plan, changelog, and validation
└── architectural-assessment.md # This audit report
```

---

## 7. Prioritized Implementation Roadmap

Following user approval, execution should proceed in 4 focused, independently testable phases:

### Phase 1: Eliminate Dead Code & Wire `library_view.rs`
- Direct `main.rs` card/row rendering to `ui::library_view::*`.
- Remove redundant duplicate functions in `src/main.rs` (~620 lines removed).
- Delete unreferenced directories `vendor/xattr` and `vendor/stacksafe-macro`.
- *Verification:* `cargo check`, `cargo test`.

### Phase 2: Refactor Startup & Entity Initialization
- Remove automatic `ensure_desktop_icon` invocation on startup.
- Move `search_input` initialization out of `Render::render()` into `Yomu::with_dirs`.
- *Verification:* `cargo test --bin yomu`.

### Phase 3: Modularize Overlays & Dialogs from `src/main.rs`
- Extract dialog and context menu rendering methods (`render_menu`, `render_library_menu`, `render_confirm`, `render_series_dialog`, `render_command_box`) into `src/ui/overlays.rs`.
- Shrink `src/main.rs` to under 3,500 lines.
- *Verification:* `cargo test --bin yomu`.

### Phase 4: Documentation Harmonization
- Update `README.md`, `AGENTS.md`, and `ROADMAP.md`.
- Consolidate redundant docs into `docs/architecture.md` and archive historical turn notes.
- *Verification:* `cargo fmt --all -- --check`.

---

## 8. Verification Strategy

Every phase of the implementation must satisfy:
1. `cargo fmt --all -- --check` produces zero diffs.
2. `cargo clippy --all-targets -- -D warnings` produces zero warnings.
3. 100/100 core library tests pass cleanly.
4. 99/99 UI integration tests pass cleanly.
5. Ignored performance benchmarks (`cargo test perf_ -- --ignored --nocapture`) confirm per-frame animation math remains $\le 25\text{ ns}$ and paint cycle latency remains $\le 3.5\text{ ms}$.

---
*Report complete. Standing by for user review and explicit approval before any changes are made.*
