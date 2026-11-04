# Yomu — Authoritative Agent Instructions & Engineering Guide

This file is the single, authoritative, non-negotiable engineering standard for AI coding agents (Google Antigravity, Gemini, and any future agents) working on the Yomu repository.

Read [GEMINI.md](GEMINI.md) for agent discovery context, [docs/architecture.md](docs/architecture.md) for architectural invariants and module topology, [docs/development.md](docs/development.md) for build, validation, and profiling recipes, and [docs/agent-handoff.md](docs/agent-handoff.md) for the active operational snapshot.

---

## 1. Project at a Glance

Yomu is a minimal, keyboard-first, high-performance desktop manga and comic reader built in safe Rust 2024 on **GPUI Kit 0.8.1** (`gpui-pre = 0.3.10` runtime) under `#![forbid(unsafe_code)]`. It imports CBZ, CBR, ZIP, and RAR archives as well as image directories, decodes and scales pages on background executors, and persists metadata and progress locally.

- **Primary Target:** Linux (Wayland-native with X11 fallback; Nix/devenv runtime).
- **Secondary Targets:** macOS and Windows compatibility maintained through platform abstractions.
- **UI Architecture:** Modern macOS HIG two-pane design (persistent 240px sidebar, context-aware 52px toolbar, responsive content views, edge-to-edge immersive reader).
- **Three-Tier Architectural Boundary:**
  1. *Core Domain Layer (`src/lib.rs` and `src/*.rs`):* GUI-independent Rust combining pure algorithms (layout geometry, animation physics, Vim token parsing, search filtering, selection state machines) with filesystem and persistence operations (archive unpacking, SIMD image decoding, JSON storage). See [src/AGENTS.md](src/AGENTS.md).
  2. *UI Presentation Layer (`src/ui/`):* Reusable GPUI Kit views, components, dialogs, and overlays. See [src/ui/AGENTS.md](src/ui/AGENTS.md).
  3. *Application Orchestration (`src/app/` and `src/main.rs`):* Window lifecycle, state routing, async background scheduling, input dispatch, and persistence coordination. See [src/app/AGENTS.md](src/app/AGENTS.md).
- **Test Baseline:** **390 active tests** passing (171 core library unit tests in `src/lib.rs`, 219 binary-target UI tests in `src/main.rs`, plus 16 opt-in performance benchmarks; 406 total).
- **User-Owned Files:** `problems.md` is user-owned. **Never edit, modify, or delete it.**

---

## 2. Seven Mandatory Engineering Principles

These principles are non-negotiable and apply to every implementation, refactor, review, and bug fix.

### Rule 1 — Absolutely No Unsafe Rust
Yomu must contain zero unsafe Rust code. Agents must never introduce:
- `unsafe` blocks.
- `unsafe` functions.
- Unsafe trait implementations.
- Other unsafe Rust constructs.

**Mandatory Requirements:**
- Preserve `#![forbid(unsafe_code)]` at the root of `src/lib.rs` and `src/main.rs`.
- Never weaken, bypass, or remove the unsafe code prohibition.
- Never use unsafe Rust merely to micro-optimize performance or access foreign functionality.
- Prefer safe Rust standard library abstractions and official APIs.
- If a requested capability appears to require unsafe code, stop, explain the limitation, and propose a safe alternative.

### Rule 2 — Idiomatic Rust Everywhere
All Rust code must adhere to modern, idiomatic Rust 2024 practices.

**Mandatory Requirements:**
- **Ownership & Borrowing:** Use ownership and borrowing correctly; prefer borrowed references (`&str`, `&[T]`, `&Path`) over owned allocations (`String`, `Vec<T>`, `PathBuf`) in function parameters.
- **Type Safety:** Model domain invariants using expressive enums, newtypes, and standard traits (`From`, `AsRef`, `Display`).
- **Error Handling:** Handle `Result` and `Option` explicitly using `?`, pattern matching, or `anyhow::Result` context; never use unhandled `.unwrap()` in production paths.
- **Standard Library First:** Prefer standard library functionality (`std::sync::OnceLock`, `std::collections`, `std::sync::atomic`) before considering external dependencies.
- **Clean Allocations:** Avoid unnecessary cloning, string formatting, and buffer reallocations in hot execution loops.
- **Strict Lint Compliance:** Never suppress compiler or Clippy warnings without a compelling, documented justification. All targets must pass `cargo clippy --all-targets --all-features -- -D warnings`.

### Rule 3 — Native GPUI, GPUI Kit and GPUI Component APIs
**Always prioritize official GPUI, GPUI Kit, and GPUI component APIs for implementing, reimplementing, improving, and refactoring application functionality.**

This is one of the most critical rules for Yomu.

**Before Implementing or Modifying UI:**
1. Inspect the installed toolkit versions in `Cargo.lock` (`gpui-kit = 0.8.1`, `gpui-pre = 0.3.10`).
2. Search the existing codebase (`src/ui/`, `src/app/views/`) for established component patterns.
3. Inspect the installed toolkit source when API availability or behavior is uncertain (`cargo check`, local definitions, or `.agents/skills/gpui-kit/`).
4. Identify the most appropriate official component or API for the task.
5. Prefer native components over custom-built equivalents.

**Mandatory Requirements:**
- Prefer official GPUI Kit components (`Button`, `ButtonGroup`, `Input`, `DropdownMenu`, `Dialog`, `Kbd`, `ScrollableElement`, `Spinner`) over manual `div()` recreation.
- Use native APIs for buttons, menus, dropdowns, dialogs, navigation, inputs, selectors, lists, loading indicators, and animations.
- Prefer native toolkit defaults, behavior, state management, and interaction patterns.
- Never recreate functionality already provided by a suitable native component.
- Do not introduce CSS, HTML, JavaScript, webviews, or external web-style UI frameworks.
- Never modify vendored GPUI or GPUI Kit source code in `vendor/`.
- Never invent APIs or assume features exist in unreleased toolkit versions.
- Do not introduce custom workarounds simply because a native component looks or behaves slightly differently; accept minor visual differences when adopting official components provided functionality is preserved.
- **Never build a second UI framework or design system on top of GPUI Kit.**

### Rule 4 — Maximum Practical Performance
Performance is a first-class engineering requirement.

**Mandatory Considerations:**
- CPU and GPU utilization, frame pacing (60 Hz, 144 Hz, 240 Hz), and input responsiveness.
- Decoded image memory footprint and texture lifecycle.
- Disk I/O frequency and filesystem syscall overhead.
- Asynchronous task coordination and thread scheduling.
- Cache efficiency, data locality, and algorithmic scaling.

**Mandatory Practices:**
- **Zero UI-Thread Synchronous I/O:** Never perform synchronous filesystem reads (`std::fs::read_to_string`, `fs::metadata`, `/proc/meminfo`) inside the UI render loop, animation frame, or scroll handler. Cache static metrics once at startup (`std::sync::OnceLock`).
- **Off-UI-Thread Processing:** Decode archives, decompress images, and perform SIMD resizing strictly on background executors (`cx.background_executor()`).
- **Adaptive Memory Budget:** Decoded page textures must respect `pagecache::system_memory_budget()`, defaulting to 1.0 GiB (`BUDGET_BYTES`) and scaling between 512 MiB and 3.0 GiB based on 1/8th of host RAM.
- **Texture Garbage Collection:** Explicitly release unused GPU textures in `Yomu::render` via `window.drop_image`.
- **Hot-Path Allocation Discipline:** Avoid transient heap allocations, string clones (`book.id.clone()`), and temporary collection creation (`HashSet`, `Vec`) inside per-frame render routines.
- **Scalable Lookups:** Avoid $O(N^2)$ linear comparisons over collections during render loops; prefer contiguous slice iterations or indexed lookups.
- **Asynchronous Cancellation Safety:** Background prefetch tasks aborted during rapid scrubbing must be cleanly dropped without polluting `reader.failed`. A cancelled page must remain requestable on subsequent navigation.

### Rule 5 — Consistent, Intentional and Native UI Design
Treat Yomu as one cohesive desktop application, not a collection of independently implemented screens.

**Mandatory Requirements:**
- **Component Consistency:** Equivalent controls must use the same component and API everywhere. Similar toolbar actions use the same button variant and sizing; confirmation dialogs share native dialog APIs.
- **Icons and Glyphs:** Prefer recognizable native icons or glyphs instead of text labels for compact, frequently repeated actions (Import, Search, Settings, Sort, Navigation, Close). Retain text labels when communicating complex or ambiguous actions.
- **Visual Hierarchy & Palette:** Maintain consistent spacing, sizing, radii, and component variants. Use semantic theme tokens (`UiColors` / Gruvbox palette) rather than hardcoded colors.
- **Restrained Motion:** Use official GPUI Kit animation mechanisms (`EffectTransition`, 120–140 ms ease-out curves) for spatial orientation. High-frequency surfaces (library card grids, search results, decoded comic page display) remain intentionally static to preserve visual stability and rendering speed.
- **Permanent Animation Policy:** Animations remain permanently enabled across all platforms; application initialization explicitly enforces `cx.set_reduce_motion(false)`. Legacy `disable_animations` settings load safely without disabling motion.

### Rule 6 — Evaluate Functionality and Real-World Use Cases
Never implement a feature merely because it sounds interesting or is easy to code.

**Evaluation Checklist Before Implementation:**
- What concrete problem does this feature solve for a manga/comic reader?
- What user workflow benefits from it?
- Are the keyboard and mouse interactions intuitive and non-conflicting?
- What happens in edge cases (empty archives, missing files, corrupt configs, zero pages, rapid window resizing)?
- How does it interact with existing features (single/double spread layout, LTR/RTL reading direction, continuous scrolling, Vim mode)?
- Does it introduce redundant controls, visual clutter, or confusing state?
- Can existing functionality solve the problem more simply?

If a requested change appears functionally incorrect or conflicts with established application behavior, explain the trade-offs and propose a better approach before altering code.

### Rule 7 — Maintainability and Simplicity Over Overengineering
Follow the Ponytail engineering philosophy: prefer the simplest correct, maintainable, secure, and performant solution.

**Mandatory Requirements:**
- Prefer the simplest solution that correctly satisfies requirements.
- Avoid speculative traits, redundant wrappers, premature generic frameworks, and layers of indirection.
- Keep responsibilities clear and functions focused.
- Avoid creating modules for trivial functionality.
- Do not refactor unrelated code during focused bug fixes or feature work.
- Preserve the established three-tier architecture unless there is a justified, documented technical reason to change it.
- Optimize for clarity, correctness, performance, and long-term maintenance over fewest lines of code.

---

## 3. Mandatory Workflow for Every Coding Task

Every agent must follow this exact sequence:

1. **Read Instructions:** Read this root `AGENTS.md` and applicable nested instructions (`src/AGENTS.md`, `src/ui/AGENTS.md`, `src/app/AGENTS.md`).
2. **Review Architecture:** Review [docs/architecture.md](docs/architecture.md) and [docs/agent-handoff.md](docs/agent-handoff.md).
3. **Inspect Before Modifying:** Inspect existing source files, data structures, and tests before making changes.
4. **Clarify Requirements:** Understand the user-facing behavior, edge cases, and constraints.
5. **Research Native APIs:** Check installed GPUI Kit capabilities before implementing custom widgets.
6. **Design Minimal Solution:** Choose the simplest design that satisfies requirements without introducing bloat.
7. **Consider Invariants:** Verify impact on performance, memory budget, persistence ordering, and reading modes.
8. **Implement Focused Changes:** Modify only necessary code; do not churn unrelated formatting or stable logic.
9. **Review Code Quality:** Verify `#![forbid(unsafe_code)]`, idiomatic Rust, native component usage, and error context.
10. **Execute Validation Suite:** Run the complete validation matrix inside the devenv environment.
11. **Update Documentation:** Update documentation only when architecture, behavior, or workflows genuinely change.
12. **Provide Factual Report:** Deliver a transparent, concise implementation report detailing modified files, verification results, and exact test counts.

---

## 4. Project-Specific Technical Constraints

- **Language & Edition:** Rust 2024 edition (`rust-version = "1.92"`).
- **Core Frameworks:** GPUI Kit 0.8.1 (`gpui-kit = "=0.8.1"`, `gpui-pre = 0.3.10`).
- **Development Environment:** NixOS / `devenv` development shell (`devenv.nix`, `flake.nix`).
- **Archive Decoders:** Pure Rust for ZIP/CBZ (`zip`) and image decompression (`image`, `zune-jpeg`, `zune-core`, `fast_image_resize`). C++ library linked for RAR/CBR (`unrar`).
- **Persistence Model:** `$XDG_DATA_HOME/yomu/library.json` (format version 3). Lenient Serde parsing with fallback defaults and automatic timestamped `.corrupt` backups.
- **Safety Invariant:** Deleting books, folders, or collections from the UI **never** deletes comic files from user storage.
- **Reading Order Invariant:** Archive entry indices are strictly zero-based and ordered physically. Reading direction (LTR vs RTL) alters only visual motion and navigation mapping, never archive indexing or caching logic.
- **Global Allocator Policy:** Linux GNU targets use `tikv-jemallocator = "0.6.1"` (`cfg(all(target_os = "linux", target_env = "gnu"))`) to eliminate multi-threaded glibc arena memory retention and actively purge unused pages via `madvise(MADV_DONTNEED)`. Windows and macOS deliberately retain their platform-native system allocators. The global allocator is declared strictly once in `src/main.rs`. Never remove, duplicate, or unconditionally enable jemalloc on non-Linux platforms without explicit justification.

---

## 5. Mandatory Validation Matrix

For every meaningful Rust change, execute the complete validation matrix inside the development shell:

```bash
# 1. Format check
devenv shell cargo fmt --all -- --check

# 2. Fast type check across all targets
devenv shell cargo check --all-targets

# 3. Strict Clippy lint check with warnings as errors
devenv shell cargo clippy --all-targets --all-features -- -D warnings

# 4. Full test suite across all targets (390 active tests)
devenv shell cargo test --all-targets
```

**Validation Rules:**
- Fix failures immediately rather than hiding or ignoring them.
- Never weaken existing test assertions just to make builds pass.
- Never silently mark failing tests as `#[ignore]`.
- Report exact command exit codes and test counts in final handoffs.
- Headless simulated GPUI tests (`gpui_kit::TestAppContext`) validate state, layout, and event dispatch, but cannot verify compositor-level visual transparency or Wayland rendering. Explicitly report uninspected Wayland appearance as **UNVERIFIED**.

---

## 6. Agent Behavior and Reporting

Coding agents must maintain complete transparency and technical accuracy:
- Never claim code compiles or passes tests without running the validation commands and verifying code 0 exit status.
- Never claim visual correctness on Wayland based solely on headless test execution.
- Never invent toolkit APIs or assume hypothetical features.
- Never omit requested functionality silently.
- Do not make unrelated changes or execute broad refactorings during focused tasks.
- Do not commit changes unless explicitly instructed by the user.
- Summarize modified files, responsibilities changed, and tests executed in the completion report.
