# GPUI Kit 0.9 Migration Plan & Execution Roadmap

**Project:** Yomu Manga Reader  
**Document Type:** Migration Plan & Structured Checklist  
**Target Upgrade:** GPUI Kit 0.8.1 (`gpui-pre 0.3.10`) $\to$ GPUI Kit 0.9.0 (`gpui-pre 0.3.11`)  
**Prerequisite:** [`GPUI_KIT_09_COMPATIBILITY_AND_ADOPTION_AUDIT.md`](GPUI_KIT_09_COMPATIBILITY_AND_ADOPTION_AUDIT.md)

---

## 1. Migration Strategy Overview

The upgrade from GPUI Kit 0.8.1 to 0.9.0 is structured into **five sequential, atomic stages**. Each stage contains verifiable acceptance criteria and preserves Yomu's core invariants under `#![forbid(unsafe_code)]`.

```text
Stage G2.1: Pre-Upgrade Test Clock Remediation
     │ (Replace wall-clock std::thread::sleep with advance_clock)
     ▼
Stage G2.2: Dependency & Build Graph Migration
     │ (Bump gpui-kit to 0.9.0, update Cargo.lock, reconcile vendor patch)
     ▼
Stage G2.3: Verification of Upstream Fixes & Component Improvements
     │ (Validate VirtualList gap fixes, Table AccessKit, Submenus, Popover focus)
     ▼
Stage G2.4: Accessibility & UI Consistency Polish
     │ (Add tooltips/accessible names to icon-only buttons, configure caret blink)
     ▼
Stage G2.5: Settings View Refinement & Final Qualification
       (Streamline SettingGroup primitives, full validation matrix, memory verification)
```

---

## 2. Prioritized Execution Checklist

### Phase 1: Breaking API & Test Preparation (Stage G2.1)
*Can be applied independently before or concurrently with dependency update.*

- [ ] **T-01: Audit Test Sleep Calls**
  - Target: `src/tests.rs` (lines 489, 1332, 1358).
  - Action: Replace `std::thread::sleep(Duration::from_millis(350))` with `cx.executor().advance_clock(Duration::from_millis(350))` or `cx.advance_clock(...)`.
  - Verification: Run `devenv shell cargo test test_context_menu test_right_click_on_an_unselected_book` and ensure tests pass deterministically.

### Phase 2: Dependency & Build Graph Migration (Stage G2.2)
*Must be applied together.*

- [ ] **D-01: Update `Cargo.toml` Dependencies**
  - Update `gpui-kit = { version = "=0.9.0", features = ["test-support"] }`.
  - Preserve `tikv-jemallocator = "0.6.1"` and profile overrides (`opt-level = 2` for `tikv-jemalloc-sys`).
  - Preserve `unrar = { path = "vendor/unrar" }` patch.
- [ ] **D-02: Reconcile `vendor/gpui-component` Patch**
  - Check whether `gpui-component 0.9.0` re-exports `Selectable` and includes `.debug_selector(|| "popup-menu".into())`.
  - If resolved upstream: Remove `gpui-component = { path = "vendor/gpui-component" }` patch from `Cargo.toml`.
  - If still required: Rebase `vendor/gpui-component` on `0.9.0`.
- [ ] **D-03: Execute `cargo check` and Dependency Audit**
  - Run `devenv shell cargo check --all-targets`.
  - Run `devenv shell cargo audit`.
  - Verify zero compiler errors, zero warnings.

### Phase 3: Core Framework Behavior Verification (Stage G2.3)
*Validates that existing native integrations benefit from 0.9.0 fixes without regressions.*

- [ ] **V-01: VirtualList Invalidation Check**
  - Target: `src/app/views/library.rs` (`v_virtual_list`), `src/ui/library_view.rs` (`LibraryRowCache`).
  - Verify that window resizing, gap adjustments, and column reflowing correctly update the virtual list view without stale item offsets.
  - Test Target: `test_virtual_grid_filtering_and_sorting_invalidation`, `test_virtual_grid_row_construction_and_partially_filled_rows`.
- [ ] **V-02: Context Menu & Submenu Navigation**
  - Target: `src/app/actions.rs` (`build_book_context_menu`, `build_group_context_menu`).
  - Verify submenu activation and verify smart positioning on screen right edge.
- [ ] **V-03: Popover & Dialog Focus Restoration**
  - Target: `src/ui/overlays.rs`, `src/app/actions.rs` (`show_confirm_alert_on_window`, `open_series_dialog`).
  - Verify focus returns to previous element or window root upon closing confirmation and properties dialogs.

### Phase 4: Accessibility & UI Consistency Polish (Stage G2.4)
*Targeted refinements to improve usability and assistive technology support.*

- [ ] **A-01: Icon-Only Button Accessible Names**
  - Target: `src/ui/toolbar.rs`, `src/ui/reader.rs`.
  - Add explicit `.tooltip(...)` / `.aria_label(...)` to:
    - Toolbar: `mode-groups` ("Group by Series"), `mode-volumes` ("Show All Volumes"), `view-mode-toggle` ("Toggle Grid/List View"), `toolbar-sort` ("Sort Books").
    - Selection Toolbar: `unselect-all` ("Deselect All"), `select-all` ("Select All"), `select-remove` ("Remove Selected"), `select-cancel` ("Cancel Selection").
    - Reader HUD: `reader-back-btn` ("Return to Library"), `reader-header-prev-btn` ("Previous Page"), `reader-header-next-btn` ("Next Page"), `reader-layout-toggle` ("Toggle Layout"), `reader-fit-toggle` ("Toggle Fit Mode"), `reader-fullscreen-toggle` ("Toggle Fullscreen").
- [ ] **A-02: Shortcuts Table AccessKit Verification**
  - Target: `src/ui/settings_view.rs` (`render_tab_shortcuts`).
  - Verify that `Table`, `TableHeader`, `TableRow`, and `TableCell` emit appropriate accessibility tree nodes.

### Phase 5: Settings Primitives & Final Qualification (Stage G2.5)
*Simplification and final validation matrix.*

- [ ] **S-01: Optional Settings View Form Group Simplification**
  - Target: `src/ui/settings_view.rs`.
  - Evaluate replacing repetitive manual `h_flex()` / `v_flex()` wrappers with `SettingGroup` and `SettingItem` from `gpui_kit::component::setting`.
  - Preserve Yomu's custom tab switching and settings state mutations.
- [ ] **S-02: Full Validation Matrix Execution**
  - Execute complete verification suite inside `devenv`:
    ```bash
    devenv shell cargo fmt --all -- --check
    devenv shell cargo check --all-targets
    devenv shell cargo clippy --all-targets --all-features -- -D warnings
    devenv shell cargo test --all-targets
    devenv shell cargo audit
    ```
- [ ] **S-03: Memory & Allocator Plateau Verification**
  - Confirm `tikv-jemallocator 0.6.1` is active on Linux GNU.
  - Verify zero memory retention drift across consecutive reader cycles.

---

## 3. Dependency & Blast Radius Map

```text
┌────────────────────────────────────────────────────────────────────────┐
│                        UPGRADE BLAST RADIUS                            │
├─────────────────────────┬───────────────────┬──────────────────────────┤
│ Subsystem               │ Files Affected    │ Expected Impact          │
├─────────────────────────┼───────────────────┼──────────────────────────┤
│ Core Domain (`src/`)    │ None              │ Zero impact (guaranteed  │
│                         │                   │ decoupled from GUI)      │
├─────────────────────────┼───────────────────┼──────────────────────────┤
│ Build Configuration     │ `Cargo.toml`      │ Bump gpui-kit version;   │
│                         │ `Cargo.lock`      │ rebase/remove vendor     │
│                         │ `vendor/`         │ patch                    │
├─────────────────────────┼───────────────────┼──────────────────────────┤
│ Test Suite              │ `src/tests.rs`    │ Replace 3 sleep calls    │
│                         │                   │ with advance_clock       │
├─────────────────────────┼───────────────────┼──────────────────────────┤
│ Toolbar & Reader HUD    │ `src/ui/toolbar.rs│ Add explicit tooltips to │
│                         │ `src/ui/reader.rs`│ 16 icon-only buttons     │
├─────────────────────────┼───────────────────┼──────────────────────────┤
│ Settings View           │ `src/ui/settings_ │ Simplify form layout via │
│                         │  view.rs`         │ SettingGroup primitives  │
└─────────────────────────┴───────────────────┴──────────────────────────┘
```

---

## 4. Rollback Criteria & Safety Invariants

If any of the following occur during migration, halt and roll back:
1. **Unsafe Code Invariant Breach:** Any dependency or workaround requiring `#![allow(unsafe_code)]` or unsafe blocks in Yomu.
2. **Persistence Regression:** Any failure in `library.json` or `progress.json` durability, atomic rename, or monotonic save sequencing.
3. **Memory Plateau Violation:** Memory retention post-reader closure creeping upward across repeated cycles (> 50 MiB drift across 10 cycles).
4. **Scrolling Glitch:** Loss of kinetic inertia glide on Linux/Windows.
5. **Test Assertion Weakening:** Any temptation to mark failing tests `#[ignore]` or weaken assertions.

---

## 5. Post-Migration Verification Commands

Upon receiving user approval to begin Stage G2 implementation, execute the following commands inside `devenv`:

```bash
# 1. Format check
devenv shell cargo fmt --all -- --check

# 2. Type check all targets (lib, bin, tests, benchmarks)
devenv shell cargo check --all-targets

# 3. Strict Clippy lint check with warnings as errors
devenv shell cargo clippy --all-targets --all-features -- -D warnings

# 4. Full test suite (all 390 active tests must pass)
devenv shell cargo test --all-targets

# 5. Security audit of dependency graph
devenv shell cargo audit
```
