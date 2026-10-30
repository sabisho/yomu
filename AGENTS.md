# Agent guide

This file is the starting point for coding agents working on yomu. Read [docs/agent-handoff.md](docs/agent-handoff.md) for the current implementation snapshot and product decisions, [README.md](README.md) for user-facing behavior and shortcuts, and [docs/architecture.md](docs/architecture.md) for code boundaries and invariants. Read [docs/ui-redesign-plan.md](docs/ui-redesign-plan.md) before changing the UI or GPUI Kit integration.

## Project at a glance

yomu is a Rust desktop comic reader built on GPUI Kit (GPUI). It imports CBZ/ZIP archives and image folders, stores library metadata locally, and decodes/scales pages off the UI thread. It targets Linux (Wayland, with X11 fallback) and uses Nix/devenv for native build dependencies. The user-provided UI reference is `gpui-kit_inspiration.jpg`; current redesign progress and remaining work are recorded in `docs/agent-handoff.md`.

## Start here

- Core library and unit tests: `src/lib.rs` and modules under `src/`.
- Window, event handling, overlays, rendering, and gpui UI tests: `src/main.rs`.
- Build metadata and Rust dependencies: `Cargo.toml`, `Cargo.lock`.
- Development environments: `devenv.nix`, `devenv.yaml`, `flake.nix`.
- Nix package: `nix/package.nix`.
- `problems.md` is user-owned. Do not edit it as part of feature work; verify feature status against source and README.

## Build and run

On NixOS, enter the configured shell and run the app:

```sh
devenv shell
cargo run
```

`nix develop` is the alternative shell. The package is Linux-only and needs Wayland/Vulkan runtime libraries. Rust edition is 2024 and the minimum compiler is 1.92 (required by GPUI Kit 0.7.3's dependency graph). See README for NixOS graphics and portal prerequisites.

Useful project commands:

```sh
cargo fmt --check
cargo check
cargo test
cargo test perf_ -- --ignored --nocapture
```

The last command runs opt-in performance measurements, not the normal suite. UI tests use gpui's simulated platform and do not validate appearance on a real compositor. Run focused tests while iterating, then the full suite for behavior changes when verification is requested or appropriate.

## Engineering Philosophy (Ponytail)

Follow the Ponytail engineering philosophy: prefer the simplest correct, maintainable, secure, and performant solution. Avoid unnecessary complexity, but never sacrifice engineering quality merely to reduce code.

- **Investigate before implementing:** Understand the existing architecture, dependencies, GPUI Kit capabilities, and relevant code paths before making changes.
- **Prefer native solutions:** Use Rust standard library functionality, existing project abstractions, GPUI/GPUI Kit components and official APIs before introducing custom implementations.
- **Avoid unnecessary dependencies:** Reuse existing dependencies where practical. Add a new dependency only when it provides meaningful value that justifies its maintenance cost.
- **Avoid overengineering:** Do not introduce unnecessary traits, wrappers, abstractions, configuration layers, generic frameworks, or premature extensibility.
- **Preserve architectural boundaries:** Keep core logic independent of GPUI and respect the existing modular architecture.
- **Optimize based on evidence:** Prioritize responsiveness, smooth rendering, efficient resource management, and measurable performance. Do not trade performance for fewer lines of code.
- **Make focused changes:** Avoid unrelated refactoring, broad rewrites, and unnecessary modifications to working functionality.
- **Test meaningful changes:** Add focused regression tests where appropriate. Verify behavior, compilation, and performance when relevant.
- **Review before completion:** Check for regressions, duplicated logic, unnecessary complexity, security issues, resource leaks, and incomplete error handling.
- **Document technical debt:** If a necessary improvement cannot be completed safely, document the reason, impact, and recommended follow-up rather than silently leaving it unresolved.

### Project-specific priority

Ponytail principles are engineering guidelines, not permission to override Yomu's established requirements.

When instructions conflict, prioritize:
1. Explicit user requirements and product decisions.
2. Yomu's documented architectural invariants, data-preservation requirements, and platform compatibility.
3. Correctness, security, accessibility, and performance.
4. Ponytail's simplicity and minimalism preferences.

Never remove necessary caching, preloading, validation, error handling, tests, or abstractions simply to reduce code size. Never replace official GPUI Kit functionality with ad-hoc custom implementations when an appropriate native capability exists or is planned.

Do not compromise Linux, Windows, or macOS compatibility for platform-specific convenience.

## Working conventions

- The UI follows a macOS HIG two-pane design (`src/ui/sidebar.rs` + `src/ui/toolbar.rs` + content router `MainView`). Persistent sidebar width is 240px; UI integration tests clicking library cards must offset mouse coordinates accordingly (`x = 240.0 + padding + card_offset`).
- Views are modularized under `src/ui/`: `sidebar`, `toolbar`, `continue_reading`, `details`, `collections_view`, `settings_view`, `components`, `overlays`.
- Collections management lives in `src/collections.rs` and `src/library.rs`. Deleting collections or library entries must never remove user source files from disk.
- Keep reusable logic independent of gpui. Put parsing, persistence, archive handling, geometry, selection rules, animation math, and cache planning in their existing core modules.
- Keep filesystem, archive, image decode, and resize failures as `anyhow::Result` with useful context. Avoid silently replacing unreadable user data; library loading preserves a corrupt file as a backup.
- Treat `library.json` as user data. Add serde defaults for new fields, read settings leniently where possible, and add a migration when stored meaning or defaults change. Preserve old data and document format changes.
- Page indices are zero-based internally and ordered in reading order regardless of RTL. Direction maps input and visual travel; it must not reverse archive ordering or cache indices.
- Continuous vertical mode is an opt-in persisted reader setting. Keep it working with page-number jumps, marks, zoom, input scrolling, saved progress, and bounded visible-page caching; single-page mode remains the default.
- Library search is transient UI state. `/` and `Ctrl+F` activate it; it matches a case-insensitive substring of title, source path, or series and filters only the rendered cards. Bulk selection and actions still operate on the full library.
- Reader key `v` toggles continuous vertical mode; `:continuous [on|off]` and the Settings option remain available. Single-page mode stays the default.
- Webcomics mode is separately persisted; it forces continuous layout with zero gap between pages and restores the saved continuous/single-page preference when disabled. Reader key `w` and `:webcomics [on|off]` control it.
- Series is optional persisted book metadata (missing values are unassigned). Series grouping is an opt-in persisted library setting; preserve normal grid mode, search, and bulk assignment behavior when changing it.
- Do not do expensive page decode/resizing on the UI thread. Keep decoded pages bounded by the cache policy and free gpui image resources when no longer used.
- Preserve accessibility/input behavior and the README's documented shortcuts when editing UI interactions.
- Vendor patches under `vendor/` are deliberate compatibility fixes; inspect their comments and `Cargo.toml` rationale before changing dependency patches.
- Avoid unrelated formatting or broad UI rewrites when addressing a focused issue.

## Before handing work back

Summarize changed files, behavior, and any tests or commands actually run. If UI behavior depends on a real Wayland compositor, state whether it was visually checked. Do not claim commands passed unless they were run.
