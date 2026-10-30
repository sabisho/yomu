# Agent Handoff

This document is the current operational snapshot for coding agents working on Yomu. Read it after [AGENTS.md](../AGENTS.md). For structural invariants and module boundaries, see [docs/architecture.md](architecture.md); for user-facing features and keybindings, see [README.md](../README.md).

---

## 1. Current Implementation Status

Yomu is a high-performance desktop manga and comic reader built in Rust 2024 using **GPUI Kit 0.8.0** (`gpui-pre = 0.3.9` runtime). It operates with `#![forbid(unsafe_code)]` in safe Rust.

- **Primary Target:** Linux (Wayland-native with X11 fallback; Nix/devenv native packaging).
- **Secondary Targets:** macOS and Windows compatibility maintained through abstraction layers.
- **UI Architecture:** Modern macOS HIG two-pane design (persistent 240px sidebar, context-aware 52px toolbar, responsive content views, edge-to-edge immersive reader).
- **Test Baseline:** **201 tests passing** (100 core library tests in `src/lib.rs`, 101 binary UI integration tests in `src/main.rs`, plus 3 opt-in performance benchmarks).
- **Clean Toolchain:** Zero compiler errors, zero warnings under `cargo check` and `cargo clippy --all-targets -- -D warnings`, fully formatted under `cargo fmt`.

---

## 2. Recently Completed Work (Phases 0–3)

### Phase 0: Baseline & Test Discrepancy Resolution
- Resolved the 98 vs 99 UI test discrepancy: confirmed 102 total binary runner tests (98 in `tests.rs` + 3 in `ui/theme.rs` + 1 in `ui/reader.rs` = 99 active + 3 ignored benchmarks).
- Verified equivalence between duplicate library rendering in `src/main.rs` and `src/ui/library_view.rs`.

### Phase 1: Consolidate Library Rendering
- Fully wired `src/ui/library_view.rs` for rendering book cards, folder cards, and list rows.
- Eliminated 620 lines of redundant duplicate rendering logic from `src/main.rs`.

### Phase 2: Lifecycle, Startup & Vendor Cleanup
- **Search Input Lifecycle:** Moved `search_input` initialization and event subscription from `render()` into `Yomu::with_dirs`. Prevents duplicate subscriptions and eliminates entity recreation in render loops.
- **Desktop Integration:** Removed automatic filesystem writes and `update-icon-cache` spawns on application launch. Created safe, non-destructive `install_desktop_integration_to` exposed via `--install-desktop` / `--install-icons` CLI flags with `--force` protection.
- **Vendor Cleanup:** Removed dead vendored crates `vendor/xattr` and `vendor/stacksafe-macro` (-1,600 lines) after confirming Cargo dependency resolution and pure-Rust decoders.
- **Documentation Correction:** Corrected misleading claims regarding pure-Rust decoders vs `unrar` C++ library bindings.
- Added regression tests `test_search_input_lifecycle` and `test_desktop_integration_install` (increasing binary tests to 101).

### Phase 3: Architectural Modularization of `main.rs`
- **Overlays Extraction (`src/ui/overlays.rs`):** Extracted confirm dialogs, properties dialog, series dialog, collection dialog, context menus, floating command prompt (`render_command_box`, `command_suggestions`), and reader help modal (`help_overlay`, `HELP`). Introduced `OverlayStyle` to bundle visual parameters cleanly.
- **History View Extraction (`src/ui/history_view.rs`):** Extracted reading history screen into a dedicated view module, matching the pattern of other sidebar screens.
- **Dead Code Removal:** Excised unused `icon_button` and redundant imports.
- **Net Result:** Reduced `src/main.rs` from 6,809 lines to 5,360 lines (-1,449 lines / -21.3%) without altering rendering behavior or performance.

---

## 3. Known Limitations & Technical Debt

1. **Wayland Compositor Visual Verification:**
   - Headless simulated GPUI integration tests validate layout, interaction, focus, and state transitions, but do not validate visual appearance (translucent surfaces, backdrop blur, compositor shadows) under real Wayland compositors (e.g., Niri, Sway, GNOME Mutter, KDE KWin).
   - *Rule for agents:* Never claim visual verification on Wayland unless manually inspected on a real desktop session.
2. **Remaining Congestion in `src/main.rs`:**
   - `src/main.rs` still hosts the two-pane shell router (`render_two_pane_shell`), reader input gesture handling (`on_wheel`, `on_pinch`, `drag_*`), and keyboard vim action dispatching (`on_key`, `apply`).
3. **Cross-Platform Scrolling:**
   - Linux and Windows rely on `PlatformScrollController`'s kinetic inertia glide loop because GPUI does not yet synthesize momentum events on non-macOS platforms.
   - *Action required:* Monitor official GPUI Kit releases for native cross-platform momentum support.

---

## 4. Next Recommended Engineering Tasks

Refer to [ROADMAP.md](../ROADMAP.md) for prioritized upcoming work:

1. **Double-Page Spread Reading Mode:**
   - Support reading two pages side-by-side (portrait manga pairs) with automatic single-page display for wide landscape covers.
2. **Library Sorting Options:**
   - Provide sorting controls (Title, Last Read, Date Added, Page Count) in the library toolbar.
3. **CB7 Archive Support:**
   - Integrate 7z archive reader support alongside existing CBZ, CBR, ZIP, and RAR handlers.
4. **Upstream GPUI Momentum Migration:**
   - Replace `scroll.rs` custom kinetic decay once upstream GPUI provides official Linux/Windows momentum events.
