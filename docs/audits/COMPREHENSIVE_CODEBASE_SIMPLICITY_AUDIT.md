# Yomu Comprehensive Codebase Simplicity, Native GPUI Adoption & Architecture Audit

**Audit date:** 2026-11-05  
**Scope:** Read-only source and repository audit. No production, dependency, configuration, vendor, or existing documentation files were changed.  
**Evidence boundary:** Local `gpui-kit` is exactly 0.9.0 (`Cargo.toml:21`); `vendor/gpui-component` is the 0.9.0 source patched into the build. No network or external upstream checkout was used. APIs described as verified below are verified against this installed source; claims about whether upstream 0.9.0 lacks Yomu patches use the checked-in historical compatibility audit and are marked that way.

## 1. Executive summary

Yomu has a deliberate three-tier split: pure/archive/persistence algorithms in the library crate, GPUI Kit presentation in `src/ui`, and GPUI orchestration in `src/app`/`src/main.rs`. Major correctness-sensitive flows—archive bounds checks, background image decode, cancellation, texture eviction, atomic persistence, and reading-order/spread logic—have dedicated owners and tests. The codebase does not show a general pattern of needless abstraction; most complexity corresponds to reader behavior, cross-platform input, or data safety.

The largest actionable simplicity opportunity is maintenance ownership of the local GPUI Kit patch: four small compatibility extensions are integrated into a large vendored tree and currently verified by a historical report, but there is no visible patch-isolation/build script. First verify exact diffs against upstream 0.9.0 and keep only required edits. Other concrete opportunities are to verify whether the custom six-tab settings layout can compose with GPUI Kit's 0.9.0 `SettingPage` APIs without losing Yomu's category navigation, and to consolidate repeated cover-cache eviction/removal logic. Neither warrants immediate replacement without behavior tests.

A material finding is documentation drift: `docs/architecture.md` still says GPUI Kit 0.8.1 in several places while the manifest and vendored source are 0.9.0. This is a low-risk documentation correction, deliberately not performed under the audit-only scope. There is no evidence for allocator changes, removing image/cache/concurrency machinery, or replacing the reader's custom canvas with generic components.

## 2. Overall architectural assessment

The separation between `src/` core algorithms, `src/ui/` reusable GPUI views, and `src/app/` state/orchestration is sensible and reflected in real code. Domain modules such as `layout`, `motion`, `pagecache`, `selection`, `search`, and `vim` are pure or mostly pure. Presentation functions accept props and return GPUI elements; orchestration owns async tasks and app state. This provides testable seams without an unnecessary generic service layer.

The code is feature-rich and some orchestration modules are large, especially `reader_controller.rs`, `state.rs`, and `main.rs`. Size alone is not a defect: page loading spans archive metadata, task identity, cancellation, memory budgets, and GPU resources. Prefer future extraction only where a distinct ownership boundary is demonstrated. Existing docs overstate some architectural descriptions and should be refreshed alongside future code changes.

### Repository and module map

- **Core:** `src/archive.rs` handles archive/folder enumeration and bounded reads; `src/imaging.rs` handles decode/resize/cover generation; `src/library.rs` owns library models, import metadata, and persistence support; `src/progress.rs` owns sidecar progress; `src/layout.rs`, `src/motion.rs`, `src/pagecache.rs`, `src/search.rs`, `src/selection.rs`, `src/vim.rs`, `src/menu.rs`, `src/util.rs` contain algorithms and policy.
- **Presentation:** `src/ui/reader.rs` provides reader canvas and reader state calculations; `library_view.rs`, `continue_reading.rs`, `details.rs`, `settings_view.rs`, `sidebar.rs`, `toolbar.rs`, `overlays.rs`, `components.rs` compose GPUI Kit controls and custom product layouts; `theme.rs` maps themes; `cover_cache.rs` manages cover image lifetime; `scroll.rs` adapts scroll momentum.
- **Orchestration:** `src/app/state.rs` owns `Yomu`; `reader_controller.rs` owns decode/prefetch; `persistence.rs` owns saves and shutdown coordination; `actions.rs` routes menus and destructive confirmations; `keybindings.rs` routes keyboard/Vim actions; `views/` adapts state into UI props; `main.rs` owns startup, render, and window events.
- **Platform/build:** `build.rs`, `devenv.nix`, `flake.nix`, `nix/package.nix`, `packaging/`, `resources/`, `vendor/unrar`, and `vendor/gpui-component` support packaging and native integration.

## 3. Native GPUI / GPUI Kit adoption findings

The application already uses native Kit `Button`, `Input`, `Switch`, `DropdownMenu`/menu, `ScrollableElement`, `Scrollbar`, `VirtualList`, themes, tooltips, accessibility labels, and GPUI image/window APIs. The custom `div()` usage is largely for product-specific card composition, reader canvas, toolbar geometry, and surfaces; a same-named component is not automatically an equivalent.

### Findings

#### A-01 — P1 / INVESTIGATE — Vendor patch isolation and upstream comparison

- **Evidence:** `Cargo.toml:37-39` patches `gpui-component` to `vendor/gpui-component`; `vendor/gpui-component/Cargo.toml:14-15` identifies it as 0.9.0. Local modifications are visible in `vendor/gpui-component/src/scroll/scrollable.rs:102-118, 200-235`, `menu/popup_menu.rs:1547`, and `lib.rs:99`.
- **Current behavior:** `Scrollable::track_scroll` accepts caller-owned handles; `scroll_wheel` configures wheel forwarding. Popup menu debug selectors and `Selectable` root re-export are added locally. Actual callers include `src/app/views/library.rs:701,725`, `src/ui/continue_reading.rs:121-123`, `src/ui/details.rs:88-90`, and `src/ui/settings_view.rs:129-131`.
- **Why review:** A whole crate path patch increases update/reconciliation work even if only a few lines are Yomu-specific. Existing `docs/audits/GPUI_KIT_09_COMPATIBILITY_AND_ADOPTION_AUDIT.md` says upstream 0.9.0 lacked these APIs, but this audit did not independently diff upstream sources.
- **Alternative:** Maintain a minimal, reproducible patch against `gpui-component` 0.9.0, or remove an individual hunk only after proving an equivalent public API exists. `ScrollableElement`, `Scrollable`, `Scrollbar`, and `VirtualList` are the installed public APIs; the two scroll builder methods are present locally.
- **0.9.0 verification:** Installed local API verified. Upstream absence verified only by the historical compatibility report; recheck from a clean upstream 0.9.0 source before changing.
- **Benefits:** Smaller, reviewable upgrade surface; avoids carrying unrelated local code changes.
- **Trade-offs:** Removing required behavior breaks synchronized scrollbars, inertia event delivery, or tests.
- **Complexity:** Medium.
- **Required verification:** Exact diff against upstream tag; run library/reader scrolling and menu integration tests on Linux/Windows/macOS and the headless menu tests.

#### A-02 — P2 / INVESTIGATE — Settings category UI versus native Settings APIs

- **Evidence:** `src/ui/settings_view.rs:44-235` builds a custom split-pane six-category view; tab contents span `269-1014`. The installed 0.9.0 source contains `component::setting::{SettingPage, SettingGroup, SettingItem}` (`vendor/gpui-component/src/setting/`).
- **Current behavior:** Yomu groups settings into General, Appearance, Reader, Library, Shortcuts, Advanced/About and wires controls into app-owned state. The local component supports generic setting pages/groups and fields.
- **Why review:** Existing docs mention a generic setting empty state and unused `SettingGroup`; there may be duplicated row/group styling. A generic settings component is not automatically a replacement for Yomu's left category rail, persistence callbacks, or specialized rows.
- **Alternative:** Prototype a single representative category using `SettingPage`/`SettingGroup` and native fields, keeping the category shell if necessary. Adopt only if API supports required grouping, labels, descriptions, custom rows, and keyboard focus.
- **0.9.0 verification:** Component names/source verified locally. Exact fit and keyboard/accessibility parity not verified.
- **Benefits:** More consistent field layout and less repeated row construction if compatible.
- **Trade-offs:** Potential loss of compact density, product-specific grouping, or category navigation behavior.
- **Complexity:** Medium.
- **Required verification:** UI tests for each setting, tab focus/navigation, persistence roundtrip, keyboard traversal, and visual comparison on supported platforms.

#### A-03 — P2 / SIMPLIFY — Repeated cover-cache eviction mechanics

- **Evidence:** `src/ui/cover_cache.rs:65-80` evicts on load; `85-96` duplicates eviction during `insert_item`; `118-128` separately removes one entry; `130-145` clears with/without a window. Capacity is 256 entries (`15-16`).
- **Current behavior:** Custom bounded LRU wraps GPUI `ImageCacheItem`, calls `drop_image` for GPU texture lifecycle, and listens for entity release.
- **Why review:** The cache is justified, but the core eviction sequence is repeated and can drift (window-aware and no-window cases differ appropriately in texture release).
- **Alternative:** Extract one private eviction helper accepting the appropriate image-drop context, or remove dead insertion/accessor methods only after confirming tests and call sites. No generic cache crate is indicated.
- **0.9.0 verification:** GPUI Kit `ImageCache` and `ImageCacheItem` integration verified in local source usage; no evidence that built-in cache provides this capacity/LRU/GPU-drop policy.
- **Benefits:** One eviction invariant, less maintenance risk.
- **Trade-offs:** Helper must preserve correct `Window` context and release behavior.
- **Complexity:** Low.
- **Required verification:** Cache capacity, LRU order, replacement, clear, release, texture drop tests.

#### A-04 — P3 / KEEP — Product-specific reader canvas and card/layout composition

- **Evidence:** `src/ui/reader.rs:158-209, 211-549, 564-864`; custom card/list rendering in `src/ui/library_view.rs:319-973`.
- **Current behavior:** Uses native buttons/scroll primitives around custom manga page presentation, spread turners, overlays, card metadata, selection, and responsive layout.
- **Assessment:** Generic `Button`, `Scrollable`, `VirtualList`, and `Image` primitives do not supply comic spread composition, per-page state, bespoke edge controls, or library card semantics. Replacing with a generic panel/list would risk required behavior and visual identity.
- **Alternative:** Continue composing native primitives; target duplicated style helpers only where exact repeated behavior is confirmed.
- **0.9.0 verification:** Native primitives are present and used; no equivalent complete reader/card component exists in installed component source.
- **Benefits:** None from wholesale replacement; targeted reuse can improve consistency.
- **Trade-offs:** Keep bespoke view code and its tests.
- **Complexity:** High for replacement; low for narrow reuse.
- **Required verification:** Existing reader spread/RTL/continuous-mode and selection/layout tests.

#### A-05 — P3 / KEEP — GPUI native controls are already the prevailing pattern

- **Evidence:** Buttons in `src/ui/components.rs:142-180`, toolbar `src/ui/toolbar.rs:191-338`, settings switches `src/ui/settings_view.rs:289-667`, search `src/ui/toolbar.rs:148`, virtual library and scrollbar `src/app/views/library.rs:701-727`.
- **Assessment:** No broad custom button/input framework was found. Custom styling is layered on native controls, and accessibility labels/tooltips are present for compact actions per prior 0.9 migration report.
- **Recommendation:** Reuse helpers in `components.rs` for equivalent controls; do not migrate already-native controls solely to match a component name.
- **0.9.0 verification:** Yes for current APIs, using local dependency source.

## 4. Overengineering and Rust idiomaticity

#### A-06 — P2 / KEEP — Theme reload metadata and content fingerprint checks protect atomic updates

- **Evidence:** `src/ui/theme.rs:253-282` defines file metadata/fingerprint; `custom_theme_fingerprint` and `custom_theme_metadata` support reload detection; `src/app/state.rs` owns the theme watcher/reload coordination (search `theme` within that file; entry point near line 172).
- **Current behavior:** Supports user TOML themes and change detection; `theme.rs:161-232` parses user themes and `335-399` resolves names.
- **Assessment:** It checks metadata every 1.5 seconds, waits for atomic replacement to stabilize, then hashes changed content so same-length writes and timestamp collisions are detected. `DefaultHasher` is used, not SHA-256 as the stale architecture document claims. This avoids rereading content while metadata is unchanged.
- **Recommendation:** Keep the current strategy unless a reliable native watcher preserves atomic-replacement and same-size-edit detection on all supported platforms.
- **0.9.0 verification:** GPUI Kit theme config/registry APIs verified in vendor source; no equivalent file watcher API established here.
- **Benefits:** Avoids missed updates and unnecessary background reads.
- **Trade-offs:** Periodic metadata wakeups remain; filesystem watchers have platform and atomic-rename edge cases.
- **Complexity:** Medium to replace, with no change recommended on current evidence.
- **Required verification:** Preserve large-file, rapid-replacement, same-length-write, corrupt TOML recovery, and theme-switching tests if this mechanism changes.

#### A-07 — P2 / KEEP — Persistence synchronization protects ordering and shutdown

- **Evidence:** `src/app/persistence.rs` contains debounced saves, generation tracking, background dispatch, locks, and quit/flush coordination (core implementation around lines 150-355; tests below 359). `src/app/state.rs` stores shared sequence/lock state.
- **Current behavior:** Prevents stale writes from overtaking newer library/progress state, keeps disk operations off the UI thread, and coordinates shutdown.
- **Assessment:** `Arc`, atomics, mutexes, and task coordination look elaborate but solve observable ordering and shutdown requirements. The tests explicitly exercise concurrency, failed writes, and cancellation. Replacing with a simple background write risks data loss or stale persistence.
- **Recommendation:** Keep. Future simplification should start with a written state-transition model and preserve sequence invariants.
- **0.9.0 verification:** GPUI executor APIs used; persistence design is application-owned.
- **Required verification:** Existing race, failure, rapid-save, and shutdown tests.

#### A-08 — P2 / KEEP — Reader task identity/cancellation and decoded cache are justified

- **Evidence:** `src/app/reader_controller.rs:115-167` starts page work; `1192-1316` decodes, validates cancellation identity, and commits results; `941-1138` plans cache/prefetch. Pure policy is in `src/pagecache.rs:34-110`; image work in `src/imaging.rs:197-323`.
- **Assessment:** Background execution, `Arc` image sharing, cancellation tokens, cache eviction, and task identity protect responsiveness, memory, and rapid navigation. No standard-library simplification removes these semantics.
- **Recommendation:** Keep. Do not merge pure cache policy into GPUI controller or move decoding onto UI thread.
- **Required verification:** Existing cancellation, re-request, memory-budget, page-order, and decode tests.

#### A-09 — P2 / KEEP — Archive and persistence defensive handling is trust-boundary work

- **Evidence:** `src/archive.rs:92-112` validates archive entry names; archive enumeration/read paths at `186-233, 493-508, 636-719`; JSON/atomic persistence in `src/library.rs` and sidecar `src/progress.rs:168-245`.
- **Assessment:** Bounds, traversal defenses, corrupt-data backup, atomic replacement, and source-file preservation are required for untrusted archives and user-owned data. They are not redundant validation absent proof at each boundary.
- **Recommendation:** Preserve current safeguards. Audit error messages and duplicated validation only alongside a threat model and tests.
- **Required verification:** Malformed archive, oversized entries, traversal, corrupt JSON, interrupted writes, and no-source-deletion tests.

#### A-10 — P3 / INVESTIGATE — Cover cache linear recency bookkeeping

- **Evidence:** `src/ui/cover_cache.rs:57-61, 118-122` linearly locates a key in `VecDeque` for touch/remove; max 256 items.
- **Assessment:** This is O(capacity) but bounded at 256 and likely insignificant relative to image load/texture work. A linked LRU/index map would add bookkeeping and complexity.
- **Alternative:** Keep unless profiling or a materially larger capacity shows this path is hot; if needed use a small indexed LRU design.
- **Benefit/trade-off:** Avoid premature complexity; possible lookup speed only under measured cache-hit pressure.
- **Complexity:** Medium to change; no change recommended on current evidence.
- **Required verification:** Profile repeated cache hits before implementation.

## 5. Architecture and maintainability

#### A-11 — P2 / IMPROVE — Architecture documentation version drift

- **Evidence:** `Cargo.toml:21` pins `gpui-kit = 0.9.0`; `vendor/gpui-component/Cargo.toml:15` is 0.9.0; `docs/architecture.md` describes UI as 0.8.1 and motion APIs as 0.8.1. `docs/audits/GPUI_KIT_09_MIGRATION_REPORT.md` records 0.9.0 migration.
- **Issue:** New contributors can make wrong API assumptions and audit references contradict the build.
- **Alternative:** Update version references and source links in architecture/development docs in a separate documentation task; retain historical audit reports as historical records.
- **Verification:** Search all maintained docs for 0.8.1 after update; make sure historical reports are labeled historical.
- **Complexity:** Low.

#### A-12 — P3 / KEEP — Domain modules and view adapters have useful boundaries

- **Evidence:** `src/lib.rs` exposes core modules; `src/app/views/` adapts state; `src/ui/` owns reusable rendering. `src/layout.rs`, `motion.rs`, `pagecache.rs`, `vim.rs`, `selection.rs`, and `search.rs` expose pure logic.
- **Assessment:** This is not excessive fragmentation by itself; it keeps algorithms testable and prevents GPUI from entering domain logic.
- **Recommendation:** Preserve. Consolidate only genuinely duplicated rules, not simply small modules.

## 6. Performance and resource usage

No new benchmarks or runtime measurements were performed; performance conclusions below are source-based, not measured findings.

#### A-13 — P1 / KEEP — Decode off UI thread and memory-bounded page texture lifecycle

- **Evidence:** Decode path in `src/app/reader_controller.rs:1192-1233`; adaptive budget and eviction policy in `src/pagecache.rs:34-110`; render releases textures in `src/main.rs:1178+` (inspect `garbage`/`drop_image` calls around render); resize/decoding in `src/imaging.rs:197-323`.
- **Assessment:** Required for large comics and texture constraints; no native GPUI component replaces archive decode or page-level memory policy.
- **Recommendation:** Keep, and retain explicit GPU image release.
- **Verification:** Existing tests and platform runtime profiling when changing; compositor appearance remains outside this audit.

#### A-14 — P2 / INVESTIGATE — Library row cache validity and render-time work

- **Evidence:** `src/ui/library_view.rs:97-149` defines row cache; grouped/sort helpers at `167-305`; library rendering uses Kit `VirtualList` at `src/app/views/library.rs`.
- **Question:** Verify invalidation correctness and whether rows are recomputed or allocated during render. The presence of a cache can either reduce work or complicate invalidation.
- **Alternative:** Keep `VirtualList`; measure before removing custom row cache or changing grouping. Use stable indexed data and precomputed projections only if allocations/render work are shown hot.
- **Evidence level:** Potential concern, not measured inefficiency.
- **Required verification:** Large-library benchmark/profile and mutation tests for rename, progress, sorting, selection, import, and deletion.

#### A-15 — P2 / KEEP — Cover cache controls GPU retention

- **Evidence:** `src/ui/cover_cache.rs:15-45, 47-80, 130-155`; entity release drops images.
- **Assessment:** GPUI's image cache trait is reused, but Yomu's bounded policy and explicit `drop_image` are domain-specific resource controls. Do not remove without a replacement memory bound.

## 7. Vendor patches and custom extensions

The only explicit Cargo path patches are `gpui-component` and `unrar` (`Cargo.toml:37-39`). `vendor/unrar` exists to support CBR/RAR and should be treated as a separate native dependency/packaging boundary. No GPUI core fork is configured.

`PlatformScrollController` in `src/ui/scroll.rs:1-223` is a custom cross-platform adapter: macOS passes through native AppKit momentum; Linux/Windows estimate trackpad velocity and animate a GPUI `ScrollHandle` using pure `motion::KineticScroll`. The implementation explicitly distinguishes pixel deltas from line wheel events and notes platform event-phase differences. `Scrollable::scroll_wheel(false)` forwards event delivery to it; `track_scroll` binds caller-owned handle. It is custom behavior that generic scroll API similarity does not replace. Retain pending tests on real Linux/Windows hardware and a verified upstream API offering equivalent phase/momentum/handle semantics.

The historical GPUI 0.9 compatibility report states four local additions: popup debug selector, root `Selectable` re-export, `track_scroll`, and `scroll_wheel`. Current local source confirms those additions exist. This pass did not independently inspect official upstream 0.9.0 tag or event propagation in upstream; A-01 records that verification gap. The local `PopupMenu::render` debug selector (`popup_menu.rs:1547`) supports test selectors, not runtime menu behavior.

## 8. Dependency assessment

Direct dependencies in `Cargo.toml` have visible responsibilities: GPUI Kit UI/runtime; serde/JSON/TOML configuration and persistence; dirs for platform paths; `image`/`zune-jpeg`/`zune-core`/`fast_image_resize` for supported decode and SIMD resize; `zip`/`unrar` for archive formats; `anyhow` contextual application errors; `embed-resource` for Windows resources. Feature flags narrow image and ZIP capabilities. There is no obvious direct dependency that can be removed from manifest inspection alone.

The lockfile contains a large transitive tree because GPUI Kit and media/platform stacks are broad; that is not evidence of unnecessary direct dependencies. `tikv-jemallocator` and `tikv-jemalloc-ctl` are target-specific Linux GNU dependencies and are expressly outside any recommendation to alter here. Preserve NixOS accommodations in `devenv.nix`, `flake.nix`, and `nix/package.nix`; this pass found no concrete basis for removing them. Do dependency usage analysis with `cargo tree -i` and source symbol tracing before any later cleanup.

## 9. Material 3 Expressive readiness

- **Directly supported:** GPUI Kit 0.9.0 provides theme configuration/tokens, reusable buttons, switches, menus, inputs, dialogs, scrollbars, tabs, settings, tooltips, and motion primitives in the installed vendor source. `src/ui/theme.rs:22-75` maps semantic colors; builtin theme TOML is checked in.
- **Achievable through reusable Rust components:** semantic color roles, button variants/states, category navigation, spacing/radius scales, surface compositions, and typography hierarchy can be consolidated in `UiColors` and small helpers in `components.rs`. Current `CornerRounding` setting is applied from app state (`src/app/state.rs:358+`).
- **Scattered/uncertain:** Per-view custom `div()` compositions and hardcoded sizing/spacing need a measured inventory before asserting inconsistency. Search of selected high-use surfaces shows many values are purpose-specific (reader overlay and card dimensions); no exhaustive token-use linter was run.
- **Lower-level custom rendering:** page layers, fit/zoom/spread placement, turn chevrons, and cover cards require application-owned composition; low-level GPUI painting should remain limited to cases where standard elements cannot represent behavior.
- **Platform-specific:** titlebar/traffic lights/native decorations and OS momentum are platform integrations and cannot be made uniformly expressive purely through a token file.
- **Readiness:** Good foundation, incomplete centralization. Do not add a Material dependency or rewrite visual identity. A future design-system pass should first inventory literal colors and repeated metrics, then consolidate only stable semantic roles.

## 10. Prioritized findings

| ID | Priority | Classification | Finding |
|---|---|---|---|
| A-01 | P1 | INVESTIGATE | Reconcile vendor patch against official upstream 0.9.0 and isolate necessary hunks |
| A-02 | P2 | INVESTIGATE | Evaluate native settings-page/group composition for actual Yomu settings behavior |
| A-03 | P2 | SIMPLIFY | Centralize repeated cover-cache eviction mechanics |
| A-04 | P3 | KEEP | Preserve custom reader canvas and card composition |
| A-05 | P3 | KEEP | Existing UI already uses GPUI Kit controls extensively |
| A-06 | P2 | KEEP | Preserve metadata/content checks for robust custom-theme reload |
| A-07 | P2 | KEEP | Preserve persistence ordering, atomicity, and shutdown coordination |
| A-08 | P2 | KEEP | Preserve reader cancellation, prefetch, and cache invariants |
| A-09 | P2 | KEEP | Preserve archive/path and persistence trust-boundary validation |
| A-10 | P3 | INVESTIGATE | Profile bounded linear LRU bookkeeping before changing it |
| A-11 | P2 | IMPROVE | Correct maintained docs that still specify GPUI Kit 0.8.1 |
| A-12 | P3 | KEEP | Preserve pure domain modules and view adapters |
| A-13 | P1 | KEEP | Preserve background decode and explicit GPU texture release |
| A-14 | P2 | INVESTIGATE | Measure library row cache invalidation/allocation behavior |
| A-15 | P2 | KEEP | Preserve bounded cover cache and image release policy |

### Counts

- **Priority:** P0 0; P1 2; P2 9; P3 4.
- **Classification:** REPLACE 0; SIMPLIFY 1; REMOVE 0; CONSOLIDATE 0; IMPROVE 1; KEEP 9; INVESTIGATE 4; WAIT 0.

## 11. Things that should explicitly remain unchanged

- No-unsafe Rust policy and crate-root `forbid(unsafe_code)` declarations.
- Linux GNU allocator selection and platform-specific allocator boundaries.
- User archive safety, bounds checks, corrupt JSON backup, atomic persistence, and “remove from library never deletes source comic” behavior.
- Off-UI-thread archive decoding/resizing and page cancellation/re-request semantics.
- Decoded page memory budget, page cache, virtualized library, and explicit GPU texture release.
- Reader archive index ordering, spread plan, RTL mapping, continuous mode, and per-page canvas.
- Existing native GPUI Kit buttons/inputs/switches/menus and product-specific view composition absent a proven behavior-equivalent replacement.
- NixOS runtime/build accommodations and offline-first operation.

## 12. Areas requiring further investigation

1. Compare exact local vendor diff against the official `gpui-component` 0.9.0 source. Current evidence confirms local methods exist, but upstream absence is sourced from an existing report.
2. Test real touchpad event propagation and momentum on Linux Wayland/X11 and Windows; headless tests cannot prove device event phases.
3. Prototype Kit settings group/page composition and compare focus, keyboard accessibility, and visual density before recommending adoption.
4. Profile theme-file fingerprint reads and library row-cache behavior; no measurements were made in this audit.
5. Complete a literal token/spacing scan across all UI modules before making claims that theme centralization is missing or recommending broad visual changes.
6. Verify third-party direct dependency use with cargo-tree/source analysis if dependency minimization is later requested.

## 13. Overall recommendations

Start with A-01's upstream patch reconciliation because it bounds the ongoing maintenance surface and informs any further component adoption. Correct A-11 in a separate documentation task. Treat A-02/A-03 as targeted follow-ups with behavior-focused proof; preserve A-06 unless an equivalent cross-platform watcher is demonstrated. Keep all reader, archive, persistence, cache, and allocator protections unless measurements and regression tests demonstrate a safe alternative. No production changes or tests were performed as part of this audit.
