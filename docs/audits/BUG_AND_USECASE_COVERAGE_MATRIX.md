# Bug and Use-Case Coverage Matrix

Audit date: 2026-11-07. “Inspected” means the source path was traced; it does not mean a test was executed. No test/build commands were run during this read-only audit.

| Workflow | Scenario | Code path | Existing coverage | Finding ID | Status |
|---|---|---|---|---|---|
| Import | Import one CBZ/ZIP/RAR | `main.rs::import_paths` → `library::import_paths_with_progress_cancellable` → `archive::list_pages` → `build_book` | Library archive/import unit tests; not run | — | Inspected; ordinary path covered in source tests |
| Import | Import a folder with nested volumes | `library.rs::collect` → candidate scan → `build_book` | Nested and grouping tests exist in library and UI tests; not run | — | Inspected |
| Import | Reimport same canonical path | `book_id` path hash → existing ID set → skip; `Library::add` duplicate guard | Reimport tests exist; not run | YOMU-001 | Stale sidecar progress can resurrect after bulk removal |
| Import | Import unsupported, empty, corrupt, missing input | candidate errors / empty-page validation / `build_book` | Unsupported/empty folder tests exist; coverage for every archive corruption class not established | — | Inspected; comprehensive malformed archive behavior UNVERIFIED |
| Import | Cancel during directory scan, decode cover, or shutdown | import token checks; receiver rejects token after cancellation; quit sets token | Cancellation tests exist; not run | — | Inspected |
| Import | Very large archive entry count | ZIP `file_names().collect`; RAR listing loop; page vector | No bounded entry-count assertion identified | INVESTIGATE-001 | Metadata/page-name aggregate resource bound not verified |
| Import | Archive entry traversal or absolute path | `is_safe_archive_entry_name`; ZIP/RAR page listing and reads | Path validation tests exist; not run | — | Inspected; extraction-to-disk path is not used |
| Import | ZIP decompression bomb | `read_zip_page` declared and streamed byte checks | Adversarial ZIP size-spoofing tests exist; not run | — | Inspected; no defect confirmed |
| Import | RAR decompression bomb | patched `unrar::ReadBoundedToVec` / `read_bounded` callers | RAR hardening tests exist in source; native backend not executed | — | Static path verified; platform runtime UNVERIFIED |
| Import | Source file renamed/moved or made inaccessible | `open_book` checks path existence; later archive reads propagate errors | Missing-source UI tests not fully established from this audit | INVESTIGATE-002 | Reader-open and later-read UX should be exercised |
| Library | Remove one book without deleting source | `actions.rs::remove_book` → `Library::remove`; sidecar removal | Removal/data safety tests exist; not run | — | Inspected; source file is untouched |
| Library | Bulk remove book(s) and reimport | `actions.rs::remove_books` → library save only | No regression coverage found for sidecar cleanup | YOMU-001 | Confirmed stale progress record |
| Library | Reset all app data | `main.rs::clear_all_data` clears library/progress, async cover clear, then saves | Persistence reset tests exist; not run | — | Inspected; cover cleanup failure is silently ignored |
| Library | Groups/Volumes, search, sort, selection | `app/views/library.rs` and `ui/library_view.rs`, search/selection helpers | Extensive UI state tests exist; not run | — | Source-level inspection only; visual and large-library behavior UNVERIFIED |
| Library | Reimport an ID after normal single removal | removal removes progress and metadata | Single removal path differs from bulk; no end-to-end reimport test confirmed | YOMU-001 | Single path clears sidecar; bulk path does not |
| Reader | Open/close book and list pages asynchronously | `reader_controller::open_book` → `pages_listed` → `sync_reader` | Reader tests exist; not run | — | Inspected |
| Reader | Rapid A→B navigation; A finishes late | `request_page` token → `page_decoded` token identity and cancellation checks | Cancellation tests exist; no race stress run | — | Inspected; stale callback rejected |
| Reader | Rapidly switch books while page list/decode remains active | cleanup cancellation, book ID guard in callbacks | Existing tests include cancellation lifecycle scenarios; not run | — | Inspected; same-ID reopen and changed archive contents deserve a focused test |
| Reader | Rapid prefetch with very large pages | `sync_reader` eviction → multiple `request_page` tasks → `decode_fit_bgra` | No aggregate transient decode-memory budget test identified | YOMU-003 | Confirmed budget gap; measured impact UNVERIFIED |
| Reader | Canceled page remains requestable | canceled token removed from pending; canceled error not added to `failed` | Source tests document this invariant; not run | — | Inspected |
| Reader | Page progress survives normal restart | `ProgressStore` autosave → library startup overlay | Persistence tests cover rapid navigation/quit paths; not run | YOMU-002 | Corrupt-sidecar fallback can lose newest progress silently |
| Reader | Single/double spread, LTR/RTL, continuous/webcomic | layout/spread plan + controller mapping + render | Many layout/reader tests exist; not run | — | Inspected selectively; native visual behavior UNVERIFIED |
| Reader | Resize, zoom, scroll, mouse, touchpad | `reader_controller` events, reader view, scroll component | UI tests and pure layout/motion tests exist; not run | INVESTIGATE-003 | Cross-platform input behavior requires native verification |
| Persistence | Save interrupted before rename | temp file sync and atomic replace | Atomic write and retry tests exist; not run | — | Source path is atomic at destination; parent sync failures ignored |
| Persistence | Corrupt `library.json` | `Library::load` backs up and returns error | Corruption tests exist; not run | — | Startup fallback experience not exercised here |
| Persistence | Corrupt `progress.json` | `ProgressStore::load` backup error swallowed by `Library::load` | Parse/backup tests may exist; no app-level visible recovery check identified | YOMU-002 | Confirmed silent fallback path |
| Persistence | Two saves complete out of order | per-file write lock + monotonic sequence guard | Extensive persistence ordering tests exist; not run | — | Inspected; no stale-write defect confirmed |
| Persistence | Downgrade/forward schema round trip | Serde library parse → migrate → save | Legacy compatibility tests exist; unknown future fields not tested | YOMU-004 | Compatibility design risk |
| Settings | Change settings, restart, reset defaults | settings mutation and library save; `clear_all_data` | Settings/persistence tests exist; not run | — | Inspected; every setting path not individually enumerated |
| Themes | Malformed custom theme parse/reload | `parse_custom_themes`, `reload_custom_themes` | Theme parsing tests exist; not run | — | Parse retains prior map on error; live file watcher not found |
| Themes | Theme file replaced while app is open | explicit `reload_custom_themes` only | No watcher path found; reload is marked dead-code | INVESTIGATE-004 | Automatic replacement reload is unverified/apparently unsupported |
| GPU | Eviction and reader close release images | `window.drop_image` in `sync_reader`, garbage drain in main render | UI tests cannot establish driver resource release | — | Static lifecycle verified; physical GPU behavior UNVERIFIED |
| GPU | GPU loss/init failure | GPUI window/image creation paths | No native device-loss test identified | INVESTIGATE-005 | Platform/backend behavior UNVERIFIED |
| Platform | Linux Wayland and X11 fallback | GPUI startup and `PlatformScrollController` | Prior audit docs report Wayland screenshots; not repeated here | INVESTIGATE-003 | Current compositor behavior UNVERIFIED |
| Platform | Windows/macOS filesystem, shortcuts, RAR, scroll | cfg/platform code and native GPUI integrations | No native platform execution in this environment | INVESTIGATE-006 | UNVERIFIED |
| Shutdown | Quit with pending writes/import | `request_quit`, flush locks, quit-on-success / force-quit dialog | Persistence shutdown tests exist; not run; no live window interaction | — | Source-level path inspected |

## Investigation items

- **INVESTIGATE-001:** Decide whether archive entry count and aggregate filename storage need explicit resource caps. Test with generated archives in isolated temporary directories.
- **INVESTIGATE-002:** Verify user-facing recovery when a source archive disappears after import or becomes unreadable during reading.
- **INVESTIGATE-003:** Run real Wayland input/scroll/resizing checks and compare with native macOS momentum and Windows behavior.
- **INVESTIGATE-004:** Confirm product expectation for automatic theme-file watching versus manual reload. Do not classify the lack of a watcher as a defect unless live reload is an intended workflow.
- **INVESTIGATE-005:** Exercise image upload/drop and device-loss behavior on supported GPUI backends.
- **INVESTIGATE-006:** Build and run on Windows and macOS; static source inspection is not a substitute for platform qualification.

