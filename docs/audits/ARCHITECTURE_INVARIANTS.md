# Yomu Architecture Invariants

Audit date: 2026-11-07. Verified invariants are distinguished from proposed invariants. These statements describe current observed behavior or identify a requirement that current code does not yet satisfy.

## Verified in the inspected implementation

1. **Comic source files are not removed by library removal.** `Library::remove` removes the book record and cached cover files only. Bulk and single removal call this method.
2. **Archive entry names are validated before page reads.** ZIP/RAR page reads reject absolute names, traversal segments, drive-colon strings, hidden/system path components, empty segments, and NUL bytes.
3. **ZIP page extraction has a per-page byte ceiling.** The declared size is checked and the read is limited to the configured cap plus one byte for oversize detection.
4. **RAR extraction uses the vendored bounded-reader API in the inspected call paths.** The RAR task and probing paths call bounded extraction and check cancellation between work units.
5. **Old page decode results cannot replace the current request for the same page index.** The callback checks the active cancellation token with `Arc::ptr_eq`, in addition to book identity and cancellation state.
6. **Cancelled page tasks do not enter `reader.failed`.** Cancellation errors are filtered, and stale/cancelled callback tokens return before result handling.
7. **Archive page indices are based on natural-sorted page-name order and zero-based vectors.** Reading direction changes display/navigation mapping, not page storage indexing.
8. **Library and progress writes use same-directory temporary files and replacement.** A failed replacement does not deliberately remove the prior destination. Unix parent-directory sync is attempted but its failure is ignored.
9. **UI-triggered library/progress saves are dispatched asynchronously.** Explicit synchronous save helpers are reserved for startup/test paths; actual UI action paths call background save methods.
10. **Reader texture eviction and closure go through GPUI image release.** Evicted images are passed to `window.drop_image`; queued reader garbage is drained by application rendering.
11. **Import cancellation is cooperative and late UI commit is guarded.** The worker checks its token, UI event handling checks active state/token, and quit sets the import token.
12. **The local GPUI Kit scroll patch uses a shared `ScrollHandle` and optional wheel bounce control.** Call sites use this to coordinate scrollbar/programmatic scrolling and route raw wheel events to Yomu where configured.

## Confirmed invariant violations to fix

1. **Removal must delete the matching progress record for every removed book.** Single removal currently does this; bulk removal does not (YOMU-001). A stale record can reappear if the same stable path ID is imported again.
2. **Corrupt progress recovery must not be silently ignored before a later save.** `Library::load` discards `ProgressStore::load` errors (YOMU-002), allowing stale duplicated library fields to replace newer progress after a later write.
3. **The adaptive cache budget must account for transient image decode allocations if it is intended as an overall memory bound.** Current eviction counts completed images only; in-flight decode buffers can exceed the budget (YOMU-003).

## Proposed invariants (not fully guaranteed today)

1. **A library save should not erase fields written by a newer compatible schema.** Current closed Serde structs discard unknown fields on round trip; product compatibility policy must decide preserve-versus-reject (YOMU-004).
2. **Progress corruption and failed backup operations should be visible and recoverable.** Current startup suppresses the error.
3. **The memory policy should include compressed bytes, decoder output, resize intermediates, completed CPU images, and GPU images or state explicitly which layers it bounds.** Current budget only accounts for cached page byte estimates.
4. **A page-probe completion should be tied to the exact reader generation/source snapshot, not just book ID, if a same-ID source can be replaced during a session.** The inspected callback checks book ID; behavior under in-place source replacement was not tested. This is a proposed hardening invariant, not a confirmed user bug.
5. **Every asynchronous operation that writes app-owned files should report failures in a recoverable way.** Import cover writes and reset cache clearing contain best-effort ignored failures; these affect caches, not source archives or library records.
6. **Automatic theme-file reload behavior should be explicit.** The inspected code exposes a reload method but no watcher path was found; live replacement behavior is UNVERIFIED and may not be a supported workflow.

