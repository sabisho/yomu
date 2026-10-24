# Architecture

## Runtime shape

The binary starts in `src/main.rs`, initializes GPUI Kit and the app window, then owns the `Yomu` application state. The library crate (`src/lib.rs`) exports the core modules and deliberately does not depend on GPUI/Kit. This keeps most behavior deterministic and testable without creating a window.

```text
GPUI Kit window / Yomu (src/main.rs)
  ├─ Library + Settings (src/library.rs) ── JSON + covers in XDG data dir
  ├─ Reader state ── page list from archive.rs
  │                    └─ async read/decode/scale ── pagecache.rs policy
  ├─ Input parser (vim.rs) ── actions applied to reader
  ├─ View geometry (layout.rs) and motion (motion.rs)
  └─ menu.rs / selection.rs ── UI-independent interaction rules
```

## Modules and ownership

| Module | Responsibility | Important boundaries |
|---|---|---|
| `main.rs` | GPUI Kit app/window, state router (`MainView`), key/mouse dispatch, overlays/dialogs, rendering two-pane shell and reader, background-task coordination | UI integration layer; routes between persistent two-pane shell (`render_two_pane_shell`) and immersive reader (`render_reader`). Navigation history (`nav_history`) supports back/forward navigation. |
| `ui/` | Modular GPUI Kit view components conforming to macOS HIG design board | Submodules: `sidebar` (navigation & window controls), `toolbar` (history, search badge, view modes, import), `continue_reading` (hero & queue), `details` (manga detail sheet & sync status), `collections_view` (smart & user collections), `anilist_search` (GraphQL search screen), `settings_view` (7-tab preferences), `components` (stat badges, tags, ratings). |
| `anilist.rs` | AniList GraphQL client, metadata search, progress sync, rate limiter, and local offline cache | Connects to official AniList GraphQL API (`https://graphql.anilist.co`). Rate limited to 90 req/min. Persists offline queue and media cache in `anilist.json`. Zero fictional data. |
| `collections.rs` | Custom user collections model and smart collection query primitives | Decoupled from book disk storage. Deleting a collection only deletes list groupings, never user comic files. |
| `archive.rs` | Enumerate and read CBZ/ZIP pages or images in folders | Filters supported image names, ignores hidden/resource-fork files, natural-sorts names, limits archive page inflation. Source types are `Archive` and `Folder`. |
| `imaging.rs` | Decode supported images, scale for display, create JPEG covers | No gpui dependency. Decoded pixel buffers are BGRA for gpui; original dimensions are retained for layout. Has dimension guardrails. |
| `library.rs` | Book model/status/progress, settings, JSON load/save, imports, generated covers | `DataDirs` resolves storage. `Library` owns persisted data; comic files remain at their original paths. Format version 3. Continuous mode and webcomics are lenient settings defaulting off; folder grouping defaults on while honoring explicit saved values. Each book's optional series name defaults to unassigned when loading older data. |
| `layout.rs` | Fit modes, zoom bounds/steps, viewport geometry, pan offsets, decode tiers | Pure math. Page indices and geometry do not depend on reading direction except arrival placement. |
| `vim.rs` | Turn key tokens and command-line input into reader actions | Knows no UI. The UI consumes `Action` values and performs side effects. |
| `motion.rs` | Frame-time based scroll easing, page-turn transitions, reduced-motion detection | Pure calculations/state machines; avoid tying animation duration to frame count. |
| `pagecache.rs` | Prefetch order and eviction planning | Pure policy. Current page is preserved even if that alone exceeds the budget. Current constants: 5 ahead, 3 prefetched behind, keep 4 behind/5 ahead, adaptive 1.0–3.0 GiB decoded-page budget. |
| `selection.rs` | Library multi-selection and range anchor semantics | Pure state; return selected IDs in library order at the call site for stable batch operations. |
| `search.rs` | Case-insensitive title/source-path/series substring matching | Pure helper; query state is transient in the UI and search never changes library ordering or stored books. |
| `menu.rs` | Context menu actions, navigation, confirmations, properties rows | Pure menu rules; UI decides how entries are drawn and dispatched. |
| `util.rs` | Natural ordering, FNV-1a IDs, date/size helpers | Shared small helpers. |

## Main data and control flows

### Import

`library::import_paths_with_progress` expands selected files/folders into candidates, scans image folders with a depth limit, determines source kind, enumerates pages, creates a stable path-derived ID and generates a cover. It invokes a callback after each cover is ready. `main.rs` forwards each book through a channel to the UI task, adds it, and saves immediately. Library order is persisted insertion order; imports append in natural directory scan order. An imported book references the original path; it is not copied. Archive and folder page names are naturally sorted.

### Opening and reading

The UI gets the selected `Book`, enumerates its pages with `archive::list_pages`, clamps/restores its saved zero-based page index, and requests bytes and image processing in background work. `imaging::decode_fit_bgra` returns pixels scaled to an appropriate device-pixel tier chosen using layout/zoom. gpui image objects are then used by rendering. Page index updates and progress persistence happen on navigation; direction changes which control means forward/backward but does not change the index model.

`pagecache` determines what should be loaded and retained. A cache change should preserve the current page, bound decoded buffers, and account for image dimensions/bytes. The policy's adaptive budget (1.0 GiB default, scaling up to 3.0 GiB depending on available host RAM) is a decoded image budget, not a hard bound on all process memory: GPU textures, source bytes, allocator overhead, and transient decode buffers can add to RSS.

### Two-Pane Shell & View Navigation

The app layout follows the approved macOS Human Interface Guidelines design specification (`Screenshots/macOS_redign_design_board.png`):
- **Persistent Sidebar (`src/ui/sidebar.rs`)**: 240px fixed width featuring macOS traffic-light window controls (Close, Minimize, Zoom), primary navigation categories (`ContinueReading`, `AllManga`, `Collections`, `History`, `Downloads`), AniList status categories (`Reading`, `Plan to Read`, `Completed`, `Paused`, `Dropped`), and bottom profile & settings trigger.
- **Top Toolbar (`src/ui/toolbar.rs`)**: 52px fixed height with history back/forward buttons (`< >`), centered search field with interactive `⌘K` badge, layout grid/list toggle, and primary `+ Import` action button.
- **State Router (`MainView`)**: The active screen is driven by `self.active_view` in `Yomu`, with full browser-like history tracking via `nav_history` and `nav_history_idx`. Views include Continue Reading, All Manga, Manga Details, Collections, Downloads, History, AniList Category, AniList Search, and Settings.
- **Manga Details View (`src/ui/details.rs`)**: Replaces the former slide-over drawer with a rich dedicated screen featuring high-resolution cover display, interactive star rating, genre pill tags, collapsible synopsis, underline tab navigation, 4x2 metadata matrix, and bottom sync tracking card.

### AniList Tracking & Discovery Subsystem

The AniList integration (`src/anilist.rs`) communicates with the official GraphQL endpoint at `https://graphql.anilist.co`:
- **Search & Discovery**: Submits GraphQL queries with sanitized titles, returning official metadata, synonyms, genres, studio/author staff credits, and cover media.
- **Rate Limiting & Safety**: Strictly enforces a sliding window rate limiter capped at 90 requests per minute to comply with API quotas.
- **Offline Sync Queue**: Progress updates made while offline or unauthenticated are queued in `QueuedSyncItem` buffers and synchronized automatically when network availability is restored.
- **Local Persistence**: User authentication tokens, matched book mappings, synced progress, and media metadata are persisted locally in `anilist.json` in the user's data directory.

### Collections & Custom Groups

Custom user collections (`src/collections.rs`) provide flexible organizational taxonomies:
- 4 built-in smart collections: *Favorites*, *Plan to Read*, *Completed*, *Recently Added*.
- User custom collections with custom names, creation timestamps, and book ID memberships.
- Complete isolation from underlying comic storage: adding, renaming, or deleting collections never touches or removes the comic files on disk.

### Immersive Reader

When opening a book, the reader enters an edge-to-edge reading mode:
- The persistent sidebar auto-hides completely to maximize reading surface area.
- Top navigation bar auto-hides, exposing fit mode toggles, reading direction selectors, chapter/page titles, and back-to-library navigation.
- Lateral hover chevrons appear on left and right window margins for mouse-driven page turns.
- A floating translucent bottom pill displays precise reading progress with page slider and scrub preview.

### Persistence

Default storage is `$XDG_DATA_HOME/yomu/` (normally `~/.local/share/yomu/`); `YOMU_DATA_DIR` overrides the root. `library.json` contains the version, books, progress, marks, and settings. Generated cover JPEGs live in `covers/<book-id>.jpg`. The comic source is never modified by library operations.

Reader and library preferences currently live in `library.json`. `Settings::from_value` defaults missing/invalid fields and clamps opacity. The separate XDG config file `$XDG_CONFIG_HOME/yomu/themes.toml` contains only user-defined theme configs; `themes.example.toml` documents its supported schema. When a default's meaning changes, update the library version/migration deliberately; migrations should be idempotent and covered by tests.

### Input and animation

`vim::Vim` parses tokens and command-line text into `Action`s; `main.rs` applies actions. `layout.rs` calculates fit/zoom/pan; `motion.rs` advances transitions and scroll using elapsed frame time. Respect the environment/desktop reduced-motion policy as well as the stored animation preference.

Continuous mode is owned by `Reader` and persisted in `Settings`. It stacks page images vertically at the current viewport width, keeps known source dimensions separately from evictable decoded textures, estimates unknown pages as portrait until decode completes, and updates the saved current page as the viewport crosses pages. Cache retention follows the visible range plus nearby pages, subject to the existing decoded-memory budget. Webcomics mode is a persisted override that forces this layout and sets the inter-page gap to zero; turning it off restores the saved continuous/single-page preference. Single-page mode retains page-turn behavior.

## Invariants worth preserving

- Library-owned mutations are saved to disk; removal forgets metadata and covers but never deletes source comics.
- `Book.series` is optional persisted manual grouping metadata; assignment trims and normalizes whitespace, and reuses existing spelling case-insensitively to avoid duplicate groups. Clearing a manual assignment restores automatic grouping by the book path's parent directory.
- Directory grouping is on by default. Groups with at least two books render as folder cards; the first member's cover is the folder thumbnail. Opening a card navigates to a second view of its volumes. Both group cards and members retain library/import order. The search filter is applied to group contents and individual items.
- A zero-based page index is clamped against the enumerated page count, and finished status means the final page is current.
- Archive/folder sorting is natural (e.g. `2` before `10`) and hidden junk is excluded.
- Archive entry size and decoded image pixel count have defensive limits.
- Page rendering consumes BGRA buffers; changing channel order can produce incorrect colors.
- Input parsing and core state rules remain independent of gpui where practical.
- The active GPUI Kit theme is the source of UI colors. Do not reintroduce a fixed Yomu palette. The library toolbar intentionally has no app name, total book count, opaque fill, or bottom rule; keep search centered and actions in the upper-right menu.
- Async work must not apply stale results to a different book/page/request. Follow the existing request identity/cancellation checks in `main.rs` when extending it.

## Testing map

Tests are mostly colocated with each module. Pure logic tests cover archive ordering/limits, geometry, status/progress, migration/settings parsing, selection, menu rules, input parsing, page-cache eviction and animation timing. `main.rs` also contains gpui test-support tests for interactions and view state. Inspect nearby tests before extending a behavior; tests may reveal edge cases beyond the README description. `cargo test perf_ -- --ignored --nocapture` runs ignored benchmarks.

The Nix package sets `doCheck = false`; packaging builds do not run the test suite. The development shell's `devenv test` runs `cargo test`.
