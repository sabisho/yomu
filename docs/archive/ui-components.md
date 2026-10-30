# GPUI Kit UI Components Documentation

This document describes the modular UI components created for the Yomu macOS redesign, their properties, interactions, and integration with the GPUI Kit component ecosystem.

---

## 1. Component Architecture Overview

All redesigned UI components reside in `src/ui/`:

- `src/ui/sidebar.rs`: Persistent macOS two-pane navigation sidebar.
- `src/ui/toolbar.rs`: Window header toolbar with history navigation, search, and view mode toggles.
- `src/ui/continue_reading.rs`: Hero manga banner, Up Next queue, and Recently Added cover cards.
- `src/ui/details.rs`: Manga details inspector with 4x2 metadata grid, synopsis toggle, tabs, and sync card.
- `src/ui/collections_view.rs`: Smart collections (Favorites, Reading, Want to Read, Completed) and user custom collections.
- `src/ui/settings_view.rs`: Split-pane macOS preferences with 6 dedicated categories.
- `src/ui/components.rs`: Reusable atomic primitives (window controls, badges, genre tags).
- `src/ui/overlays.rs`: Modal overlays, confirmation dialogs, and popups.

---

## 2. Component Specifications

### 2.1 Navigation Sidebar (`ui::sidebar`)

```rust
pub struct SidebarProps {
    pub current_section: SidebarSection,
    pub continue_reading_count: usize,
    pub all_manga_count: usize,
    pub collections_count: usize,
    pub downloads_count: usize,
    pub colors: UiColors,
    pub radius: Pixels,
}
```

- **Traffic Light Controls**: Custom-rendered red, yellow, and green window management buttons mapped to `remove_window`, `minimize_window`, and `toggle_fullscreen`.
- **Primary Navigation**: Continue Reading, All Manga, Collections, History, Downloads.
- **Footer**: Direct link to Settings.

### 2.2 Main Toolbar (`ui::toolbar`)

```rust
pub struct MainToolbarProps<'a> {
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub search_input: Option<&'a Entity<InputState>>,
    pub is_grid_mode: bool,
    pub colors: UiColors,
    pub radius: Pixels,
}
```

- **Navigation Arrows**: Forward and Back navigation buttons honoring window history stack.
- **Search Bar**: Centered rounded text input with leading search icon and trailing `⌘K` keyboard accelerator pill.
- **View Mode Switcher**: Segmented toggle between Cover Grid and List view.
- **Action Button**: Primary `+ Import` button with accent styling.

### 2.3 Continue Reading View (`ui::continue_reading`)

- **Hero Banner**: Large featured cover preview, reading progress bar, chapter info (`Ch. 12 of 180`), genre tags, primary Continue Reading button, and context menu trigger.
- **Up Next Queue**: Compact right-hand column displaying the top in-progress volumes sorted by recency.
- **Recently Added Grid**: Responsive multi-column grid of recently imported books with thumbnail cover art, progress percentage, and more-options menu.

### 2.4 Manga Details View (`ui::details`)

- **Header Section**: Full-height high-resolution cover preview, title, series, format, and reading status.
- **Underline Tab Bar**: Overview, Chapters, and Details.
- **4x2 Metadata Matrix**: Structured cards showing Chapters, Reading Status, Pages, Format, and Storage Info.

### 2.5 Collections View (`ui::collections_view`)

- **Smart Collections**: 2x2 grid featuring Favorites (Heart), Reading (Book), Want to Read (Clock), and Completed (Checkmark).
- **Custom Collections**: Folder cards with custom user tags, item count indicators, and collection deletion/renaming context actions.
- **New Collection Action**: Header button opening a modal dialog with sanitized name validation.

### 2.6 Split-Pane Settings (`ui::settings_view`)

Implements 6 settings tabs in a macOS-standard 200px sidebar + scrollable content pane:
1. **General**: Startup behavior, update checks, default directories.
2. **Appearance**: Theme selection (macOS Light, Dark, Catppuccin, Noctalia), corner rounding, border thickness.
3. **Reader**: Reading direction (LTR vs RTL), Continuous vertical scroll toggle, Webcomics mode, page turn animations.
4. **Library**: Folder & series grouping, thumbnail size cycle.
5. **Shortcuts**: Keybindings table for reader, library, and vim navigation.
6. **Advanced**: Cache size calculation, clear cache action, settings reset, and full data wipe with safety confirmation.

### 2.7 Atomic Visual Components (`ui::components`)

- `mac_window_controls(on_close, on_min, on_zoom)`: Pixel-precise window controls with hover states.
- `stat_badge(count, active, colors)`: Pill counter with inverted active color schemes.
- `genre_tag(label, colors)`: Compact chip for manga categories.
