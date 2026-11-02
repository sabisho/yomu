# Yomu — Verified GPUI Kit 0.8.1 Component Mapping Specification

Every visual element designed in FigCraft is mapped directly to official **GPUI Kit 0.8.1** (`gpui-kit = "=0.8.1"`, `gpui-pre = 0.3.10`, `gpui-component = 0.8.1`) components and APIs verified against the installed crate source at `vendor/gpui-component/`.

---

## 1. Verified Native GPUI Kit Components vs Layout Primitives

We strictly distinguish between **Native GPUI Kit Components** (official composite structs in `gpui-component` with built-in accessibility, event dispatch, focus management, and theme tokens) and **GPUI Layout Primitives** (foundational GPUI elements for flex layout, coordinate placement, and custom canvas rendering).

### A. Official Native GPUI Kit Components (`vendor/gpui-component/src/`)

| Designed UI Element | Native Component Type | Source File Location | Official API Pattern & Implementation Notes |
|---|---|---|---|
| **App Titlebar & Chrome** | `TitleBar` | `src/title_bar.rs` | `TitleBar::new().child(...).window_controls()` with Wayland/macOS traffic light insets. |
| **Sidebar Navigation Pane** | `Sidebar`, `SidebarItem`, `SidebarGroup` | `src/sidebar/` | `Sidebar::new().header(...).child(SidebarGroup::new(...).child(SidebarItem::new(...)))` with active badge pills. |
| **Primary Action Button** | `Button` | `src/button/button.rs` | `Button::new("import").primary().label("+ Import")`, height 36px (`CONTROL_HEIGHT_PRIMARY`). |
| **Secondary Action Button** | `Button` | `src/button/button.rs` | `Button::new("cancel").outline().label("Cancel")`, height 32px (`CONTROL_HEIGHT_ACTION`). |
| **Ghost / Toolbar Button** | `Button` | `src/button/button.rs` | `Button::new("view_grid").ghost().icon(IconName::LayoutGrid)`. |
| **Destructive Button** | `Button` | `src/button/button.rs` | `Button::new("delete").destructive().label("Remove Item")` with Gruvbox danger token `#fb4934`. |
| **Segmented View Toggle** | `ButtonGroup` | `src/button/button_group.rs` | `ButtonGroup::new().child(Button::new("single")).child(Button::new("double"))` for PageLayout modes. |
| **Search Input Field** | `Input` | `src/input/mod.rs` | `Input::new(&search_state).prefix(IconName::Search).placeholder("Search titles, authors...")`. |
| **Sort / Option Selector** | `Select` | `src/select.rs` | `Select::new(&sort_state).options(vec!["Sort: Last Read", "Sort: Title", "Sort: Progress"])`. |
| **Action Dropdown Menu** | `DropdownMenu` | `src/menu/dropdown_menu.rs` | `DropdownMenu::new().trigger(Button::new(...)).menu(PopupMenu::new())`. |
| **Right-Click Context Menu** | `ContextMenuExt` | `src/menu/context_menu.rs` | `element.context_menu(|menu, cx| menu.item("Open Reader").item("Mark as Completed").separator().item("Remove"))`. |
| **Destructive Confirmation** | `AlertDialog` | `src/dialog/alert_dialog.rs` | `AlertDialog::new("remove_dialog").title("Remove from Library?").content("...").action_button(...).cancel_button(...)`. |
| **Modal Properties Dialog** | `Dialog` | `src/dialog/dialog.rs` | `Dialog::new("properties_dialog").header(DialogHeader::new("Archive Properties")).footer(...)`. |
| **Preferences Categories** | `Tabs`, `TabBar` | `src/tab/` | `Tabs::new().vertical().child(TabBar::new())` for the 6 preferences categories. |
| **Preferences Form Group** | `GroupBox` | `src/group_box.rs` | `GroupBox::new().title("Reader Engine").child(...)` for grouping related controls. |
| **Boolean Settings Toggle** | `Switch` | `src/switch.rs` | `Switch::new(&toggle_state).on_click(...)` for continuous scrolling, first page is cover, etc. |
| **Threshold / Ratio Slider**| `Slider` | `src/slider.rs` | `Slider::new(&slider_state).min(1.0).max(2.0).step(0.05)` for spread detection ratio. |
| **Library List View** | `List`, `ListItem` | `src/list/` | `List::new(&list_state).render_item(...)` with virtualized item recycling. |
| **Format & Count Badges** | `Tag` | `src/tag.rs` | `Tag::new("CBZ").color(TagColor::Blue)` for format pills and page counts. |
| **Keyboard Shortcut Badges** | `Kbd` | `src/kbd.rs` | `Kbd::new("/").size(KbdSize::Small)` for keybinding indicators. |
| **Reading Progress Bar** | `Progress` | `src/progress/` | `Progress::new().value(percentage)` with Gruvbox orange fill. |
| **Empty State Presentation**| `Empty` | `src/empty.rs` | `Empty::new().icon(IconName::BookOpen).title("Your Library is Empty").action(...)`. |
| **Background Spinner** | `Spinner` | `src/spinner.rs` | `Spinner::new().size(SpinnerSize::Small)` during archive parsing. |
| **Notification Toasts** | `Notification` | `src/notification.rs` | `Notification::info("Added to Favorites")` for transient confirmations. |

---

### B. Standard GPUI Layout Primitives (`gpui`)

These layout primitives are used solely for structural geometry and custom hardware canvas rendering, never as ad-hoc button or dialog reimplementations:

| Primitive Element | Module | Usage in Yomu Redesign |
|---|---|---|
| `div()` / `h_flex()` / `v_flex()` | `gpui::elements` | Window shell container, responsive flex grid layout, padding and margin alignment. |
| `canvas()` | `gpui::elements` | High-performance interactive reader viewport in `src/ui/reader.rs` for page texture drawing and gesture panning. |
| `img()` | `gpui::elements` | Hardware-accelerated image presentation for comic page textures streamed from background decoders. |
| `svg()` | `gpui::elements` | Vector glyph drawing for Lucide icon paths (`gpui_kit::assets::IconName`). |

---

## 2. Behavioral & Domain Model Invariants

- **Safety Invariant:** Deleting books, folders, or collections from the UI **never** deletes comic files from user storage. The destructive confirmation dialog explicitly communicates this.
- **Reading Order Invariant:** Archive entry indices are strictly zero-based and ordered physically. Reading direction (`ReadingDirection::RightToLeft` vs `ReadingDirection::LeftToRight`) alters only visual motion and navigation mapping, never archive indexing or caching logic.
- **Reading Modes:** Exactly two layout modes exist in Yomu: `PageLayout::Single` and `PageLayout::Double`. No non-existent "Webtoon" layout mode is introduced. Continuous vertical scrolling is a reading behavior toggle, not a layout mode enum.
- **Settings Categories:** Exactly six categories from `src/ui/settings_view.rs`:
  1. `General`: Continuous scrolling toggle, About Yomu v0.1.0.
  2. `Appearance`: Theme Mode (Dark/Light/System), Color Scheme (Gruvbox Dark / Gruvbox Light ONLY), Card corner rounding, Card border size.
  3. `Reader`: Page Layout (Single/Double), Reading Direction (RTL/LTR), Spread Detection Ratio (1.25), First page is cover toggle, Remember zoom toggle, Vim keybindings toggle.
  4. `Library`: Series grouping toggle, Thumbnail size preset.
  5. `Shortcuts`: Interactive keybinding cheat-sheet table.
  6. `Advanced`: Cover cache size + Clear Cache, Reset Settings, Reset Library & Data.
- **Permanent Animation Policy:** Animations remain permanently enabled across all platforms; application initialization explicitly enforces `cx.set_reduce_motion(false)`. No animation disable option is exposed.
- **Strict Lint Compliance:** Safe Rust 2024 under `#![forbid(unsafe_code)]` with zero Clippy warnings (`cargo clippy --all-targets --all-features -- -D warnings`).
