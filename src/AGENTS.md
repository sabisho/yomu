# Core Domain Layer — Agent Guide (`src/`)

This guide supplements the root [AGENTS.md](../AGENTS.md) with specific instructions for the **Core Domain Layer** (`src/lib.rs` and core domain modules).

---

## 1. Architectural Boundary & Invariants

The core domain layer constitutes Tier 1 of Yomu's three-tier architecture. It must remain **strictly GUI-independent**:

- **Zero GPUI Dependencies:** Modules in `src/` (outside `src/ui/` and `src/app/`) must never import `gpui`, `gpui_kit`, or windowing crates.
- **Forbid Unsafe Code:** `#![forbid(unsafe_code)]` is strictly enforced in `src/lib.rs`.
- **Pure Domain Operations:** Combines pure computational algorithms (geometry calculation, eviction planning, Vim token parsing, substring search, selection state) with filesystem operations (archive reading, image decoding, JSON persistence).
- **Independent Testability:** All domain logic must be 100% testable via headless unit tests (`cargo test --lib`) without a window, display server, or GPU context.

---

## 2. Module Responsibilities & Constraints

| Module | Core Responsibility | Mandatory Invariants |
|---|---|---|
| `archive.rs` | Enumerate and read CBZ, CBR, ZIP, RAR pages and directories | Natural-sorts entries; filters hidden/system files; enforces bounded allocation caps (`INITIAL_PAGE_BUF_CAPACITY = 8 MiB`, `MAX_PAGE_BYTES = 256 MiB`) against archive bombs. Never mutates source archives on disk. |
| `imaging.rs` | Decode images (JPEG, PNG, WebP, GIF), SIMD scaling, JPEG cover generation | Strictly decoupled from GPUI. Produces raw BGRA pixel byte vectors. Large image decoding and scaling run off the UI thread via `zune-jpeg` and `fast_image_resize`. |
| `layout.rs` | Fit modes (Page, Width, Height, Original), zoom calculation, viewport geometry, decode tiers, dual-page spreads, and mixed-source `SpreadPlan` | Pure math. Page indices are strictly zero-based and ordered in physical archive sequence regardless of RTL display travel. Spread plans precalculate single-image vs two-image spreads with zero per-frame allocation. |
| `library.rs` | Book models, reading progress, marks, user settings, JSON persistence, and import pipeline | Format version 3. Backwards-compatible and lenient with missing or legacy fields. Deleting books, folders, or collections never removes comic files from user storage. Backs up `.corrupt` on parse failure. |
| `collections.rs` | Smart categories (Favorites, Reading, Want to Read, Completed) and custom user collections | Grouping metadata only; decoupled from physical comic storage. |
| `motion.rs` | Frame-time scroll easing, page-turn transitions, reduced-motion detection | Driven by delta time ($dt$), not frame counts. Pure analytical decay; zero UI dependencies. |
| `pagecache.rs` | Prefetch order and texture eviction planning | Pure policy. Preserves current page texture even if it exceeds budget. Bounded by adaptive decoded image budget (`system_memory_budget()`, defaulting to 1.0 GiB and scaling between 512 MiB and 3.0 GiB). Zero allocations during steady-state cache checks. |
| `vim.rs` | Parse key tokens, counts, and motions into typed `Action`s | Pure parser without UI dependencies. The UI or application layer applies the resulting `Action` enum. |
| `selection.rs` | Multi-selection state, toggle, and range selection anchors | Pure state machine. Emits selected IDs in stable library order for deterministic batch operations. |
| `search.rs` | Case-insensitive title, source path, and series substring matching | Pure helper. Query state is transient in the UI and never alters persisted library order. |
| `menu.rs` | Context menu item generation, validation, and confirmation requirements | Decides available book and library actions; UI renders and dispatches them. |
| `util.rs` | Natural sorting, FNV-1a path hashing, date/size formatting | Reusable pure helpers. |

---

## 3. Core Implementation Guidelines

1. **User Data Safety:** Any persistence logic mutating `library.json` must preserve existing unknown fields and write atomically to prevent corruption. Deletion operations must only remove records from `Library`, never invoke `std::fs::remove_file` on comic source paths.
2. **Allocation Discipline:** Avoid allocating intermediate collections in hot path queries. Use iterators, borrowed slices, and scalar ranges (`RangeInclusive<usize>`) rather than transient `HashSet`s.
3. **Archive Memory Bounds:** When reading archive entries, never allocate an unbounded buffer based on unverified archive header size declarations.
4. **Error Context:** Return meaningful `anyhow::Result` context with file paths and operation details for filesystem, archive, and image decode failures.
