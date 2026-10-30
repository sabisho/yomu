# Agent Guide

This file is the primary starting point for coding agents working on Yomu. Read [docs/agent-handoff.md](docs/agent-handoff.md) for the latest implementation status and active tasks, [docs/architecture.md](docs/architecture.md) for module boundaries and architectural invariants, [docs/development.md](docs/development.md) for build recipes, and [README.md](README.md) for user-facing features and keybindings.

---

## Project at a Glance

Yomu is a minimal, keyboard-first, high-performance desktop manga reader built in Rust 2024 on **GPUI Kit 0.8.0** with `#![forbid(unsafe_code)]`. It imports CBZ, CBR, ZIP, and RAR archives and image folders, decodes/scales pages on background executors, and persists metadata and progress locally.

- **Target platform:** Linux (Wayland-native with X11 fallback; Nix/devenv runtime). Abstraction layers maintain macOS and Windows compatibility.
- **UI Architecture:** Modern macOS HIG two-pane design (persistent 240px sidebar, context-aware 52px toolbar, responsive content views, edge-to-edge immersive reader).
- **Core policy:** Domain logic lives in pure, GPUI-independent modules under `src/`; UI views live under `src/ui/`; window orchestration and background scheduling live in `src/main.rs`.

---

## Start Here

- **Core domain logic & unit tests:** `src/lib.rs` and pure modules under `src/`.
- **UI components & view screens:** `src/ui/` (`sidebar`, `toolbar`, `library_view`, `continue_reading`, `details`, `collections_view`, `history_view`, `settings_view`, `overlays`, `reader`, `components`, `theme`).
- **Window orchestration & UI tests:** `src/main.rs` and `src/tests.rs`.
- **Cargo manifest:** `Cargo.toml`.
- **Dev environments & package:** `devenv.nix`, `flake.nix`, `nix/package.nix`.
- **User-owned files:** `problems.md` is user-owned. **Do not edit or delete it as part of feature work.** Verify claims against actual source.

---

## Build and Verification Commands

Inside the development shell (`devenv shell` or `nix develop`):

```sh
# 1. Format check
cargo fmt --all -- --check

# 2. Fast type check
cargo check

# 3. Strict Clippy check
cargo clippy --all-targets -- -D warnings

# 4. Core domain unit tests (100 tests)
cargo test --lib

# 5. UI integration tests (101 tests)
cargo test --bin yomu

# 6. Opt-in performance benchmarks
cargo test perf_ -- --ignored --nocapture
```

Always run the full test suite (201 active tests) before completing an engineering phase. UI tests execute under GPUI's simulated headless platform and do not validate physical Wayland compositor appearance; do not claim visual verification on Wayland unless manually checked on a real compositor.

---

## Engineering Philosophy (Ponytail)

Follow the Ponytail engineering philosophy: prefer the simplest correct, maintainable, secure, and performant solution. Avoid unnecessary complexity, but never sacrifice engineering quality merely to reduce code.

- **Investigate before implementing:** Inspect the actual code, dependencies, and GPUI Kit APIs before modifying.
- **Prefer native solutions:** Use Rust standard library, GPUI Kit official components, and existing abstractions before inventing custom mechanisms.
- **Avoid unnecessary dependencies:** Reuse existing dependencies. Never introduce new crates without compelling, demonstrable justification.
- **Avoid overengineering:** No speculative traits, redundant wrappers, premature generic frameworks, or excessive indirection.
- **Preserve architectural boundaries:** Keep core domain logic strictly independent of GPUI.
- **Optimize based on evidence:** Keep hot paths allocation-free; preserve background decoding, bounded caching, and frame pacing.
- **Make focused changes:** Avoid broad rewrites, unrelated formatting, or churning working code.
- **Test meaningful changes:** Add regression unit/integration tests for every behavioral modification.
- **Document technical debt:** Never leave unresolved debt undocumented.

### Project-Specific Priority Order
1. Explicit user instructions and product decisions.
2. Documented architectural invariants, data preservation, and platform compatibility.
3. Correctness, security, accessibility, and performance.
4. Simplicity and minimalism preferences.

---

## Critical Architectural Invariants

1. **User Data Preservation:**
   - Deleting books, folders, or collections from the UI **never** removes source files from disk.
   - `library.json` is user data. Use serde defaults for new fields, parse leniently, and add migrations when schema meaning changes. On parse failure, preserve a corrupt backup file before initializing defaults.
2. **Reading Order Independence:**
   - Page indices are strictly zero-based and ordered in physical archive sequence. RTL mode changes input mapping and visual slide direction, never archive or cache indices.
3. **Off-UI-Thread Processing:**
   - Never decode or scale images on the UI thread. Use background executors and respect `pagecache::BUDGET_BYTES` (1.0 GiB default). Release unused GPU textures via `window.drop_image` in `Yomu::render`.
4. **Library Search is Transient:**
   - Search filtering operates only on rendered cards. Bulk selection and library operations continue to act on the full library order.
5. **Continuous & Webcomics Modes:**
   - Single-page is the default. Continuous vertical reading is persisted in `Settings`. Webcomics mode is an override forcing continuous zero-gap layout; disabling it restores the saved continuous/single-page preference.
6. **No Silent Workarounds:**
   - Keep filesystem, archive, and image failures as `anyhow::Result` with clear error context.

---

## Before Handing Work Back

- Summarize modified files, responsibilities changed, and tests or benchmarks executed.
- If UI behavior relies on a real Wayland compositor, explicitly state whether it was visually checked.
- Never claim commands passed unless they were run and exited with code 0.
