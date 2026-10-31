# Agent Guide

This file is the authoritative, concise engineering guide for coding agents working on Yomu. Read [docs/agent-handoff.md](docs/agent-handoff.md) for the active operational snapshot, [docs/architecture.md](docs/architecture.md) for architectural invariants and module boundaries, [docs/development.md](docs/development.md) for build and profiling recipes, and [README.md](README.md) for user-facing features and keybindings.

---

## 1. Project at a Glance

Yomu is a minimal, keyboard-first, high-performance desktop manga and comic reader built in Rust 2024 on **GPUI Kit 0.8.1** (`gpui-pre = 0.3.10` runtime) with `#![forbid(unsafe_code)]`. It imports CBZ, CBR, ZIP, and RAR archives and image folders, decodes/scales pages on background executors, and persists metadata and progress locally.

- **Primary Target:** Linux (Wayland-native with X11 fallback; Nix/devenv runtime).
- **Secondary Targets:** macOS and Windows compatibility maintained through platform abstractions.
- **UI Architecture:** Modern macOS HIG two-pane design (persistent 240px sidebar, context-aware 52px toolbar, responsive content views, edge-to-edge immersive reader).
- **Three-Tier Architectural Boundary:**
  1. *Core Domain Layer (`src/lib.rs`):* GUI-independent Rust combining pure algorithms (layout geometry, animation physics, Vim token parsing, search filtering, selection state) with filesystem and persistence operations (archive unpacking, SIMD image decoding, JSON storage).
  2. *UI Presentation Layer (`src/ui/`):* Reusable GPUI Kit views, components, and overlays.
  3. *Application Orchestration (`src/main.rs` / planned `src/app/`):* Window lifecycle, state routing, async background scheduling, and input dispatch.
- **User-Owned Files:** `problems.md` is user-owned. **Never edit, modify, or delete it.**

---

## 2. Build and Verification Commands

Execute inside the development shell (`devenv shell` or `nix develop`):

```sh
# 1. Format check
cargo fmt --all -- --check

# 2. Fast type check
cargo check

# 3. Strict Clippy lint check
cargo clippy --all-targets -- -D warnings

# 4. Core domain unit tests (100 tests)
cargo test --lib

# 5. Binary-target UI tests (102 tests)
cargo test --bin yomu

# 6. Opt-in performance benchmarks
cargo test perf_ -- --ignored --nocapture
```

Always run the full test suite (**202 active tests**: 100 library unit tests, 102 binary-target UI tests) passing cleanly before completing an engineering phase. Binary-target UI tests execute under GPUI's simulated headless platform; never claim visual verification on Wayland unless manually inspected on a running Wayland compositor.

---

## 3. Engineering Philosophy (Ponytail)

Follow the Ponytail engineering philosophy: prefer the simplest correct, maintainable, secure, and performant solution.

- **Investigate before implementing:** Inspect the actual source code, dependencies, and installed GPUI Kit APIs before writing code.
- **Prefer standard solutions:** Use the Rust standard library, official GPUI Kit components, and existing abstractions before inventing custom mechanisms.
- **Avoid unnecessary dependencies:** Reuse existing dependencies. Never introduce new crates without compelling, demonstrable justification (e.g. use standard library collections before reaching for third-party map crates).
- **Avoid overengineering:** No speculative traits, redundant wrappers, premature generic frameworks, or unnecessary layers of indirection.
- **Preserve architectural boundaries:** Keep the core domain layer strictly independent of GPUI.
- **Optimize based on evidence:** Optimize hot paths based on benchmark evidence; avoid unnecessary allocations in hot paths while avoiding speculative micro-optimizations, and honor background scheduling.
- **Make focused changes:** Avoid broad rewrites, unrelated formatting churn, or modifying stable working code.
- **Test meaningful changes:** Add regression unit or integration tests for every behavioral change.
- **Document technical debt:** Never leave unresolved debt undocumented.

### Skills Usage
- `gpui-kit` & `gpui-kit-design-guides`: Authoritative reference for desktop components, layouts, spacing, and styling.
- `emil-design-eng`: Motion principles and UI polish adapted for native desktop applications.
- `rust-best-practices` & `rust-async-patterns`: Idiomatic Rust patterns, borrowing discipline, and concurrency.
- `m10-performance`: Evidence-based benchmarking, flamegraphs, and allocation profiling.

---

## 4. Critical Architectural Invariants

1. **User Data Preservation:**
   - Deleting books, folders, or collections from the UI **never** deletes original comic files from disk.
   - `library.json` is user data. Parse leniently using Serde defaults. On parse failure, preserve a timestamped `.corrupt` backup before initializing defaults.
2. **Reading Order Independence:**
   - Page indices are strictly zero-based and ordered in physical archive sequence. Reading direction (LTR vs RTL) alters only visual motion and navigation mapping, never archive indexing or caching logic.
3. **Off-UI-Thread Processing & Adaptive Memory Policy:**
   - Never decode or scale images on the UI thread. Use background executors and respect the adaptive decoded image memory budget (`pagecache::system_memory_budget()`, which defaults to 1.0 GiB (`BUDGET_BYTES`) and scales between 512 MiB and 3.0 GiB based on 1/8th of detected host RAM). Release unused GPU textures via `window.drop_image` in `Yomu::render`.
4. **No Synchronous Filesystem I/O on UI Thread:**
   - Never perform synchronous disk or procfs reads (`std::fs::read_to_string`, `fs::metadata`, `/proc/meminfo`) inside the UI render loop, animation step, or scroll handlers. Cache static metrics once at startup.
5. **Asynchronous Task Cancellation Safety:**
   - Background prefetch tasks cancelled during fast scrolling must be cleanly aborted without being marked as permanent decode errors in `reader.failed`. A cancelled page must remain requestable when the user navigates back to it.
6. **Library Search is Transient:**
   - Search filtering operates only on rendered cards. Bulk selection and library operations act on the full underlying library order.
7. **Continuous & Webcomics Modes:**
   - Single-page is the default. Continuous vertical reading is persisted in `Settings`. Webcomics mode is an override forcing continuous zero-gap layout; disabling it restores the saved preference.
8. **No Silent Workarounds:**
   - Preserve clear `anyhow::Result` error context for filesystem, archive, and image decode failures.

---

## 5. GPUI & GPUI Kit Implementation Rules

- **Authoritative Framework:** Use **GPUI Kit 0.8.1** (`gpui-kit` crate) as the single implementation authority. Verify APIs against the installed version; never invent methods.
- **Official Components First:** Prefer official components (`Button`, `ButtonGroup`, `Input`, `DropdownMenu`, `Kbd`, `ScrollableElement`) over raw `div()` recreation.
- **No Web Habits:** Never introduce CSS, HTML, JavaScript, web-framework patterns, inline string styling, or ad-hoc style workarounds. Use GPUI Kit typed builder methods (`.bg()`, `.rounded()`, `.text_color()`, `.flex()`, `.gap()`).
- **Stable Identity:** Assign domain-derived, stable `ElementId`s to interactive and list elements rather than transient index numbers.
- **Render Loop Discipline:** Keep `Render::render` free of state machine side-effects, synchronous I/O, entity recreation, and unnecessary per-frame closure or collection allocations.

---

## 6. Animation & Motion Standards

- **Restrained & Purposeful:** Animations exist solely to provide spatial orientation and responsive tactile feedback. Never animate purely for decoration.
- **Interruptible & Responsive:** Animations must use analytical time delta ($dt$) formulas or GPUI Kit official spring animations. An in-flight animation must immediately redirect or settle when new input arrives, never queueing behind previous steps.
- **Honor Global Settings:** All non-essential animations must strictly respect the global `disable_animations` setting and system reduced-motion preferences (`motion::detect_reduced_motion()`), snapping immediately to final targets (0 ms duration) when disabled.
- **Recommended Curves & Durations (Starting Guidelines):**
  Treat these values as initial reference points to evaluate against interaction ergonomics and feel, rather than rigid implementation requirements:
  - *Sidebar toggle:* ~160–180 ms cubic ease-out.
  - *Settings pane / modal fade:* ~120–140 ms subtle opacity drift.
  - *Reader page slide:* ~160 ms normal, ~90 ms interrupted (`PAGE_TURN_INTERRUPTED_SECS`).

---

## 7. Cross-Platform Compatibility

- **Linux (Primary):** Wayland-native first, X11 fallback. Respect client-side vs server-side window decorations (`Decorations::Client`). Do not block or drop keystrokes on modifier combinations.
- **macOS:** Respect AppKit native traffic lights padding (`pl(px(80.))`) and native momentum pass-through in `scroll.rs`. Use `⌘` command key mapping.
- **Windows:** Compile native application resources via `build.rs` and `embed-resource`. Normalize archive file path separators (`\` vs `/`). Use `Ctrl` key mapping.

---

## 8. Before Handing Work Back

- Summarize modified files, responsibilities changed, and tests or benchmarks executed.
- If UI behavior relies on a real Wayland compositor, explicitly state whether it was visually checked.
- Never claim commands passed unless they were run and exited with code 0.
