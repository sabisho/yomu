# Library Import UX, Folder Grouping & UI Regression Fixes Report

**Date:** 2026-11-04  
**Runtime:** Linux x86_64 (NixOS / Wayland `wayland-0` Compositor)  
**Rust Edition:** 2024 (`rustc 1.99.0`), GPUI Kit 0.8.1 (`gpui-pre = 0.3.10`)  
**Safety Invariant:** `#![forbid(unsafe_code)]` preserved across all modules  
**Test Suite:** 386 active tests passing (170 lib unit tests, 216 main tests, plus 16 performance benchmarks)

---

## 1. Executive Summary & Root Cause Analysis

Four targeted issues were investigated, remediated, and verified across Yomu's library import pipeline, data model, toolbar, and settings views:

### Issue 1 — Home Screen Single Import Button Only Handled Files
- **Root Cause:** In `src/ui/continue_reading.rs` and `src/app/views/library.rs`, empty state presentation only provided a single `+ Import Manga` action wired unconditionally to `this.import_dialog(false, cx)` (`directories: false`). Users could not invoke directory selection from the empty home state despite the description text indicating support for manga folders.
- **Resolution:** Replaced the single action with two balanced, distinguishable buttons:
  - **Import Files** (`empty-import-files-btn`): primary accent styling with `IconName::FilePlus`, invoking `import_dialog(false)`.
  - **Import Folder** (`empty-import-folder-btn`): outline styling with `IconName::FolderPlus`, invoking `import_dialog(true)`.

### Issue 2 — Folder Imports Did Not Group Volumes
- **Root Cause:**
  1. In `src/library.rs` (`collect`), candidates gathered during folder recursion did not retain the imported folder identity.
  2. In `build_book`, `Book::series` was unconditionally initialized to `None`.
  3. `directory_groups` in `src/ui/library_view.rs` ignored groups with fewer than 2 books even if explicitly assigned to a series.
  4. `Library::add` did not normalize casing against existing series names, risking fragmented groups on case variations.
- **Resolution:**
  - Extended candidate collection to track folder group hierarchy:
    - Folders with direct volume contents (archives or image folders) associate all volumes with the imported folder's display name (`series: Some(name)`).
    - Container folders with multiple sub-manga directories (e.g. `Comics/` containing `Berserk/` and `Claymore/`) preserve relative folder structures without flattening.
    - Single file imports retain `series: None`.
  - Normalized series casing in `Library::add` case-insensitively against existing series names.
  - Automatically enabled `group_series = true` when folder imports introduce grouped volumes.
  - Updated `directory_groups` to permit single-volume explicit series groups while maintaining `>= 2` threshold for arbitrary unassigned directories.

### Issue 3 — Disappeared Toolbar Grouping Toggle
- **Root Cause:** In earlier refactoring (commit `50beeac`), `show_grouping` and `is_group_series` were removed from `MainToolbarProps` in `src/ui/toolbar.rs`, leaving users unable to switch between Groups view and Volumes view in the toolbar.
- **Resolution:**
  - Restored `grouping-mode-toggle` in `render_main_toolbar` alongside `view-mode-toggle` and `toolbar-sort`.
  - Configured matching 28px (`CONTROL_HEIGHT_TOOLBAR`) ghost button with visual active-state distinction (`selected(props.is_group_series)`).
  - Toggles between `IconName::Folder` ("Switch to Volumes View") and `IconName::BookOpen` ("Switch to Groups View").
  - Preserved selection and persistence via existing `toggle_group_series(group, cx)` without destructively altering underlying library data.

### Issue 4 — Blue-Tinted Yomu Icon in Settings → About
- **Root Cause:** In `src/app/desktop.rs` (`app_logo_image()`), `LOGO_PNG` was loaded via `image::load_from_memory(...).to_rgba8()` and passed directly into `gpui_kit::RenderImage`. However, GPUI's texture rendering pipeline natively expects **BGRA** pixel format (as documented in `src/imaging.rs`: `/// The returned buffer is already **BGRA**, the channel order gpui expects`). Because the Red channel was at byte index 0 and Blue at byte index 2, GPUI's GPU compositor uploaded red pixels into the blue texture channel, turning the warm reddish/orange Yomu logo into an unnatural blue hue.
- **Resolution:**
  - In `app_logo_image()`, swapped channels 0 and 2 (`px.swap(0, 2)`) before wrapping into `RenderImage::new`.
  - Preserved original artwork, alpha channel, 48x48px container with 10px corner rounding, and `ObjectFit::Contain`.

---

## 2. Files and Modules Modified

| File | Responsibilities Changed |
| :--- | :--- |
| [`src/app/desktop.rs`](file:///home/sabisho/Projects/yomu/src/app/desktop.rs) | Swapped Red/Blue channels to BGRA in `app_logo_image()`; added channel-order unit test. |
| [`src/library.rs`](file:///home/sabisho/Projects/yomu/src/library.rs) | Candidate group tracking, series assignment in `build_book`, case-insensitive series reuse in `Library::add`, unit tests for folder grouping. |
| [`src/ui/continue_reading.rs`](file:///home/sabisho/Projects/yomu/src/ui/continue_reading.rs) | Split empty state import button into "Import Files" and "Import Folder" with balanced layout and icons. |
| [`src/app/views/continue_reading.rs`](file:///home/sabisho/Projects/yomu/src/app/views/continue_reading.rs) | Wired `on_import_files` (`false`) and `on_import_folder` (`true`) handlers. |
| [`src/app/views/library.rs`](file:///home/sabisho/Projects/yomu/src/app/views/library.rs) | Added dual import actions to `render_empty_library_state` with debug selectors. |
| [`src/ui/toolbar.rs`](file:///home/sabisho/Projects/yomu/src/ui/toolbar.rs) | Restored `grouping-mode-toggle` button, `show_grouping`, `is_group_series`, and `on_toggle_group_series`. |
| [`src/app/views/shell.rs`](file:///home/sabisho/Projects/yomu/src/app/views/shell.rs) | Forwarded grouping state and `toggle_group_series` callback to main toolbar. |
| [`src/main.rs`](file:///home/sabisho/Projects/yomu/src/main.rs) | Automatically enabled `group_series` on folder imports with series volumes. |
| [`src/ui/library_view.rs`](file:///home/sabisho/Projects/yomu/src/ui/library_view.rs) | Supported single-volume explicit series groups in `directory_groups`. |
| [`src/tests.rs`](file:///home/sabisho/Projects/yomu/src/tests.rs) | Added unit and integration UI tests for empty state actions, toolbar toggle, and folder grouping. |

---

## 3. How Folder Grouping is Represented and Persisted

1. **Data Model Representation:**
   - Volumes are modeled as `yomu::library::Book`.
   - Grouping is represented by `Book.series: Option<String>`.
   - When a folder `Berserk/` is imported, all discovered volumes receive `series: Some("Berserk".into())`.
2. **Persistence:**
   - Stored in `$XDG_DATA_HOME/yomu/library.json` within each book record: `"series": "Berserk"`.
   - Persists across restarts without requiring secondary mapping tables.
3. **Case-Insensitive Normalization:**
   - On `Library::add`, if a volume has `series: Some(name)`, `Library::add` scans existing books for a case-insensitive match (`existing.eq_ignore_ascii_case(&name)`). If found, it adopts the existing canonical string, preventing group fragmentation (e.g., `Berserk` vs `berserk`).
4. **Nested Directory Discovery:**
   - Folders directly containing volumes assign those volumes to that folder's name.
   - Root container directories with multiple sub-manga directories (e.g. `Comics/` containing `Berserk/` and `Claymore/`) discover and preserve individual series folders independently.

---

## 4. How the Toolbar Toggle Works

1. **Toolbar Control:**
   - Element debug selector: `grouping-mode-toggle`.
   - Button height: 28px (`CONTROL_HEIGHT_TOOLBAR`), matching `view-mode-toggle` and `toolbar-sort`.
   - In Groups view: icon is `IconName::Folder`, tooltip is `"Switch to Volumes View"`, active indicator is `selected(true)`.
   - In Volumes view: icon is `IconName::BookOpen`, tooltip is `"Switch to Groups View"`, active indicator is `selected(false)`.
2. **Behavior & Non-Destructive State:**
   - Invokes `this.toggle_group_series(group, cx)`.
   - Updates `self.library.settings_mut().group_series = group`.
   - Clears `self.open_group = None` and resets group scroll if exiting group mode.
   - Persists setting to disk via `self.save_library(cx)`.
   - Does NOT alter book records, IDs, reading progress, or series assignments.

---

## 5. Original Icon Asset & Blue-Tint Root Cause

- **Asset Used:** Embedded PNG at `assets/logo.png` (`pub const LOGO_PNG: &[u8] = include_bytes!("../../assets/logo.png")`).
- **Cause of Blue Tint:**
  - `image::load_from_memory(...).to_rgba8()` outputs standard `[R, G, B, A]`.
  - GPUI texture uploads expect `[B, G, R, A]`.
  - Without swapping byte 0 and byte 2, the GPU interprets Red values as Blue, causing the warm tones of the Yomu logo to display with an inverted blue hue.
  - Adding `px.swap(0, 2)` before creating `RenderImage` restored 100% color fidelity.

---

## 6. Testing & Validation Results

### Added Tests:
1. `test_folder_import_automatic_grouping_and_nested_structure` (`src/library.rs`): validates direct folder grouping, individual file exclusion, nested directory preservation, reimport skipping, and case-insensitive group reuse.
2. `test_folder_import_empty_and_unsupported_files` (`src/library.rs`): validates empty directory and unsupported file handling.
3. `test_app_logo_image_bgra_channel_order_and_no_blue_tint` (`src/app/desktop.rs`): validates channel swap from RGBA to BGRA on non-transparent colored pixels.
4. `test_empty_library_import_actions_distinguishable_and_cancel_safe` (`src/tests.rs`): validates presence and layout of both Import Files and Import Folder buttons in Home and All Manga empty states.
5. `test_library_toolbar_grouping_toggle_presentation_and_persistence` (`src/tests.rs`): validates toolbar grouping toggle presence, mode toggling, visual distinction, and persistence.
6. `test_folder_import_automatic_grouping_and_reimport_behavior` (`src/tests.rs`): integration test verifying end-to-end folder import, series assignment, grouping activation, and duplicate skipping.

### Full Validation Suite Execution:
- `devenv shell cargo fmt --all -- --check` -> **Exit 0**
- `devenv shell cargo check --all-targets` -> **Exit 0**
- `devenv shell cargo clippy --all-targets --all-features -- -D warnings` -> **Exit 0**
- `devenv shell cargo test --all-targets` -> **Exit 0** (386 passed, 0 failed, 16 ignored)

---

## 7. Native Wayland Runtime Verification

- App binary built with `cargo build`.
- Executed on Wayland compositor (`wayland-0`): launched cleanly without panics, errors, or assertion failures.
- Both empty state import buttons and toolbar grouping controls respond cleanly to window layout.
- Application icon displays in its authentic colors in Settings → About.
