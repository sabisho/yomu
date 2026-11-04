# Codebase Simplification Implementation Report

**Date:** 2026-11-05  
**Basis:** The comprehensive audit, simplification roadmap, and native GPUI inventory in this directory.  
**Scope:** Controlled implementation of stages 1–5. No dependency, allocator, Nix configuration, archive, reader, scroll-controller, persistence, or user-data changes were made.

## Summary

Completed documentation correction, exact GPUI Kit 0.9.0 vendor reconciliation, a temporary native Settings prototype, and cover-cache consolidation. The Settings prototype showed the Kit API works, but replacing Yomu's existing native sidebar and custom settings callbacks did not demonstrate a simpler complete implementation, so it was not adopted. The bounded LRU and virtual library row cache remain unchanged.

The cache work also corrected a resource-lifecycle defect found while tracing reset-all: `clear_without_window()` used to drop cache entries without asking GPUI to release their textures. It now calls `App::drop_image` for each loaded image.

## Stage status

| Stage | Status | Result |
|---|---|---|
| 1 — Documentation accuracy | Complete | Current-facing framework and test-baseline documentation updated |
| 2 — Vendor reconciliation | Complete | One redundant export removed; required popup and scroll patches retained and documented |
| 3 — Native Settings evaluation | Prototype complete; adoption deferred | Headless Appearance prototype passed; no full-screen replacement justified |
| 4 — Cover-cache simplification | Complete | Eviction/release paths consolidated; reset-all now releases GPU images |
| 5 — Evidence-based investigations | Partial | A-14 cached render path measured; invalidation allocations not profiled. A-10 remains unchanged |
| 6 — Optional refinements | Deferred | No upstream API or performance evidence justified further changes |

## Findings and disposition

### A-11 — Documentation accuracy — Completed

Updated current-facing framework references to GPUI Kit 0.9.0 and `gpui-pre` 0.3.11 in [AGENTS.md](/home/sabisho/Projects/yomu/AGENTS.md), [GEMINI.md](/home/sabisho/Projects/yomu/GEMINI.md), [src/ui/AGENTS.md](/home/sabisho/Projects/yomu/src/ui/AGENTS.md), [docs/architecture.md](/home/sabisho/Projects/yomu/docs/architecture.md), and [docs/agent-handoff.md](/home/sabisho/Projects/yomu/docs/agent-handoff.md).

The architecture guide's theme description now says content-fingerprint reload rather than SHA-256: the implementation uses `DefaultHasher`. Historical milestone entries retain the framework version used at that time. Current test counts now reflect observed results: 171 library tests and 220 UI tests pass, with 16 ignored performance benchmarks (391 active, 407 total). [docs/development.md](/home/sabisho/Projects/yomu/docs/development.md) has matching target-specific counts.

### A-01 — GPUI Kit vendor patch reconciliation — Completed

Compared the checked-in vendor tree with the official crates.io `gpui-component` 0.9.0 archive. The original source differed in exactly three files: `src/lib.rs`, `src/menu/popup_menu.rs`, and `src/scroll/scrollable.rs`.

- **Removed:** The explicit `Selectable` root export in `vendor/gpui-component/src/lib.rs`. Upstream `pub use styled::*` already exposes `Selectable`; the application compiled and the full suite passed without the duplicate export.
- **Retained:** `Scrollable::track_scroll` and `Scrollable::scroll_wheel` in `vendor/gpui-component/src/scroll/scrollable.rs`. The upstream 0.9.0 source has neither method. Live Yomu call sites share scroll handles with the scrollbar/controller and forward wheel events to the Linux/Windows momentum controller.
- **Retained:** The `PopupMenu` debug selector in `vendor/gpui-component/src/menu/popup_menu.rs`. Removing it caused `right_click_opens_the_menu_and_never_opens_the_book` to fail at `debug_bounds("popup-menu")`; restoring it made that test pass. The upstream `.id("popup-menu")` does not supply GPUI's test debug selector.
- **Documented:** [vendor/gpui-component/YOMU_PATCHES.md](/home/sabisho/Projects/yomu/vendor/gpui-component/YOMU_PATCHES.md) lists the remaining additions, their callers, and the comparison steps for the next upgrade.

After reconciliation, the only vendor source differences from upstream are the popup selector and the two scroll methods in `scrollable.rs`.

### A-02 — Native Settings evaluation — Prototype completed; adoption deferred

Built and ran a temporary headless Appearance page using GPUI Kit 0.9.0 `Settings`, `SettingPage`, `SettingGroup`, and `SettingItem`, with two groups and an accessible native `Switch`. The focused prototype test rendered the page and verified that the switch callback updated its owner. The temporary test code was removed after evaluation; no product settings behavior changed.

The Kit `Settings` component supplies its own searchable sidebar and active-page/group navigation. Its `SettingItem::render` path can host custom controls and callbacks, while typed fields can use controlled getter/setter closures. Yomu already uses the native `Sidebar`, `Switch`, and dropdown controls; its appearance rows include Yomu-specific choices and persistence callbacks. A replacement would still need those custom controls/adapters and would change the settings navigation shell. The prototype did not establish lower implementation complexity or a behavior-equivalent migration, so the current view remains.

**Evidence limits:** The prototype was a headless functional check, not a visual comparison on Wayland/macOS/Windows. It checked the accessible label and callback, but did not exercise the complete Yomu persistence roundtrip or compare keyboard/focus behavior across every settings category. Revisit only with a category-by-category parity prototype.

### A-03 — Cover cache simplification — Completed

Consolidated repeated make-room eviction, keyed removal, image release, and clear operations in [src/ui/cover_cache.rs](/home/sabisho/Projects/yomu/src/ui/cover_cache.rs). Capacity remains bounded, recency tracking remains unchanged, and an `insert_item` for an existing key now removes/releases that prior entry before reinserting it.

Reset-all previously called `clear_without_window()` with no `App` context, which cleared the cache's maps but could leave uploaded textures in GPUI's image atlas. The method now accepts `&mut App` and routes through the shared clear path, which calls `App::drop_image(image, None)` for loaded cache items. Entity release uses the same path. [src/main.rs](/home/sabisho/Projects/yomu/src/main.rs) passes the application context from `clear_all_data`; the cache capacity test uses the updated signature.

This preserves the 256-entry default bound and does not introduce a cache dependency or a second cache abstraction.

### A-10 — Bounded LRU lookup — No change

Kept the `VecDeque` recency lookup. The cache is capped at 256 entries by default (and has a minimum configured capacity of 16); each recency lookup is therefore bounded. No evidence showed that replacing it with a more complex indexed LRU would reduce total system cost. No LRU data structure or behavior changed.

### A-14 — Library row cache — Investigated; no change

Reviewed `LibraryRowCache::is_valid`, row construction call sites, and existing mutation/search/sort/group/view-mode invalidation tests. Ran the existing ignored scalability benchmark for 100, 1,000, 5,000, and 10,000 books. It reported 0.01–0.02 ms for the measured cached render-tree path on this machine.

The benchmark times render-tree generation after the row cache has been populated; it does not measure invalidation cost or allocation counts. The existing cache and `VirtualList` remain because no measured issue justified replacing them. A future change to invalidation or per-frame projections should first use an allocation profiler or a benchmark that isolates cache rebuilds.

## Preservation checks

- `PlatformScrollController`, external scroll-handle support, and Linux/Windows wheel forwarding remain.
- The reader canvas, spread/RTL/continuous layout, off-thread decode, prefetch/cancellation, and page memory limits were not modified.
- Library `VirtualList` and row-cache logic were not modified.
- Cache bounds remain, with explicit GPU release now also applied on reset-all.
- Persistence ordering, atomic writes, shutdown behavior, archive validation, and source-file safety were not modified.
- Custom theme reload, jemalloc configuration, Nix accommodations, dependencies, offline-first behavior, and unsafe-code prohibitions were not changed.

## Files changed

- **Documentation:** `AGENTS.md`, `GEMINI.md`, `src/ui/AGENTS.md`, `docs/architecture.md`, `docs/development.md`, `docs/agent-handoff.md`.
- **Application:** `src/main.rs`, `src/ui/cover_cache.rs`, `src/tests.rs` (only the existing cache test call updated).
- **Vendor:** `vendor/gpui-component/src/lib.rs` (redundant re-export removed); `vendor/gpui-component/YOMU_PATCHES.md` added. Scroll and popup-selector hunks remain in their existing vendor source files.
- **Report:** This file.

No dependency manifest, lockfile, Nix configuration, allocator declaration, or user library data changed. The three audit input documents were already untracked when this task began and were left untouched.

## Validation results

Validation ran inside `devenv shell`.

| Command / proof | Result |
|---|---|
| `cargo fmt --all -- --check` | Exit 0 (Stages 1, 2, and 4) |
| `cargo check --all-targets` | Exit 0 (Stages 1, 2, and 4) |
| `cargo clippy --all-targets --all-features -- -D warnings` | Exit 0 (Stages 1, 2, and 4) |
| `cargo test --all-targets` | Exit 0 (Stages 1, 2, and 4): 171 + 220 passed, 16 ignored, 0 failed |
| Menu selector test with selector restored | Exit 0: 1 passed |
| Touchpad momentum filter | Exit 0: 3 passed |
| Cover-cache capacity/LRU test | Exit 0: 1 passed |
| Reset-all reader/cache cleanup test | Exit 0: 1 passed |
| Temporary native Settings prototype test | Exit 0: 1 passed; test code removed after evaluation |
| Library row scalability benchmark | Exit 0: 1 ignored benchmark run; 0.01–0.02 ms reported for the cached path |
| `cargo build --release` | Exit 0; optimized build completed in 2m06s |
| Release startup smoke test | `timeout` exit 124 after 8 seconds; the isolated release process remained alive until timeout, with no startup error output |
| Unsafe-code check | No unsafe code added; crate-root `#![forbid(unsafe_code)]` declarations unchanged |

An attempted removal of the popup selector intentionally produced a failing focused test (exit 101); the selector was restored, and the same test then passed. Wayland/macOS/Windows visual appearance and physical touchpad behavior were **UNVERIFIED** in this run. The startup smoke test used `/tmp/yomu-simplification-smoke` via `YOMU_DATA_DIR` and did not use the user's library path.

## Remaining work and risks

- The Settings prototype did not establish keyboard/focus parity or cross-platform visual density for the full six-category screen; native replacement remains deferred.
- The library benchmark covered cached render-tree construction only. It did not isolate row-cache rebuild allocations.
- The 256-entry LRU lookup remains a bounded linear scan; no evidence currently warrants added indexing complexity.
- The scroll extensions still require a future upstream public equivalent for external handle ownership and wheel-event control. Recheck exact upstream diffs on each Kit upgrade.
- Interactive appearance and touchpad momentum remain unverified on physical Linux, Windows, and macOS desktops.
