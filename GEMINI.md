# Yomu — Gemini & Antigravity Agent Instructions

This document is the primary entry point for **Google Antigravity**, **Gemini**, and automated coding assistants working on Yomu.

The authoritative, non-negotiable engineering rules for this repository are defined in:

👉 **[AGENTS.md](AGENTS.md)**

All coding agents must read and strictly adhere to `AGENTS.md` before inspecting or modifying code.

---

## 1. Fast Rules Summary

1. **Absolutely Zero Unsafe Rust:** Preserve `#![forbid(unsafe_code)]` everywhere. No exceptions.
2. **Idiomatic Rust Everywhere:** Rust 2024 edition, explicit error handling, borrowing discipline, standard library first, zero Clippy warnings.
3. **Official GPUI Kit APIs First:** Always prioritize native `gpui-kit` 0.8.1 components over manual `div()` constructions. Never build a second UI framework.
4. **Maximum Practical Performance:** Zero synchronous filesystem I/O on the UI thread, background image decoding and resizing, adaptive 512 MiB–3.0 GiB memory budget, Linux GNU jemalloc global allocator (`cfg(all(target_os = "linux", target_env = "gnu"))`), no per-frame heap allocations.
5. **Native Desktop UI Consistency:** Reusable Gruvbox semantic theme tokens (`UiColors`), consistent icon glyphs, restrained `EffectTransition` motion, permanent animation policy (`cx.set_reduce_motion(false)`).
6. **Evaluate Real-World Use Cases:** Every change must solve a concrete manga reader problem, support reading modes (LTR/RTL, single/double spread, continuous), and handle edge cases gracefully.
7. **Simplicity Over Overengineering:** Follow the Ponytail philosophy: choose the simplest maintainable solution, avoid premature abstractions, and keep changes focused.

---

## 2. Architectural Structure

- **Tier 1 — Core Domain Layer (`src/`):** Pure GUI-independent Rust algorithms, archive enumeration, SIMD resizing, layout geometry, caching policy. Governed by [src/AGENTS.md](src/AGENTS.md).
- **Tier 2 — UI Presentation Layer (`src/ui/`):** GPUI Kit presentation views, dialogs, overlays, and platform momentum scrolling. Governed by [src/ui/AGENTS.md](src/ui/AGENTS.md).
- **Tier 3 — Application Orchestration (`src/app/`, `src/main.rs`):** Window lifecycle, application state routing, async decode scheduling, and persistence coordination. Governed by [src/app/AGENTS.md](src/app/AGENTS.md).

---

## 3. Reference Documentation

- **[docs/architecture.md](docs/architecture.md):** Complete runtime topology, module responsibilities, control flows, and architectural invariants.
- **[docs/development.md](docs/development.md):** NixOS/devenv development workflow, profiling benchmarks, and standard recipes.
- **[docs/agent-handoff.md](docs/agent-handoff.md):** Active operational snapshot, completed phases, known technical debt, and next recommended tasks.
- **[ROADMAP.md](ROADMAP.md):** Tracked product features and engineering roadmap.

---

## 4. Mandatory Validation Commands

Before handing work back, run the full validation matrix in `devenv shell`:

```bash
devenv shell cargo fmt --all -- --check
devenv shell cargo check --all-targets
devenv shell cargo clippy --all-targets --all-features -- -D warnings
devenv shell cargo test --all-targets
```

All 380 active tests (168 lib unit tests, 212 binary UI tests, plus 16 benchmarks; 396 total) must pass with exit code 0.
