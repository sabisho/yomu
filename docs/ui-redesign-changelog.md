# Yomu macOS UI Redesign Changelog

This document tracks all added, modified, refactored, and deprecated UI components, models, and styling systems across the macOS redesign.

---

## 1. Added Components & Modules

### 1.1 Navigation & Shell (`src/ui/sidebar.rs`, `src/ui/toolbar.rs`)
- `SidebarProps` & `render_sidebar`: Persistent two-pane sidebar implementing macOS traffic light controls, navigation items, real-time AniList count indicators, and user account footer.
- `MainToolbarProps` & `render_main_toolbar`: Unified toolbar featuring forward/backward history navigation, centered search input with `⌘K` accelerator badge, cover grid/list view mode segmented switcher, and primary `+ Import` action.

### 1.2 Core Manga Views (`src/ui/continue_reading.rs`, `src/ui/details.rs`, `src/ui/collections_view.rs`)
- `ContinueReadingProps` & `render_continue_reading_view`: Hero featured manga banner with high-resolution cover preview, chapter progress bar, and primary Continue Reading CTA; Up Next queue column; Recently Added responsive cover grid.
- `MangaDetailsProps` & `render_manga_details`: High-fidelity manga inspector featuring star rating display, romaji/native titles, expandable synopsis, underline tabs, 4x2 metadata matrix (Chapters, Status, Published, Authors, Artists, Serialization), and bottom AniList tracking & progress card.
- `CollectionsProps` & `render_collections_view`: 2x2 smart collections (Favorites, Reading, Want to Read, Completed) and user-created custom category folders with deletion/rename context menus.

### 1.3 AniList Tracking & GraphQL Search (`src/ui/anilist_search.rs`, `src/anilist.rs`)
- `AniListSearchProps` & `render_anilist_search`: Real-time manga discovery view with filter chips (All, Title, Author, Genre, Tag), cover previews, metadata lines, and quick "+ Add to Planning" button.
- `AniListClient` & `AniListData`: Official AniList GraphQL API client (`search_manga`), persistent metadata caching (`anilist.json`), rate limiting (90 req/min), and offline sync update queue (`QueuedSyncItem`).

### 1.4 Split-Pane Preferences (`src/ui/settings_view.rs`)
- `SettingsViewProps` & `render_settings_view`: macOS HIG-compliant split-pane preferences featuring 7 category tabs: General, Appearance, Reader, Library, AniList, Shortcuts, and Advanced.

### 1.5 Shared Visual Primitives (`src/ui/components.rs`)
- `mac_window_controls`: Authentic red/yellow/green window management controls.
- `stat_badge`: Numerical count badges for sidebar and collection items.
- `genre_tag`: Compact rounded chip for categories and genres.
- `star_rating`: 5-star visual rating component calculating score percentage and popularity.
- `section_header`: Consistent typography headers with optional trailing action links.

---

## 2. Modified Modules

### 2.1 Core Library Models (`src/library.rs`, `src/collections.rs`)
- Added `ReadingDirection` enum (`LeftToRight`, `RightToLeft`) with serde compatibility.
- Added `Collection` struct with ID, name, book ID list, and timestamp tracking.
- Added collection management and favorite toggling methods on `Library`: `add_collection`, `rename_collection`, `delete_collection`, `add_book_to_collection`, `remove_book_from_collection`, `is_favorite`, `toggle_favorite`.
- Added `DataDirs::root(&self)` accessor for unified configuration and cache file paths.

### 2.2 Reader Experience (`src/main.rs`)
- Hidden sidebar during active reading sessions to maximize viewport area.
- Added floating auto-hiding top bar with back navigation, book title, chapter progress, page fit mode selector, reading direction switcher, and fullscreen toggle.
- Added semi-transparent left and right edge chevrons for intuitive page turning.
- Integrated bottom HUD pill for unobtrusive reading status.

---

## 3. Deprecated & Removed Code
- Deprecated permanently visible 3rd pane inspector in favor of dedicated details view and modal inspectors.
- Replaced monolithic legacy settings popup with split-pane macOS preferences view.
- Removed ad-hoc styling and hardcoded RGB values in favor of centralized `UiColors` semantic theme tokens.

---

## 4. Keybinding & List View Modernization

### 4.1 Dynamic Host OS Check & Shortcut Strings
- Created OS shortcut helpers in `src/ui/components.rs`: `search_shortcut_str()`, `settings_shortcut_str()`, `open_shortcut_str()`, and `zoom_shortcut_str()`.
- Dynamically outputs `⌘` symbols (`⌘K`, `⌘,`, `⌘O`, `Cmd+Wheel`) on macOS and standard `Ctrl` symbols (`Ctrl+K`, `Ctrl+,`, `Ctrl+O`, `Ctrl+Wheel`) on Linux and Windows.
- Dynamically applied across toolbar search suffix, shortcuts settings table, sidebar settings tooltip, and import button.

### 4.2 Functional List View
- Added `view_mode_grid: bool` to `Settings` in `src/library.rs` (default `true`, persists across launches in `library.json`).
- Implemented `render_book_list_row` and `render_folder_list_row` in `src/main.rs`.
- Integrated across `AllManga`, `History`, and `AniListCategory` views with full selection, context menu, and navigation support.

### 4.3 Separated Icon-Pair Segmented Switches
- Separated folder grouping and list/grid view mode into two independent switches in `src/ui/toolbar.rs` with a 10px spacing gap.
- Both switches use clean icon pairs with zero text:
  - Folder Grouping: `Folder` (series folders) vs `BookOpen` (individual volumes).
  - View Mode: `LayoutGrid` (grid view) vs `List` (list view).

### 4.4 Comprehensive Native Tooltips
- Added native GPUI Kit `Tooltip` across traffic lights, toolbar controls, manga details actions, reader controls, and settings.
