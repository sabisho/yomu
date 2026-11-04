# Library Selection State, Responsive Grid & Toolbar Consistency Refinement

**Status:** Completed & Verified  
**Date:** 2026-11-04  
**Scope:** Group-card selection derivation (`FullySelected`, `PartiallySelected`, `Unselected`), responsive grid space utilization and padding, segmented toolbar control (`ButtonGroup`) for Groups vs Volumes presentation modes.  
**Rule Compliance:** Safe Rust 2024 (`#![forbid(unsafe_code)]`), native GPUI Kit 0.8.1 components, Gruvbox theme palette (`UiColors`), 390 active tests passing (code 0).

---

## 1. Executive Summary & Root Cause Analysis

### Issue 1 — Group-Card Selection State (Ctrl+A)
- **Defect:** In Groups view, pressing Ctrl+A selected all member volumes across the library and updated the selection toolbar ("118 selected"), but the visible group cards remained completely unselected visually with no checkmark or border highlight. Furthermore, navigating into and out of groups executed `self.selection.clear()`, wiping the active selection.
- **Root Cause:**
  1. `render_folder_card` only checked `selected: bool` passed down from callers, and callers in `render_all_manga_view` had no way of knowing whether member volumes of a folder were selected because `LibraryItem::Folder` did not store member book IDs.
  2. The folder card did not distinguish between partial and full selection.
  3. `enter_group_named` and `exit_group` unconditionally called `self.selection.clear()`.
- **Fix:**
  1. Created `GroupSelectionState` enum (`Unselected`, `PartiallySelected`, `FullySelected`) and `compute_group_selection_state` helper in `src/ui/library_view.rs`.
  2. Added `member_ids: Arc<[String]>` to `LibraryItem::Folder` and `render_folder_card`.
  3. Rendered distinct visual states on group cards:
     - `FullySelected`: Selected background tint, accent border, and `IconName::Check` checkmark badge.
     - `PartiallySelected`: Selected background tint, accent border, and `IconName::Minus` indeterminate badge.
     - `Unselected`: Clean default card state without selection badge.
  4. Removed destructive `self.selection.clear()` on group navigation, preserving selection across drill-down and return.
  5. Implemented `toggle_group_selection` in `src/main.rs` and bound folder card clicks during selection mode (or Ctrl+click) to cleanly toggle group member volumes.

### Issue 2 — Library Grid Horizontal Space Utilization
- **Defect:** In Groups view, two group cards occupied a narrow strip on the left with huge dead margins (up to 116px on each side). In Volumes view, large outer margins left excess horizontal space unused.
- **Root Cause:** In `compute_responsive_grid`, column counts were calculated using integer `floor` against an immutable fixed card width (`card_width() = 220px`). All leftover pixels across the viewport were dumped directly into `pad_x = (content_w - used_w) / 2`, resulting in huge outer margins.
- **Fix:** Refined `compute_responsive_grid` in `src/ui/components.rs`:
  - Calculate target columns via rounded float division: `cols = ((avail_w + gap) / slot_w).round().max(1.0)`.
  - Distribute content width proportionally across columns while constraining card scaling to a restrained ±12% window.
  - Maintain the strict 1.5 aspect ratio (2:3 manga ratio) so height dynamically tracks width (`cover_h = card_w * 1.5`).
  - Constrain horizontal padding (`pad_x`) to standard bounds (16–24px) with fixed 12–16px gaps, completely eliminating dead horizontal margins while ensuring partially filled rows align naturally.

### Issue 3 — Groups / Volumes Toolbar Active States
- **Defect:** Mutually exclusive Groups and Volumes view options lacked visual consistency; one toggle button showed an active highlight while adjacent buttons were neutral ghost controls.
- **Root Cause:** The grouping toggle was rendered as an isolated single button instead of a cohesive segmented mode control.
- **Fix:** Replaced the single toggle button in `src/ui/toolbar.rs` with a native GPUI Kit 0.8.1 `ButtonGroup::new("grouping-mode-segmented").compact()`:
  - Button 1: "mode-groups" (`IconName::Folder`) with tooltip "View groups (folders and series)".
  - Button 2: "mode-volumes" (`IconName::BookOpen`) with tooltip "View individual volumes".
  - Active button displays highlighted selection styling via `.selected(true)`; inactive button uses neutral ghost styling.
  - Sizing is locked to `CONTROL_HEIGHT_TOOLBAR = 28px`, perfectly matching neighboring sort and view-mode controls.

---

## 2. Modified Files & Component Touchpoints

| File | Changes Made |
|------|--------------|
| [`src/selection.rs`](../../src/selection.rs) | Added `select_many` and `deselect_many` batch selection helpers. Added unit test `test_select_many_and_deselect_many`. |
| [`src/ui/components.rs`](../../src/ui/components.rs) | Updated `compute_responsive_grid` to distribute horizontal space evenly, bind `pad_x` to 16–24px, scale cards within ±12%, and maintain 1.5 aspect ratio. |
| [`src/ui/library_view.rs`](../../src/ui/library_view.rs) | Added `GroupSelectionState` enum, `compute_group_selection_state`, `member_ids` on `LibraryItem::Folder`, and rendered `IconName::Check` / `IconName::Minus` badges. |
| [`src/ui/toolbar.rs`](../../src/ui/toolbar.rs) | Replaced standalone button with `ButtonGroup::new("grouping-mode-segmented").compact()` containing `mode-groups` and `mode-volumes`. |
| [`src/app/views/library.rs`](../../src/app/views/library.rs) | Passed `member_ids` to `LibraryItem::Folder`, computed `selection_state` per visible folder, and wired selection-toggle click handlers. |
| [`src/app/views/continue_reading.rs`](../../src/app/views/continue_reading.rs) | Updated `render_folder_card` call site with `selection_state` and `member_ids`. |
| [`src/main.rs`](../../src/main.rs) | Added `toggle_group_selection`; removed destructive `self.selection.clear()` in `enter_group_named` and `exit_group`. |
| [`src/tests.rs`](../../src/tests.rs) | Added 3 comprehensive UI tests (`test_group_card_selection_states_and_ctrl_a`, `test_responsive_grid_space_utilization_and_padding`, `test_toolbar_grouping_segmented_control_states`) and updated `entering_and_exiting_group_preserves_selection`. |

---

## 3. Validation Matrix

All validation steps executed inside `devenv shell`:

```bash
# 1. Format check
devenv shell cargo fmt --all -- --check
# Result: Exit code 0 (clean)

# 2. Fast type check across all targets
devenv shell cargo check --all-targets
# Result: Exit code 0 (clean)

# 3. Strict Clippy lint check with warnings as errors
devenv shell cargo clippy --all-targets --all-features -- -D warnings
# Result: Exit code 0 (0 warnings)

# 4. Full test suite across all targets
devenv shell cargo test --all-targets
# Result: Exit code 0 (171 lib unit tests passed, 219 binary UI tests passed, 16 ignored benchmarks; 406 total)
```

---

## 4. Native Wayland Visual Verification

Executed on native Wayland (`wayland-0` compositor session):

1. **`Screenshots/library_selection_ctrl_a_groups.png`**:
   - Ctrl+A in Groups view visibly selects both group cards.
   - Tinted background and accent border applied to cards.
   - Checkmark badge (`IconName::Check`) displayed in top-right of cards next to volume count.
   - Selection toolbar reports "118 selected", "Unselect all", "Remove", "Cancel".
2. **`Screenshots/library_selection_partial_group.png`**:
   - Partially selected group ("Berserk" with 39 of 40 volumes selected) displays minus badge (`IconName::Minus`).
   - Completely unselected group ("Kingdom") remains neutral with no selection indicators.
   - Toolbar reports "39 selected".
3. **`Screenshots/library_groups_mode.png` & `Screenshots/library_all_manga_view.png`**:
   - Both Groups view and Volumes view horizontally utilize available viewport width.
   - Left and right margins are bounded to 24px padding; dead 116px margin eliminated.
   - Proportional card dimensions strictly maintain 1.5 aspect ratio.
4. **`Screenshots/crop_toolbar_groups.png` & `Screenshots/crop_toolbar.png`**:
   - Native `ButtonGroup` compact segmented control displays unified pill styling.
   - Selected mode (`Folder` or `BookOpen`) highlights with active background; inactive mode uses ghost background.
   - Sizing matches neighboring toolbar controls (28px height).
