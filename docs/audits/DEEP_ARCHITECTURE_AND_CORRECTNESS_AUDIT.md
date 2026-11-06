# Yomu Deep Architecture and Correctness Audit

Date: 2026-11-07  
Scope: read-only source inspection of the repository at the recorded Git state. Only the four requested audit deliverables are created; no application code, tests, configuration, dependencies, vendor files, or user data were changed.

## Executive assessment

Yomu has a clear three-tier structure and several strong safeguards: library/progress writes use temporary files and rename, archive page extraction is bounded, reader decode callbacks reject stale task identities, source archives are not deleted on library removal, imports have cancellation checkpoints, and GPU images are explicitly released from the window lifecycle. The audited code does not support several broad failure scenarios sometimes suggested by stale handoff notes.

Three actionable findings were confirmed by reachable code paths:

| ID | Severity | Classification | Confidence | Summary |
|---|---|---|---|---|
| YOMU-001 | P2 | DATA_INTEGRITY / BUG | High | Bulk removal leaves progress sidecar records; reimporting the same path can restore old progress. |
| YOMU-002 | P2 | DATA_INTEGRITY / BUG | High | Corrupt `progress.json` recovery error is silently ignored at startup, and the next write can replace recovered progress with stale library fields. |
| YOMU-003 | P2 | SECURITY / RESOURCE_LIFECYCLE | Medium | Concurrent image decodes can exceed the cache budget because the budget only accounts for completed images, while each in-flight decode can allocate large buffers. |

No P0 or P1 defect was demonstrated. One design risk is recorded as YOMU-004: deserializing and reserializing library data drops unknown fields, which matters if a newer-format library is opened and saved by an older build. This is an interoperability concern, not a demonstrated current-version loss.

## Scope and evidence gathered

Read the repository instructions and architecture/development/agent-handoff documents; enumerated Rust modules; inspected package/build/Nix configuration; traced application startup, library/progress persistence, import coordination, archive listing/reading, image decoding, reader task dispatch/completion, user removal/reset paths, theme reload, and the locally patched GPUI Kit scrollable implementation. Relevant tests and prior audit reports were inspected by source search and targeted reads. The complete repository contains about 34,000 Rust lines, including a roughly 9,500-line UI test module; focused path tracing was used rather than claiming an exhaustive line-by-line review of every UI builder.

Important actual boundaries:

- `src/lib.rs` is the GUI-independent library crate. It exports core archive, image, layout, library, progress, cache, search, selection, and Vim logic.
- `src/app/state.rs` owns the root `Yomu` entity. `src/app/reader_controller.rs`, `persistence.rs`, `actions.rs`, and `keybindings.rs` mutate it; `src/app/views/` builds presentation props and callbacks.
- `src/ui/` provides GPUI Kit views. The app's `Render` implementation in `src/main.rs` schedules reader work and drains texture garbage.
- Import and decode work is dispatched through GPUI background executors. RAR session operations are serialized on a worker thread using channels.
- Library metadata is in `library.json`; high-frequency reading progress is in `progress.json`; covers are app-owned cache files. Comic source files remain user-owned.
- Cargo resolves GPUI Kit through the local 0.9.0 vendor patch. `vendor/gpui-component/YOMU_PATCHES.md` identifies the `track_scroll` / `scroll_wheel` additions and their call sites. The custom popup selector is documented in the vendor patch record.

## Confirmed findings

### YOMU-001 — Bulk removal leaves progress records behind

- **Severity / class:** P2; DATA_INTEGRITY, BUG.
- **Confidence / fix size:** High; small.
- **Locations:** [`src/app/actions.rs`](/home/sabisho/Projects/yomu/src/app/actions.rs:727), [`src/app/actions.rs`](/home/sabisho/Projects/yomu/src/app/actions.rs:733), [`src/app/actions.rs`](/home/sabisho/Projects/yomu/src/app/actions.rs:737), [`src/progress.rs`](/home/sabisho/Projects/yomu/src/progress.rs:98).
- **Affected path:** `Yomu::remove_books` → `Library::remove` for every selected ID → `save_library`. `save_library` calls `harvest_from_library`, which inserts/updates progress for present books but does not prune absent IDs.
- **Root cause:** Unlike `remove_book` (lines 668–673), bulk `remove_books` never calls `self.progress.remove(id)` and never schedules `save_progress` explicitly. Stale progress remains in the sidecar. At startup, `ProgressStore::apply_to_library` applies matching records to loaded books; if the exact path is imported again, its stable path-derived ID matches and its old progress is restored.
- **User scenario / reproduction:** Open and read a book; select it as part of a multi-selection and use bulk remove; reimport the same source archive. The old page/opened/last-read values may reappear.
- **Expected / actual:** Removing from Yomu should discard associated progress consistently with single-book removal. Bulk removal removes the book row but keeps its progress record.
- **Evidence:** Single-item removal explicitly invokes `self.progress.remove(id)` and `save_progress`; the bulk loop only calls `library.remove` and `selection.remove`. `ProgressStore::harvest_from_library` is additive and does not delete non-present IDs.
- **Regression test:** Remove one and several books through `remove_books`, inspect/reload `ProgressStore`, then reimport a removed path and verify it starts with no previous progress. Also verify source archives remain untouched.
- **Recommended fix:** Remove each ID from `ProgressStore` in the same bulk mutation and persist progress along with library state. Make both removal paths share the same domain cleanup to prevent drift.
- **Side effects:** Progress for removed entries becomes intentionally unrecoverable through normal reimport; this matches existing single-removal behavior and the removal confirmation UX.

### YOMU-002 — Corrupt progress recovery is silent and can discard the recoverable copy

- **Severity / class:** P2; DATA_INTEGRITY, BUG.
- **Confidence / fix size:** High; small to medium.
- **Locations:** [`src/library.rs`](/home/sabisho/Projects/yomu/src/library.rs:993), [`src/library.rs`](/home/sabisho/Projects/yomu/src/library.rs:994), [`src/progress.rs`](/home/sabisho/Projects/yomu/src/progress.rs:191), [`src/progress.rs`](/home/sabisho/Projects/yomu/src/progress.rs:195).
- **Affected path:** Startup `Library::load` calls `ProgressStore::load` only if the file exists, then discards any error via `if let Ok(progress)`. On JSON parse failure, `ProgressStore::load` attempts to rename the broken file to `progress.json.corrupt` and returns an error. Later reader activity saves a new progress file.
- **Root cause:** Recovery failure and corruption are not surfaced to the application. The library file has older duplicated progress fields, but normal page turns intentionally write only the sidecar. If that sidecar is corrupt, the app falls back to whatever progress was last copied into `library.json`, potentially much older, then overwrites the active sidecar with current in-memory state.
- **User scenario / reproduction:** Read several pages after the last library metadata save; cause a partial/invalid `progress.json` (for example, process interruption before replacement on a filesystem that does not uphold rename atomicity, or external damage); restart and continue reading. Startup quietly ignores progress-load failure and later persists the stale page.
- **Expected / actual:** Recovery should preserve any usable progress and clearly report when it cannot; actual startup silently proceeds with stale duplicated fields. The `.corrupt` rename is best-effort and its failure is also discarded.
- **Evidence:** `ProgressStore::load` returns an error after attempting backup; `Library::load` suppresses it. `save_progress` later serializes the fallback store.
- **Regression test:** Seed `library.json` with an older page, seed malformed `progress.json`, load through app startup and assert a visible/recoverable status and that the subsequent save does not silently replace the sidecar without user-visible recovery information. Include rename failure and pre-existing `.corrupt` backup cases.
- **Recommended fix:** Make the load result explicit to startup orchestration (including backup-rename outcome), surface a non-blocking recovery notice, and use a deliberate fallback policy. Avoid overwriting the only recoverable data until the user-visible recovery state is established; preserve the corrupt source if moving it fails.
- **Side effects:** Startup needs to carry a persistence warning into application state; this is modest additional state and must avoid blocking the UI.

### YOMU-003 — In-flight image decode memory is outside the cache budget

- **Severity / class:** P2; SECURITY, RESOURCE_LIFECYCLE, PERFORMANCE.
- **Confidence / fix size:** Medium; medium.
- **Locations:** [`src/imaging.rs`](/home/sabisho/Projects/yomu/src/imaging.rs:24), [`src/imaging.rs`](/home/sabisho/Projects/yomu/src/imaging.rs:148), [`src/app/reader_controller.rs`](/home/sabisho/Projects/yomu/src/app/reader_controller.rs:1010), [`src/app/reader_controller.rs`](/home/sabisho/Projects/yomu/src/app/reader_controller.rs:1202), [`src/app/reader_controller.rs`](/home/sabisho/Projects/yomu/src/app/reader_controller.rs:1216), [`src/archive.rs`](/home/sabisho/Projects/yomu/src/archive.rs:20).
- **Affected path:** `sync_reader` evicts completed images by `pagecache::system_memory_budget`, then requests multiple pages. Each task reads up to 256 MiB of compressed page data and decodes concurrently. `imaging::decode` accepts dimensions up to 120 million pixels and caps one decoder allocation at roughly 480 MB; JPEG fast path also decodes the full source before scaling. These source buffers and decode intermediates are not represented in `reader.images` or the eviction plan.
- **Root cause:** Memory policy applies after decoded images are complete. There is no aggregate in-flight byte budget or decode concurrency admission tied to the adaptive page budget. Cancellation is cooperative between archive read and decode, but cannot interrupt synchronous decode once it begins.
- **User scenario / reproduction:** Open an archive containing several near-limit, valid large scans on a constrained machine. Reader prefetch schedules current and neighboring images. Multiple workers can simultaneously hold compressed page bytes, full decode buffers, resize source/destination buffers, plus cached textures. Peak process memory can exceed the configured decoded-cache budget substantially and cause system pressure or process termination.
- **Expected / actual:** The reader should bound transient as well as resident image memory. The existing budget bounds only completed cached page bytes; it does not bound decode concurrency or temporary buffers.
- **Evidence:** `MAX_PIXELS = 120_000_000`; decoding allocates full-size decoded data before resizing in the JPEG path. The reader starts a background task for each requested page; up to the wanted prefetch set can be active. Page-cache accounting uses only `reader.images` entries.
- **Regression test / verification:** Add a deterministic decode-admission test with controlled jobs and accounting that proves concurrent worst-case reservations cannot exceed a defined cap; separately profile a synthetic large-page archive in an isolated test data directory. Do not use a user's real archive.
- **Recommended fix:** Add a small bounded decode admission mechanism (for example, a semaphore plus per-request reservation based on probed dimensions where available, conservative fallback otherwise) and include in-flight reservations in reader memory policy. Keep cancellation checks before starting a decode and release reservations on every result/cancel path.
- **Side effects:** Lower prefetch parallelism for very large pages may increase page-turn latency. The limit should preserve current responsiveness on ordinary scans.

## Design risk (not a confirmed current-version bug)

### YOMU-004 — Unknown library fields are discarded on save

- **Severity / class:** P3; ARCHITECTURE / DATA_INTEGRITY.
- **Confidence / fix size:** High for behavior; low for present user impact; medium.
- **Locations:** [`src/library.rs`](/home/sabisho/Projects/yomu/src/library.rs:920), [`src/library.rs`](/home/sabisho/Projects/yomu/src/library.rs:956), [`src/library.rs`](/home/sabisho/Projects/yomu/src/library.rs:1003).
- **Evidence and path:** `Library` is a closed Serde struct with no flattened extension map. Serde accepts unknown JSON keys by default but does not retain them; `Library::save` serializes only known fields. Opening a file written by a newer build and saving it in an older build therefore drops newer fields. `migrate` unconditionally writes the current format version on the next save.
- **Scenario:** A user downgrades Yomu, changes a setting or imports/removes a book, then upgrades again. A field introduced by the newer build may be gone.
- **Recommendation:** Decide and document supported downgrade/forward-compatibility policy. If forward preservation is required, retain unknown fields at the library/book/settings boundary or reject saves for a newer schema version with a recoverable message. This requires explicit migration/rollback policy; do not silently add broad generic schema scaffolding without confirming intended compatibility.
- **Test:** Round-trip an unknown root/book/settings field and verify the chosen policy.

## Important paths verified without a defect

- **Stale page completion:** `request_page` records a cancellation token per page; `page_decoded` checks both book ID and `Arc::ptr_eq` against the active token, ignores canceled/stale results, and does not add cancellation failures. A same-page higher-tier request is retried after current pending work completes. The documented A-after-B overwrite scenario is not reachable through the current callback gate.
- **Book switching:** `cleanup_current_reader` sets cancellation flags; page-list and probe callbacks check the active book ID. Decode completion after close or switch is rejected. Probe callbacks use book ID rather than reader-generation identity; behavior after in-place archive replacement during a same-ID reopen remains an untested hardening case.
- **Source preservation:** `Library::remove` deletes only cached covers and removes metadata. No comic source path is passed to `remove_file`.
- **Atomic persistence:** Library and progress saves serialize first, write a unique same-directory temp file, `sync_all`, rename, and attempt parent-directory sync on Unix. Replacement failures preserve the destination. Parent sync errors are intentionally ignored, so durability across sudden power loss is not guaranteed on every filesystem.
- **ZIP entry bounds:** Page reads reject declared oversized entries and stream at most `MAX_PAGE_BYTES + 1`; archive paths reject absolute, traversal, drive-colon, hidden, and null-name entries. ZIP extraction does not write archive paths to disk.
- **RAR bounds:** The local UnRAR patch supplies bounded reads and archive callers use `read_bounded`; RAR filenames are normalized and checked. RAR native runtime behavior on Windows/macOS was not executed here.
- **GPU lifecycle:** Reader eviction calls `window.drop_image`; reader cleanup queues images and drains them through the window. Main render also drains garbage before normal rendering.
- **Reset all:** Clears library and progress in memory, asynchronously clears cover directories, then schedules both progress and library saves. Cover-clear failure is ignored, so stale app cache files may remain, but this does not delete source comics or restore library entries.
- **Themes:** Parsing validates names, modes, tokens, and color syntax. Reload keeps the previous map on parse error and falls back from a removed selected custom theme. The reload method is marked dead-code and no automatic file watcher was found in the inspected paths; automatic theme-file replacement detection is therefore UNVERIFIED / apparently unsupported, not reported as a bug.
- **GPUI Kit patch:** `track_scroll` shares a caller handle and `scroll_wheel(false)` suppresses toolkit wheel bounce for the listed app scroll surfaces. Vendor patch record lists intended usage. Actual visual/platform momentum behavior needs native desktop testing.

## UNVERIFIED areas and limitations

- No test suite or build command was run. This audit is read-only and focused on source tracing; tests were inspected but not executed. The user requested command/test-result recording, so the result is **not run / no exit code** rather than an implied pass.
- Wayland compositor rendering, pointer occlusion during shutdown, real input routing, Linux kinetic scrolling, and GPU texture release under actual device loss were not manually exercised.
- Windows and macOS builds/runtime were not available. Native file dialogs, rename behavior, modifier mappings, RAR linking, native scroll/momentum, and window decorations remain UNVERIFIED.
- No adversarial archive was executed. Archive defenses were inspected statically; entry-count/aggregate archive metadata bounds and concurrent memory pressure need isolated adversarial verification.
- No performance profile or memory benchmark was run. YOMU-003 is based on concrete allocation limits and task concurrency, but its peak magnitude on a particular machine is not measured.
- Shutdown's quit-on-save-failure interaction and GPUI window-close behavior were inspected in code but not exercised on a native window.

## Commands and repository state

- Read-only inventory and targeted source reads used `pwd`, `git status --short`, `rg --files`, `find`, `wc`, `cat`, `sed`, and `nl`. Those commands completed successfully (exit code 0 in the captured tool calls).
- Git status before report creation was clean (empty `git status --short`).
- No tests, builds, formatting, clippy, audit scans, or GUI runs were invoked. No exit codes/test counts are claimed for them.
- After report creation, `git status --short` showed exactly the four requested untracked audit deliverables listed below and no other changes:
  - `docs/audits/ARCHITECTURE_INVARIANTS.md`
  - `docs/audits/ARCHITECTURE_RELIABILITY_REMEDIATION_ROADMAP.md`
  - `docs/audits/BUG_AND_USECASE_COVERAGE_MATRIX.md`
  - `docs/audits/DEEP_ARCHITECTURE_AND_CORRECTNESS_AUDIT.md`
- No application source, configuration, dependency, test, vendor, or user-data files changed.

