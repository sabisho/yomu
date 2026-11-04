# Yomu Codebase Simplification Roadmap

This roadmap maps to findings in [COMPREHENSIVE_CODEBASE_SIMPLICITY_AUDIT.md](COMPREHENSIVE_CODEBASE_SIMPLICITY_AUDIT.md). It is advisory; no implementation was performed.

## Stage 1: Critical correctness, safety, and performance issues

**Findings:** A-07, A-08, A-09, A-13, A-15 (preserve and establish regression gates, not cleanup tasks). No active P0 correctness defect was evidenced in this source audit.

- **Files/modules:** `src/app/persistence.rs`, `src/app/reader_controller.rs`, `src/archive.rs`, `src/library.rs`, `src/progress.rs`, `src/imaging.rs`, `src/pagecache.rs`, `src/ui/cover_cache.rs`, `src/main.rs`.
- **Dependencies:** None. Establish current behavior before any later change.
- **Outcome:** Protect persistence ordering, archive trust boundaries, cancellation/re-request, memory bounds, and texture release as explicit invariants.
- **Regression tests:** Existing race/failure/shutdown tests; corrupt and traversal archive tests; cancellation and page-cache tests; texture lifecycle tests. Add no tests unless an implementation changes these behaviors.
- **Risks:** “Simplification” here can cause user data loss, UI stalls, failed navigation, or GPU memory growth.
- **Completion criterion:** Each later change touching these paths names the invariant it preserves and passes its focused regression proof; no change is required in this stage based on current evidence.

## Stage 2: Native GPUI/GPUI Kit component adoption

**Findings:** A-01, A-02, A-05.

- **Files/modules:** `Cargo.toml`, `vendor/gpui-component/` (only if later authorized), `src/ui/settings_view.rs`, scroll call sites in `src/app/views/library.rs` and `src/ui/{continue_reading,details,settings_view}.rs`.
- **Dependencies:** First compare pristine upstream `gpui-component` 0.9.0 against the vendor source; inspect actual public APIs and input propagation. Then prototype one settings page.
- **Outcome:** Know exactly which vendor patches remain required; adopt Kit settings primitives only for behavior they fully cover.
- **Regression tests:** Menu selector integration tests; shared scrollbar position/drag tests; Linux/Windows momentum and line-wheel tests; settings persistence, focus order, keyboard traversal, and search tests.
- **Risks:** A similarly named upstream component may not preserve handle ownership, event bubbling, focus, category navigation, density, or accessibility.
- **Completion criterion:** A reviewed upstream diff identifies every local hunk and justification; any adopted settings component passes behavior and accessibility checks without changing data semantics.

## Stage 3: Remove unnecessary custom implementations

**Findings:** A-03, A-04, A-10, A-15.

- **Files/modules:** `src/ui/cover_cache.rs`, `src/ui/reader.rs`, `src/ui/library_view.rs`.
- **Dependencies:** Do not begin before Stage 1 protections and Stage 2 native API comparison. Keep A-04/A-15 unless concrete redundancy is proven.
- **Outcome:** Consolidate cache eviction paths if appropriate; remove only verified dead helpers or truly duplicated implementations. No wholesale reader/card replacement.
- **Regression tests:** LRU ordering/capacity, image drop on eviction/clear/release, reader spread/RTL/continuous layout, selection interactions.
- **Risks:** Incorrect texture lifecycle or lost reader interaction; more elaborate data structure may be slower at 256 entries.
- **Completion criterion:** One cache invariant owns eviction and release semantics, or profiling documents why existing code remains; no public view behavior changes unexpectedly.

## Stage 4: Simplify architecture and Rust code

**Findings:** A-07, A-08, A-09, A-12, A-14. Preserve A-06 unless an equivalent cross-platform watcher is demonstrated.

- **Files/modules:** `src/ui/theme.rs`, `src/app/state.rs`, `src/ui/library_view.rs`, `src/app/reader_controller.rs`, `src/app/persistence.rs`.
- **Dependencies:** Profiling/lifecycle tracing for row cache; retain the current theme fingerprint and state models for persistence and decode.
- **Outcome:** Apply only evidence-backed local simplifications; keep pure algorithms separate, preserve state ownership, and retain theme reload robustness.
- **Regression tests:** Large-library sorting/grouping/mutation; save ordering and page cancellation. If theme watching changes, preserve same-size edits, rapid atomic replacement, and corrupt-file recovery.
- **Risks:** Row cache can stale; moving orchestration across tiers can introduce borrow/lifecycle complexity.
- **Completion criterion:** Each proposed simplification identifies a removed duplicate responsibility or measurable cost and preserves focused tests; no architecture rewrite based on module size alone.

## Stage 5: Consolidate themes and reusable UI components

**Findings:** A-02, A-05, A-11; readiness assessment in section 9 of the main audit.

- **Files/modules:** `src/ui/theme.rs`, `src/ui/components.rs`, `src/ui/{sidebar,toolbar,settings_view,library_view,details,continue_reading}.rs`, `docs/architecture.md`.
- **Dependencies:** Confirm Kit 0.9 tokens and setting component APIs; inventory repeated literals before changing styles.
- **Outcome:** Align docs with 0.9.0 and centralize only stable semantic colors/metrics/component states. Preserve Yomu branding and compact layout.
- **Regression tests:** Theme parsing/switching, accessibility labels and focus, representative visual snapshots/layout tests, light/dark contrast review.
- **Risks:** Broad tokenization can flatten intentional distinctions or produce visual regressions across platforms.
- **Completion criterion:** Maintained docs state correct versions; repeated semantic values resolve through tokens/helpers; bespoke values remain documented as purposeful.

## Stage 6: Optional refinements and future upstream opportunities

**Findings:** A-01, A-06, A-10, A-14; WAIT opportunity only if upstream adds equivalent momentum/scroll ownership behavior.

- **Files/modules:** Vendor patch, `src/ui/scroll.rs`, `src/ui/cover_cache.rs`, `src/ui/theme.rs`, `src/ui/library_view.rs`.
- **Dependencies:** Upstream release/API or measured profile evidence; cross-platform hardware checks.
- **Outcome:** Revisit patches and potential hot-path work when new evidence is available.
- **Regression tests:** Event propagation and physical-device inertia; cache/profile benchmark; watcher/reload scenarios.
- **Risks:** Upstream API names may conceal different event semantics; benchmarks may not reflect typical libraries.
- **Completion criterion:** Changes are triggered by verified equivalent public API or measured bottleneck, with performance and behavior evidence recorded. Otherwise keep current implementation.
