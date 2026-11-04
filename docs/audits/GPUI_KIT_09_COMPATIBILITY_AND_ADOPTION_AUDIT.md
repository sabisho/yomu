# GPUI Kit 0.9 Compatibility & Native Component Adoption Audit

**Project:** Yomu Manga Reader  
**Status:** Audit & Planning Phase (Stage G1)  
**Author:** AI Engineering Agent  
**Baseline Date:** November 4, 2026  
**Target Upgrade:** GPUI Kit 0.8.1 (`gpui-pre 0.3.10`) $\to$ GPUI Kit 0.9.0 (`gpui-pre 0.3.11`)

---

## Executive Summary

This comprehensive audit evaluates the Yomu manga reader codebase for upgrading from **GPUI Kit 0.8.1** to **GPUI Kit 0.9.0**.

In accordance with Yomu's core engineering principles—**Rule 1 (Zero Unsafe Rust)**, **Rule 2 (Idiomatic Rust Everywhere)**, **Rule 3 (Native GPUI Kit APIs First)**, **Rule 4 (Maximum Practical Performance)**, **Rule 5 (Consistent UI Design)**, **Rule 6 (Real-World Use Cases)**, and **Rule 7 (Simplicity Over Overengineering)**—this audit assesses breaking API changes, opportunities for adopting native components, simplifications of custom code, NixOS build compatibility, and potential migration risks.

### Summary of Audit Findings

| Classification | Count | Description |
|---|:---:|---|
| **REPLACE** | 1 | Existing test harness wall-clock sleeping (`std::thread::sleep`) replaced with test executor clock (`advance_clock`). |
| **SIMPLIFY** | 2 | Rebase or eliminate vendored `gpui-component` patch; streamline settings group primitives. |
| **IMPROVE** | 6 | VirtualList geometry invalidation; table accessibility; submenu positioning & activatable rows; popover focus cleanup; icon-only button accessible names/tooltips; input touch selection. |
| **KEEP** | 9 | Core domain systems (archives, imaging, layout, Vim, progress sidecar); `PlatformScrollController` momentum scrolling; row-based library virtualization; app raster logo & branding; jemalloc global allocator. |
| **WAIT** | 2 | Upstream native Linux/Windows momentum scrolling; application-level dynamic third-party icon theme packs. |
| **Total Actionable Items** | **20** | **1 REPLACE, 2 SIMPLIFY, 6 IMPROVE, 9 KEEP, 2 WAIT** |

---

## 1. Current Dependency and Environment Baseline

### 1.1 Installed Versions (`Cargo.lock` & `Cargo.toml`)
- **Language & Edition:** Rust 2024 edition (`rust-version = "1.92"`, runtime `rustc 1.99.0` via NixOS/devenv).
- **Primary Toolkit:** `gpui-kit = "=0.8.1"` with `features = ["test-support"]`.
- **Runtime Backend:** `gpui-pre = "0.3.10"`.
- **Component Subcrate:** `gpui-component = "0.8.1"` (currently patched via `vendor/gpui-component`).
- **Archive Decoders:** `zip = "2"` (pure Rust), `unrar = "0.5.8"` (patched in `vendor/unrar` for bounded memory extraction), `image = "0.25"`, `zune-jpeg = "0.5"`, `fast_image_resize = "6"`.
- **Global Allocator:** `tikv-jemallocator = "0.6.1"` configured strictly for `cfg(all(target_os = "linux", target_env = "gnu"))`.
- **Persistence Store:** `$XDG_DATA_HOME/yomu/library.json` (v3) + `$XDG_DATA_HOME/yomu/progress.json` (v1 sidecar).

### 1.2 Development Environment (`devenv.nix` & `flake.nix`)
- **Host OS:** Linux (Wayland-native with X11 fallback on NixOS).
- **Nix Packages:** `pkg-config`, `cmake`, `cargo-audit`, plus runtime dynamic libraries (`wayland`, `libxkbcommon`, `vulkan-loader`, `libGL`, `fontconfig`, `freetype`, `libxcb`, `libx11`, `libxcursor`, `libxi`, `libxrandr`).
- **Profile Overrides:** `[profile.dev.package.tikv-jemalloc-sys]` and `[profile.test.package.tikv-jemalloc-sys]` with `opt-level = 2` to satisfy NixOS `-D_FORTIFY_SOURCE=2` compilation.

### 1.3 Test Suite Baseline
- **Core Library (`src/lib.rs`):** 171 passed; 0 failed.
- **Binary & UI Target (`src/main.rs`):** 219 passed; 0 failed; 16 ignored (benchmarks).
- **Total Tests:** **390 active tests** passing (406 total). Zero compiler warnings under `cargo clippy --all-targets --all-features -- -D warnings`.

---

## 2. GPUI Kit 0.9.0 Release Analysis

Official upstream sources evaluated:
- `https://gpui-kit.com/releases/`
- `https://github.com/longbridge/gpui-kit/releases/tag/v0.9.0`
- `https://gpui-kit.com/docs/`

### 2.1 Backend Dependencies Update
- **Default Backend:** Updated to `gpui-pre 0.3.11`.
- **Optional Fast Backend:** Updated to `gpui-fast 0.1.13` (Yomu maintains default backend; does not switch to Fast backend).
- **Wayland / Vulkan Stack:** Fully binary- and ABI-compatible with Yomu’s current `devenv.nix` runtime libraries.

### 2.2 Core Breaking Changes
1. **Public Theme Structs Marked `#[non_exhaustive]`:**
   - *Impact on Yomu:* Direct struct literals (`ThemeConfig { ... }` or `ColorTokens { ... }`) defined outside `gpui-kit` fail compilation.
   - *Audit Finding:* Yomu constructs themes via Serde deserialization from JSON/TOML (`serde_json::from_value(...)` in `src/ui/theme.rs`). Yomu does **not** construct `ThemeConfig` via struct literals in application code. No struct literal migration is required in `src/`.
2. **`Window::text_system()` Changed from `Arc` to `Rc`:**
   - *Impact on Yomu:* `Window::text_system()` returns `&Rc<WindowTextSystem>`, and `WindowTextSystem` is `!Send + !Sync`.
   - *Audit Finding:* Yomu contains **zero** direct calls to `Window::text_system()` or `WindowTextSystem`. Decoupled domain layout and font metrics are handled via GPUI Kit high-level components. Zero compilation impact.
3. **Animation Clock Uses Executor Clock (Not `Instant::now()`):**
   - *Impact on Yomu:* Animations now read the GPUI executor clock. Tests relying on wall-clock `std::thread::sleep` desynchronize from the mock test clock.
   - *Audit Finding:* Yomu has 3 tests in `src/tests.rs` using `std::thread::sleep(Duration::from_millis(350))`. These must be migrated to `cx.executor().advance_clock(...)`.

### 2.3 Feature Additions & Component Improvements
1. **Application-Level Icon Resolver:**
   - New `set_icon_resolver(cx, ...)` and `resolve_icon_path` allow global interception of icon assets across all components.
2. **VirtualList Enhancements:**
   - Fixed geometry invalidation when list axis or gap changes.
   - Fixed horizontal list height inference using constraints.
3. **Menu, Popover & Submenu Enhancements:**
   - Submenu rows are properly activatable.
   - Smart positioning opens submenus on the left when right screen space is insufficient.
   - Popover clears focus on close if no element had focus initially.
   - Menu checked/unchecked accessible semantics.
4. **Settings Component Improvements:**
   - Empty state view for unmatched search queries.
   - Settings sidebar smoothly follows scrolling.
5. **Accessibility (AccessKit):**
   - Accessible roles and headers for `Table`.
   - Icon-only button accessible names and tooltips.
6. **Input & Editor:**
   - `InputEditsEvent` batch edit event.
   - `Theme::cursor_blink` global caret configuration (`CursorBlink::steady()`, `CursorBlink::blinking(interval)`).
   - Native touch selection delegation (`set_native_touch_selection`).
   - Editor/diff gutter decorations (not applicable to Yomu reader).

---

## 3. Comprehensive Yomu Subsystem Audit

### A. Icon and Asset Infrastructure
- **Current Implementation:**
  - Standard UI icons use `gpui_kit::assets::IconName` (Lucide SVG icons embedded in GPUI Kit) rendered via `Icon::new(...)`.
  - Application branding uses an embedded raster PNG (`assets/logo.png`, `src/app/desktop.rs`), rendered via `img(crate::app::desktop::app_logo_image())` with BGRA byte order swapping.
  - Multi-resolution desktop icons (`HICOLOR_ICON_SIZES`, 16x16 to 512x512) are installed via `--install-desktop` on Linux.
- **Evaluation of 0.9.0 APIs:**
  - `set_icon_resolver` and `resolve_icon_path` are intended for SVG-based icon packs. They do not replace raster textures (`RenderImage`) or desktop integration.
  - Yomu does not maintain custom SVG artwork; all standard icons use official `IconName`.
- **Classification:** **KEEP** (application branding and logo rendering); **WAIT** (external icon theme pack customization).

### B. Library View & VirtualList
- **Current Implementation (`src/app/views/library.rs`, `src/ui/library_view.rs`):**
  - Uses `gpui_kit::component::v_virtual_list` with `VirtualListScrollHandle`.
  - `LibraryRowCache` partitions 2D grid items (cards and folder groups) into uniform horizontal rows, calculating `item_sizes: std::rc::Rc<Vec<Size<Pixels>>>`.
  - Caches rows and item sizes with invalidation on search, sort, folder navigation, view mode, and column resize.
  - In Phase 8, this reduced per-frame render overhead by 7,779x (0.05 ms for 10,000 books).
- **Evaluation of 0.9.0 APIs:**
  - 0.9.0 fixes geometry invalidation when gap or axis changes. This directly resolves upstream geometry cache desync on rapid window resize.
  - Yomu's row-based virtualization design remains the correct architectural choice under Rule 3 and Rule 4.
- **Classification:** **KEEP** (row-based virtualization architecture); **IMPROVE** (upstream gap/geometry invalidation reliability).

### C. Scrolling & Momentum Architecture
- **Current Implementation (`src/ui/scroll.rs`):**
  - `PlatformScrollController` connects pure `motion::KineticScroll` to GPUI `ScrollHandle`.
  - On macOS, passes through native AppKit inertial scroll events directly.
  - On Linux and Windows, provides analytical exponential glide decay for trackpad gestures (`ScrollDelta::Pixels`).
  - Reader and Library use native `ScrollBounce` and `Scrollbar` with `ScrollbarMode::Scrolling` (auto-hide).
- **Evaluation of 0.9.0 APIs:**
  - GPUI Kit 0.9.0 does **not** introduce a cross-platform kinetic momentum engine for Linux/Windows.
  - Removing `PlatformScrollController` would regress Linux Wayland and Windows scrolling to abrupt halts.
- **Classification:** **KEEP** (preserve `PlatformScrollController`); **WAIT** (monitor future GPUI upstream platform momentum).

### D. Settings and Preferences Dialog
- **Current Implementation (`src/ui/settings_view.rs`, `src/app/views/settings.rs`, `src/ui/overlays.rs`):**
  - Modal overlay dialog presenting 6 tabs: General, Appearance, Reader, Shortcuts, Advanced, About.
  - Custom split-pane layout using `Sidebar`, `SidebarMenu`, `SidebarMenuItem`, and a scrollable content pane (`div().overflow_y_scrollbar()`).
  - Shortcuts tab renders a `Table` with action names and `Tag` keybindings.
- **Evaluation of 0.9.0 APIs:**
  - GPUI Kit 0.9.0 enhances `Settings` with unmatched-search empty states and synchronized sidebar scrolling.
  - Adopting `SettingGroup` and `SettingItem` components inside Yomu’s settings tabs will standardize label-to-control alignment and spacing while retaining Yomu’s custom tab routing.
  - The Shortcuts `Table` automatically inherits 0.9.0 AccessKit accessibility roles.
- **Classification:** **SIMPLIFY** / **IMPROVE**.

### E. Menus, Dialogs, and Toolbars
- **Current Implementation (`src/app/actions.rs`, `src/ui/toolbar.rs`, `src/ui/reader.rs`):**
  - Book and group context menus use `PopupMenu` and `PopupMenuItem`.
  - Dialogs use native `window.open_alert_dialog` (confirmations) and `window.open_dialog` (properties).
  - Mode toggles use `ButtonGroup::new("grouping-mode-segmented").compact()`.
  - Toolbar contains ~10 icon-only buttons (`mode-groups`, `mode-volumes`, `toolbar-sort`, etc.).
- **Evaluation of 0.9.0 APIs:**
  - Activatable submenu rows and smart left/right overflow positioning improve context menu usability near screen edges.
  - Popover focus fix prevents focus trap on closing.
  - Icon-only buttons should be enhanced with explicit accessible names/tooltips.
- **Classification:** **IMPROVE**.

### F. Inputs and Editor Infrastructure
- **Current Implementation (`src/ui/toolbar.rs`, `src/app/state.rs`, `src/app/actions.rs`):**
  - `InputState` used in Toolbar search (`search_input`) and Series naming dialog (`open_series_dialog`).
  - Search queries update via `InputEvent::Change` and filter books via `search::matches`.
- **Evaluation of 0.9.0 APIs:**
  - `InputEditsEvent` is available, but Yomu requires the full text query for substring filtering; existing `InputEvent::Change` is optimal.
  - `Theme::cursor_blink` can be configured globally.
  - Native touch selection benefits Wayland touchscreen devices.
  - Code editor and diff decorations are not applicable to a comic reader.
- **Classification:** **KEEP** (search input handling); **IMPROVE** (touch selection & cursor blink).

---

## 4. Findings Inventory & Classification

Every finding is classified strictly into: **REPLACE**, **SIMPLIFY**, **IMPROVE**, **KEEP**, or **WAIT**.

```
┌─────────────────────────────────────────────────────────────┐
│                      FINDINGS MATRIX                        │
├──────────────┬───────┬──────────────────────────────────────┤
│ REPLACE      │  1    │ Test harness wall-clock sleep        │
│ SIMPLIFY     │  2    │ Vendored patch; Settings layout      │
│ IMPROVE      │  6    │ VirtualList, Table, Menu, Tooltip... │
│ KEEP         │  9    │ Core domain, Momentum, Rows, Logo... │
│ WAIT         │  2    │ Upstream Linux momentum, Icon themes │
└──────────────┴───────┴──────────────────────────────────────┘
```

### Finding F-01: Test Suite Animation Synchronization [REPLACE]
- **File & Location:** `src/tests.rs` (lines 489, 1332, 1358).
- **Current Implementation:** Calls `std::thread::sleep(Duration::from_millis(350))` to wait for menu dismiss transitions.
- **0.9.0 API:** Animations now synchronize with the GPUI executor clock. `cx.executor().advance_clock(Duration::from_millis(350))` or `cx.advance_clock(...)`.
- **Official Reference:** GPUI Kit 0.9.0 Release Notes — "Animation & Executor Clock".
- **Expected Benefit:** Tests become fully deterministic; eliminates 1,050 ms of arbitrary thread sleeping; eliminates timing flakiness in CI.
- **Complexity:** Low (3 line replacements).
- **Regression Risk:** None. All assertions remain identical.

### Finding F-02: Vendored `gpui-component` Patch [SIMPLIFY]
- **File & Location:** `Cargo.toml` (`[patch.crates-io]`), `vendor/gpui-component/`.
- **Current Implementation:** `vendor/gpui-component` is patched locally because commit `3095d20` added `.debug_selector(|| "popup-menu".into())` to `PopupMenu::render` and commit `2d28102` re-exported `Selectable`.
- **0.9.0 API:** `gpui-kit 0.9.0` updates the component dependency graph and test selector interfaces.
- **Official Reference:** GPUI Kit 0.9.0 Release Notes.
- **Expected Benefit:** Rebase or eliminate the 49-file vendored tree if upstream 0.9.0 re-exports `Selectable` and provides test menu selectors.
- **Complexity:** Medium (dependency reconciliation).
- **Regression Risk:** Low if test suite validates menu selector discovery.

### Finding F-03: Settings View Form Controls Alignment [SIMPLIFY]
- **File & Location:** `src/ui/settings_view.rs` (lines 400–850).
- **Current Implementation:** Manually wraps settings labels, descriptions, and inputs in `h_flex()` / `v_flex()` with hardcoded margins and widths.
- **0.9.0 API:** `gpui_kit::component::setting::{SettingGroup, SettingItem}`.
- **Official Reference:** GPUI Kit Component Guide — Settings.
- **Expected Benefit:** Standardizes spacing, typographic hierarchy, and responsive wrapping across all 6 settings tabs; eliminates ~150 lines of boilerplate flex layout code.
- **Complexity:** Medium.
- **Regression Risk:** Low (pure presentation refactoring; settings mutations remain identical).

### Finding F-04: VirtualList Geometry Invalidation [IMPROVE]
- **File & Location:** `src/app/views/library.rs` (`v_virtual_list`), `src/ui/library_view.rs` (`LibraryRowCache`).
- **Current Implementation:** Yomu calculates `item_sizes` and tracks `LibraryRowCache` invalidation manually across resize events.
- **0.9.0 API:** Fixed `VirtualList` geometry rebuilding on axis/gap change.
- **Official Reference:** GPUI Kit 0.9.0 Release Notes — "VirtualList".
- **Expected Benefit:** Seamless resize behavior with zero risk of stale item layout dimensions inside the underlying GPUI list cache.
- **Complexity:** Zero code changes in Yomu; inherited automatically upon dependency bump.
- **Regression Risk:** None.

### Finding F-05: Shortcuts Table Accessibility [IMPROVE]
- **File & Location:** `src/ui/settings_view.rs` (lines 740–770).
- **Current Implementation:** `Table`, `TableHeader`, `TableBody`, `TableRow`, `TableCell` display 16 keyboard shortcut rows.
- **0.9.0 API:** AccessKit accessibility roles for `Table` and headers.
- **Official Reference:** GPUI Kit 0.9.0 Release Notes — "Accessibility (accesskit)".
- **Expected Benefit:** Full screen-reader accessibility for Yomu's keyboard shortcut documentation without custom AccessKit code.
- **Complexity:** Zero code changes in Yomu; inherited automatically.
- **Regression Risk:** None.

### Finding F-06: Context Menu Submenu & Popover Focus [IMPROVE]
- **File & Location:** `src/app/actions.rs` (`build_book_context_menu`, `build_group_context_menu`).
- **Current Implementation:** Context menus positioned via native `.context_menu(...)`.
- **0.9.0 API:** Activatable submenu rows, left-side smart fallback positioning when screen space on right is insufficient, and popover focus clearing on dismiss.
- **Official Reference:** GPUI Kit 0.9.0 Release Notes — "Submenu & Menu", "Popover".
- **Expected Benefit:** Context menus near the right edge of small screens or tiled windows (e.g., Niri/Sway) no longer clip or fail to activate submenus; popover dismiss cleanly resets window focus.
- **Complexity:** Zero code changes in Yomu; inherited automatically.
- **Regression Risk:** None.

### Finding F-07: Icon-Only Button Accessible Names & Tooltips [IMPROVE]
- **File & Location:** `src/ui/toolbar.rs`, `src/ui/reader.rs`.
- **Current Implementation:** Toolbar and Reader HUD feature 16 icon-only buttons (`mode-groups`, `mode-volumes`, `toolbar-sort`, `reader-back-btn`, `reader-fullscreen-toggle`, etc.). Some lack explicit tooltips.
- **0.9.0 API:** Accessible name propagation from tooltips and `.aria_label(...)`.
- **Official Reference:** GPUI Kit 0.9.0 Release Notes — "Icon-only button accessible names".
- **Expected Benefit:** Comprehensive accessibility for assistive technologies and improved desktop UX via hover tooltips.
- **Complexity:** Low (add `.tooltip(...)` to remaining icon buttons).
- **Regression Risk:** None.

### Finding F-08: Touch Selection for Manga Navigation [IMPROVE]
- **File & Location:** `src/ui/reader.rs`, `src/ui/toolbar.rs`.
- **Current Implementation:** Touchpad deltas and mouse clicks handle panning and text input.
- **0.9.0 API:** Native touch selection (`set_native_touch_selection`).
- **Official Reference:** GPUI Kit 0.9.0 Release Notes — "Native Touch Selection".
- **Expected Benefit:** Improved Wayland touch interaction on 2-in-1 laptops and touchscreens.
- **Complexity:** Low.
- **Regression Risk:** None.

### Finding F-09: Global Caret Blink Cadence [IMPROVE]
- **File & Location:** `src/ui/theme.rs`.
- **Current Implementation:** Uses toolkit default caret blink.
- **0.9.0 API:** `Theme::cursor_blink` with `CursorBlink::steady()` or `CursorBlink::blinking(interval)`.
- **Official Reference:** GPUI Kit 0.9.0 Release Notes — "Cursor Blinking".
- **Expected Benefit:** Optional user preference for steady vs blinking cursor in search bar.
- **Complexity:** Low.
- **Regression Risk:** None.

### Finding F-10: Safe Manga Archive Extraction [KEEP]
- **File & Location:** `src/archive.rs`.
- **Purpose:** Pure-Rust ZIP streaming with 8 MiB bounded chunks (`INITIAL_PAGE_BUF_CAPACITY`) and 256 MiB decompression-bomb caps; single-pass sequential UnRAR extraction with dedicated worker thread (`RarSession`).
- **Reason to Keep:** Core domain capability decoupled from GUI toolkit. GPUI Kit provides no archive or comic handling.

### Finding F-11: SIMD Image Decoding & Resampling Pipeline [KEEP]
- **File & Location:** `src/imaging.rs`.
- **Purpose:** SIMD resampling via `fast_image_resize` (v6) and `zune-jpeg` (v0.5) to convert arbitrary JPEG/PNG/WebP/GIF pages to BGRA byte textures off the UI thread.
- **Reason to Keep:** Core image decompression engine. Crucial for 60/144 Hz frame pacing.

### Finding F-12: Reader Layout Math & Dual-Page Spreads [KEEP]
- **File & Location:** `src/layout.rs`, `src/ui/reader.rs`.
- **Purpose:** Pure geometric fit calculation (Page, Width, Height, Original), zoom calculation, dual-page spread detection (aspect ratio > 1.25), `SpreadPlan` with zero per-frame allocations, and LTR/RTL reading direction.
- **Reason to Keep:** Domain-specific comic geometry not offered by general-purpose UI frameworks.

### Finding F-13: Reader Navigation & Vim Motions [KEEP]
- **File & Location:** `src/vim.rs`, `src/app/keybindings.rs`.
- **Purpose:** Vim grammar token parsing (`gg`, `G`, `m[a]`, `'[a]`, counts, `:` commands) and keyboard routing.
- **Reason to Keep:** Unique manga reader keyboard interface.

### Finding F-14: Progress Sidecar Architecture [KEEP]
- **File & Location:** `src/progress.rs`, `src/library.rs`, `src/app/persistence.rs`.
- **Purpose:** Decoupled `progress.json` sidecar preventing SSD write amplification (2,184,324x reduction in write volume) with monotonic dirty-tracking.
- **Reason to Keep:** Fundamental storage durability and SSD wear mitigation.

### Finding F-15: Cross-Platform Kinetic Momentum Controller [KEEP]
- **File & Location:** `src/ui/scroll.rs`.
- **Purpose:** Smooth exponential inertia glide on Linux (Wayland/X11) and Windows where OS trackpad events stop immediately upon finger release.
- **Reason to Keep:** Upstream GPUI Kit 0.9.0 does not synthesize momentum on non-macOS platforms. Removing it would break desktop feel on Linux.

### Finding F-16: Row-Based 2D Grid Virtualization [KEEP]
- **File & Location:** `src/app/views/library.rs`, `src/ui/library_view.rs`.
- **Purpose:** Translates 2D card grid into 1D uniform rows for `v_virtual_list`.
- **Reason to Keep:** Official GPUI Kit virtualization is 1D only. Yomu's partition algorithm achieves 7,779x speedup without building a second UI framework.

### Finding F-17: Application Branding & Raster Assets [KEEP]
- **File & Location:** `src/app/desktop.rs`, `assets/logo.png`.
- **Purpose:** Raster logo (`LOGO_PNG`), BGRA byte-swapped `RenderImage`, and multi-resolution hicolor desktop integration.
- **Reason to Keep:** Preserves Yomu's visual identity. Not replaceable by `set_icon_resolver`.

### Finding F-18: Linux GNU Jemalloc Global Allocator [KEEP]
- **File & Location:** `Cargo.toml`, `src/main.rs`.
- **Purpose:** Eliminates glibc 64 MiB multi-threaded secondary arena memory retention; active `madvise(MADV_DONTNEED)` purge handling.
- **Reason to Keep:** Eliminates memory retention post-reading session. Mandatory under Rule 4.

### Finding F-19: Upstream Cross-Platform Native Momentum [WAIT]
- **Target:** Full GPUI upstream trackpad inertia on Linux and Windows.
- **Status:** Not present in 0.9.0.
- **Recommendation:** Keep `PlatformScrollController`; re-evaluate in future GPUI Kit releases.

### Finding F-20: Dynamic Third-Party Icon Theme Packs [WAIT]
- **Target:** External user-installable SVG icon packs via `set_icon_resolver`.
- **Status:** Yomu currently uses standard Lucide icons from GPUI Kit.
- **Recommendation:** Postpone introducing user icon packs until requested by users.

---

## 5. NixOS & Build Compatibility Assessment

### 5.1 System Library Compatibility
Yomu builds and tests inside a NixOS devenv shell (`devenv.nix`).

```text
NixOS System Libraries (devenv.nix)
├── Wayland ───────────── Compatible with gpui-pre 0.3.11
├── libxkbcommon ──────── Compatible with gpui-pre 0.3.11
├── vulkan-loader ─────── Compatible with gpui-pre 0.3.11
├── libGL ─────────────── Compatible with gpui-pre 0.3.11
├── fontconfig / freetype Compatible with gpui-pre 0.3.11
└── X11 Fallback Libs ─── Compatible with gpui-pre 0.3.11
```

- **Verdict:** No new C libraries or Nix packages are required. `devenv.nix` and `flake.nix` remain 100% valid.

### 5.2 Allocator Compatibility (`tikv-jemallocator 0.6.1`)
- The dependency bump to GPUI Kit 0.9.0 and `gpui-pre 0.3.11` does not affect `tikv-jemallocator 0.6.1`.
- The profile overrides in `Cargo.toml`:
  ```toml
  [profile.dev.package.tikv-jemalloc-sys]
  opt-level = 2
  [profile.test.package.tikv-jemalloc-sys]
  opt-level = 2
  ```
  must be preserved to prevent NixOS `-D_FORTIFY_SOURCE=2` compilation failures.

### 5.3 Rust Toolchain Compatibility
- Yomu specifies `rust-version = "1.92"`.
- `devenv.nix` supplies `rustc 1.99.0` (stable).
- GPUI Kit 0.9.0 requires Rust $\ge 1.92$. Full toolchain compatibility confirmed.

---

## 6. Migration Risk Analysis & Mitigation

| Risk Area | Severity | Likelihood | Mitigation Strategy |
|---|:---:|:---:|---|
| **Test Failures from Wall-Clock Sleep** | Medium | High | Replace `std::thread::sleep` in `src/tests.rs` with `cx.executor().advance_clock(...)`. |
| **Vendored Patch Merge Conflicts** | Medium | Medium | Rebase or eliminate `vendor/gpui-component` patch; verify `Selectable` and test menu selectors. |
| **Theme Deserialization Incompatibilities** | Low | Low | Yomu parses via `serde_json::from_value`; verified schema matches built-in Catppuccin and Gruvbox. |
| **Wayland Compositor Rendering Regressions** | Low | Low | Headless test suite validates geometry; perform real Wayland visual inspection before tagging release. |
| **Memory Retention Regression** | Low | Low | Jemalloc global allocator remains intact; execute 15-cycle memory plateau verification post-upgrade. |

---

## 7. Audit Conclusion & Next Steps

Upgrading to GPUI Kit 0.9.0 is **technically sound, beneficial, and low-risk** for Yomu.
- **Breaking API Changes Affecting Yomu:** Only 1 concrete breaking change: test animation timing requiring `advance_clock` in place of `std::thread::sleep`.
- **Zero Production Source Code Breaking Changes:** Yomu does not call `Window::text_system()` and does not use direct `ThemeConfig` struct literals.
- **Direct Benefits:** Improved VirtualList geometry invalidation on resize, Table accessibility in settings, smart submenu edge positioning, popover focus clearing, and clean touch selection.
- **Domain Invariants Preserved:** All manga archive processing, SIMD image decompression, spread layout plans, Vim motions, progress sidecars, kinetic momentum, and jemalloc memory management remain strictly application-owned.

The detailed execution sequence, phase gates, and rollback criteria are documented in [`GPUI_KIT_09_MIGRATION_PLAN.md`](GPUI_KIT_09_MIGRATION_PLAN.md).
