# Architecture Reliability Remediation Roadmap

Audit date: 2026-11-07. This roadmap addresses confirmed findings first. It is a plan only; no fixes were applied during the read-only audit.

## Stage 1 — Critical correctness and data integrity

**Findings:** YOMU-001, YOMU-002. No P0/P1 confirmed.

- **Files/modules:** `src/app/actions.rs`, `src/app/persistence.rs`, `src/app/state.rs`, `src/library.rs`, `src/progress.rs`, relevant tests in `src/tests.rs` and persistence module tests.
- **Dependencies:** YOMU-001 is independent. YOMU-002 needs a defined startup recovery state and clear fallback policy before implementation.
- **Recommended changes:** Share cleanup logic between single and bulk removal; remove sidecar records for every removed ID and save both stores. Surface progress corruption and backup failures at startup; do not silently replace the damaged sidecar with stale data.
- **Regression tests:** Bulk remove then reload/reimport; progress corruption with older `library.json`; backup rename failure; recovery notice; normal and reset-all persistence.
- **Risks:** A sidecar deletion policy affects whether users can recover removed-book progress on reimport; align it with existing single-remove behavior. Recovery UI must not block startup or discard the backup.
- **Completion criteria:** All mutation paths produce matching library/progress IDs; corrupted progress is observable; backup remains available; no source archives are deleted; focused tests pass.

## Stage 2 — Concurrency and lifecycle reliability

**Findings:** YOMU-003 (memory admission); reader cancellation path reviewed and no stale overwrite defect confirmed.

- **Files/modules:** `src/app/reader_controller.rs`, `src/ui/reader.rs`, `src/imaging.rs`, potentially `src/pagecache.rs`.
- **Dependencies:** Stage 1 is not a prerequisite. Define a resource policy before changing parallelism.
- **Recommended changes:** Account for active decode reservations in the memory policy; bound the number/size of concurrent in-flight decodes and release reservation on success, error, and cancellation. Preserve token identity validation and allow a canceled page to be requested again.
- **Regression tests:** Deterministic concurrent admission and cancellation tests; stale decode completion after book switch; same-ID reopen after source replacement; isolated memory profile for large pages.
- **Risks:** Conservative limits can increase page-turn latency. Avoid reserving from untrusted declared metadata alone; validate actual dimensions and retain a safe fallback.
- **Completion criteria:** Active plus resident image memory follows an explicit bound; all task completion paths release reservations; existing rapid-navigation semantics remain intact.

## Stage 3 — Core workflow correctness

**Findings:** YOMU-001/002 fixes overlap. YOMU-004 is a compatibility policy decision.

- **Files/modules:** Library persistence and import paths, reader progress, settings/theme reload, UI action tests.
- **Dependencies:** Decide whether downgrade/forward-version libraries are supported before addressing YOMU-004.
- **Recommended changes:** Add end-to-end tests for duplicate imports after removal, stale source paths, empty/corrupt archives, mixed image formats, restart after progress, group selection and bulk actions. For YOMU-004 either preserve unknown fields or reject unsupported newer versions before any save.
- **Regression tests:** Unknown-field round trip / newer-version refusal; representative workflows across restart; no-loss fallback for malformed sidecars.
- **Risks:** Forward field preservation expands serialized model ownership; explicit refusal is safer if downgrade is unsupported.
- **Completion criteria:** Each supported user workflow has normal, edge, failure, and restart coverage; persistence compatibility policy is explicit.

## Stage 4 — Cross-platform correctness

**Findings:** No platform-specific bug confirmed; INVESTIGATE-003 and INVESTIGATE-006 remain unverified.

- **Files/modules:** `src/ui/scroll.rs`, `src/main.rs`, `src/library.rs`, `src/progress.rs`, `src/app/keybindings.rs`, `nix/package.nix`, `devenv.nix`, GPUI Kit integration.
- **Dependencies:** Native Linux, Windows, and macOS build/runtime environments.
- **Recommended changes:** No code change until native runs demonstrate a defect. Verify Wayland and X11, Windows rename retries and modifiers, macOS titlebar clearance/native momentum, file dialogs, and RAR linking.
- **Regression tests:** Platform build matrix plus manual input and persistence smoke checklist.
- **Risks:** CI compile coverage cannot prove compositor or native gesture behavior.
- **Completion criteria:** Each supported OS has build evidence and a recorded native smoke run; limitations are documented.

## Stage 5 — Performance and resource efficiency

**Findings:** YOMU-003; archive aggregate entry-count behavior is INVESTIGATE-001, not currently a confirmed defect.

- **Files/modules:** `src/archive.rs`, `src/imaging.rs`, `src/app/reader_controller.rs`, `src/pagecache.rs`.
- **Dependencies:** Stage 2 resource admission; representative controlled fixtures.
- **Recommended changes:** Measure before tuning. Bound archive metadata/page-name accumulation if generated stress fixtures demonstrate unsafe growth. Preserve existing caches and prefetch until measurements justify changes.
- **Regression tests:** Maximum image fixture, many-entry archive fixture, peak in-flight memory measurement, cache accounting test.
- **Risks:** New caps can reject valid unusual comic collections; limits should be explicit and diagnostically reported.
- **Completion criteria:** Measurements demonstrate bounded peak resource use and ordinary scan responsiveness remains acceptable.

## Stage 6 — Architecture and maintainability

**Findings:** YOMU-004 and removal-path duplication; no broad architectural failure found.

- **Files/modules:** `src/app/actions.rs`, `src/library.rs`, `src/progress.rs`, schema model boundaries.
- **Dependencies:** Compatibility policy; Stage 1 behavior tests.
- **Recommended changes:** Consolidate single/bulk removal cleanup at the responsible application/domain boundary. Keep UI presentation separate from policy. Address unknown-field retention only if supported by explicit schema requirements.
- **Regression tests:** Shared deletion semantics, schema policy tests, mutation/save ordering checks.
- **Risks:** Avoid a generic persistence framework for only these cases; preserve the current three-tier design and avoid unrelated refactoring.
- **Completion criteria:** One source of truth for removal cleanup; schema compatibility behavior is tested and documented.

## Stage 7 — Testing and long-term reliability

**Findings:** All confirmed IDs; INVESTIGATE-001–006.

- **Files/modules:** Existing `src/archive.rs`, `src/library.rs`, `src/progress.rs`, `src/app/persistence.rs`, `src/tests.rs`, platform qualification notes.
- **Dependencies:** Implementations from prior stages.
- **Recommended changes:** Add targeted regression tests listed above, deterministic task-order tests, and isolated adversarial archive fixtures. Keep compositor/device checks as manual platform qualification where headless GPUI tests cannot provide evidence.
- **Risks:** Timing-based concurrency tests can be flaky; use barriers/fakes/admission state rather than sleeps where practical.
- **Completion criteria:** Every confirmed finding has a regression test; each unsupported platform behavior is labeled; test commands and exact results are recorded by the implementation work.

