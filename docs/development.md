# Development Guide

This guide covers the development workflow, build environment, testing conventions, and debugging procedures for Yomu.

---

## 1. Development Environment

Yomu targets Linux (Wayland-first with X11 fallback), requiring Vulkan graphics runtime libraries and Wayland/X11 client libraries.

### Quick Start with Devenv / Nix

```sh
# Enter development shell (provides Rust 2024, GPUI Kit dependencies, Vulkan, CMake, pkg-config)
devenv shell

# Run in debug mode
cargo run

# Run with release optimizations
cargo run --release
```

Alternatively, use standard Nix flakes:

```sh
nix develop
```

### NixOS Prerequisites

Ensure your host configuration enables graphics drivers and an XDG desktop portal (required for native file dialogs):

```nix
hardware.graphics.enable = true;          # Vulkan runtime drivers (/run/opengl-driver)
xdg.portal.enable = true;                 # Native portal file dialogs
xdg.portal.extraPortals = [ pkgs.xdg-desktop-portal-gtk ]; # Or GNOME/KDE portal
```

---

## 2. Standard Development Commands

Always verify changes with the project's quality verification suite:

```sh
# 1. Format check
cargo fmt --all -- --check

# 2. Fast type check
cargo check

# 3. Strict Clippy lint check
cargo clippy --all-targets -- -D warnings

# 4. Run core library unit tests (100 tests)
cargo test --lib

# 5. Run binary-target UI tests (102 tests)
cargo test --bin yomu

# 6. Run all active tests across targets (202 tests)
cargo test

# 7. Run opt-in performance benchmarks
cargo test perf_ -- --ignored --nocapture
```

> [!NOTE]
> Binary-target UI tests execute under GPUI's simulated headless platform (`gpui_kit::TestAppContext`) within the binary target runner (`cargo test --bin yomu`). They do not require an active display or compositor. They validate layout, interactions, animations, and state transitions, but do not validate visual appearance on a physical Wayland compositor.

---

## 3. Engineering Workflows & Change Recipes

### Adding or Modifying a User Preference
1. Add the field and default to `Settings` in `src/library.rs`.
2. Ensure `Settings::from_value` parses missing or legacy fields leniently with sensible fallbacks.
3. Expose the setting in the appropriate tab within `src/ui/settings_view.rs`.
4. If the field alters stored semantics of previous versions, bump `LIBRARY_VERSION` and add a migration test.
5. Update `README.md` and `docs/architecture.md`.

### Adding a Reader Command or Keybinding
1. Map the raw key token or command string in `src/vim.rs` to an `Action` variant. Keep parsing GUI-independent.
2. Update `command_suggestions` in `src/ui/overlays.rs` if the command supports auto-completion.
3. Handle the resulting `Action` in `Yomu::apply` in `src/main.rs`.
4. Add unit test coverage in `src/vim.rs` and update the keyboard table in `README.md`.

### Adding or Updating a View Screen
1. Create or modify the view module under `src/ui/` (e.g., `src/ui/<screen>_view.rs`).
2. Define a pure `Props` struct borrowing needed domain models and scroll handles.
3. Pass event callbacks (`on_click`, `on_select`, `on_action`) that take `(&mut Window, &mut App)` or typed arguments, decoupling presentation from `Entity<Yomu>`.
4. Expose the submodule in `src/ui/mod.rs` and route it via `Yomu.active_view` (`MainView`).

### Background Decoding and Image Memory
1. Keep archive reading, image decoding, and SIMD resizing strictly off the UI thread via `cx.background_executor()`.
2. Decoded textures must respect the adaptive memory budget policy (`pagecache::system_memory_budget()`, defaulting to 1.0 GiB (`BUDGET_BYTES`) and scaling between 512 MiB and 3.0 GiB based on detected host RAM).
3. Old textures must be collected and dropped via `window.drop_image` in `Yomu::render`.
4. **Never execute synchronous filesystem reads (`/proc/meminfo`, file metadata) on the UI thread.** Cache static system metrics at startup.
5. Handle prefetch task cancellation gracefully without marking aborted tasks as permanent decode errors.

---

## 4. Performance Engineering & Benchmarks

Benchmark regression tests measure CPU time and throughput:

```sh
cargo test perf_ -- --ignored --nocapture
```

Measurable benchmark targets (measured under the headless Linux x86_64 test harness):
- **Slide & glide frame update:** ~20 ns per frame (analytical motion math).
- **Animated slide paint time:** Mean < 3.2 ms, p95 < 4.0 ms (providing headroom for 144 Hz 6.9 ms / 60 Hz 16.6 ms display budgets).
- **SIMD decode & resize (3000x4500 JPEG):** ~300–500 ms at standard tiers on background threads (release profile).

These targets serve as empirical benchmarks under documented test conditions rather than universal guarantees across arbitrary hardware.

### Performance Guidelines
1. **Avoid Unnecessary Allocations in Hot Paths:** Minimize transient heap allocations, closure creation, and buffer cloning within per-frame render routines unless justified by measured performance or readability.
2. **Scalable Item Lookups:** Avoid repeated linear scans over growing collections in UI rendering passes where an indexed map or keyed lookup provides $O(1)$ scaling.
3. **Evidence-Based Optimization:** Profile and benchmark before adding complexity; avoid speculative micro-optimizations that obscure code without demonstrable benefit.
