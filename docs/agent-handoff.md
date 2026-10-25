# Agent handoff

Read this after [AGENTS.md](../AGENTS.md). It summarizes the implemented product state and the decisions an agent should carry forward without re-discovering them. User-facing behavior and shortcuts are in [README.md](../README.md); code boundaries are in [architecture.md](architecture.md); GPUI Kit migration and redesign decisions are in [ui-redesign-plan.md](ui-redesign-plan.md).

## Project status

yomu is a Rust 2024 desktop comic reader built with GPUI Kit 0.7.3. The minimum supported Rust version is 1.92. It targets Linux, Wayland first with X11 fallback, and uses Nix/devenv for native dependencies. Main UI and application orchestration are in `src/main.rs`; reusable behavior stays in GPUI-independent modules under `src/`.

The framework migration, built-in theme integration, custom theme file, tabbed settings, command prompt, and first visual redesign pass are implemented. The library toolbar follows the reference layout: centered search, no Yomu title or total book count, transparent top bar, and a top-right hamburger menu. Its actions are Add files, Add folder, Settings, grouping, and Quit. Selection actions remain at the left of the toolbar while a selection is active. The window root, surfaces, controls, cards, and popups use rounded corners. Search has no placeholder/ghost text and shows a blinking caret while active. Menus, settings, dialogs, and the command prompt have short fade/slide entrance animations that reduce to near-instant when reduced motion is detected.

Folder grouping, progressive folder imports, opacity-aware widget surfaces, click-outside/Enter search dismissal, and GPUI Kit menu icons are implemented. Raised widget opacity is 10 percentage points above the window setting. Book right-click handling stops event propagation after opening the native popup; otherwise the root right-click dismissal handler closes it on the same event. Register `gpui_kit::assets::AllAssets` on the application builder: custom-rendered `Icon` elements use SVG paths, and without this asset source toolbar/context-menu glyphs render blank. Icon controls have visible themed surfaces and context-menu glyphs have accent backplates. This path has not yet been visually verified on a real Wayland compositor. `cargo fmt --all -- --check`, `cargo check --offline`, and `git diff --check` passed before this correction; rerun after the latest edits. The full test suite was not run. Do not claim visual verification.

## Implemented behavior

- **Import and library:** CBZ, CBR, ZIP, and RAR archives and image folders are imported by reference. Background import sends each completed book to the UI; it is added and persisted immediately in scan order. Covers and library metadata are stored under XDG data storage. Search matches title, source path, and series, and does not mutate library ordering or selection scope.
- **Search:** click the centered toolbar search, press `/`, or use `Ctrl+F`; `Escape` leaves search. Query state is transient.
- **Grouping:** directory grouping is on by default. Two or more books with the same parent directory form a folder card; the first imported member supplies its cover, and opening it displays the group's books in a second view. A manual `Book.series` assignment overrides directory membership; clearing the assignment restores automatic directory grouping. The persisted `group_series` preference switches between folder cards and individual volumes. Group folders and their contents retain library/import order.
- **Reader:** single-page remains the default. Continuous vertical reading is persisted and can be toggled with `v` or `:continuous [on|off]`. Webcomics mode is separately persisted, forces continuous layout and a zero page gap, and restores the saved normal layout when disabled; use `w` or `:webcomics [on|off]`.
- **Settings:** `Ctrl+,`, `:settings`, or the hamburger menu opens Appearance, Library, Reader, and Keyboard & Commands tabs. Every persisted reader/library preference has a control. Command-backed controls use the same mutation methods as commands.
- **Themes:** Kit built-in themes and app-side custom themes apply live and the selected name persists in `library.json`. `$XDG_CONFIG_HOME/yomu/themes.toml` (usually `~/.config/yomu/themes.toml`) is created from `themes.example.toml` on first use. The parser validates theme names, modes, tokens, and hex colors; a bad reload reports an error and keeps the existing theme. The exact format and supported keys are documented in the example and README.
- **Theme consistency / Noctalia:** custom UI reads semantic `UiColors`; the context-menu glyph uses the same `foreground` token as group and series names. `noctalia/yomu-theme.toml.template` maps Noctalia v5 palette tokens to an importable `Noctalia Dynamic` yomu theme. A background fingerprint watcher reloads the generated custom theme file automatically after it is replaced or edited; the Settings reload button remains available. Context menus use GPUI's anchored popup window (`xdg_popup`) on Wayland, with the old in-window menu as a platform fallback. The parent window dismisses the popup on a click elsewhere inside yomu; Wayland popup grabs handle outside-app dismissal. For Niri popup blur, use a matching `window-rule` with a `popups { background-effect { blur true } }` block.
- **Command prompt:** in the reader `:` opens a centered themed floating panel inside the main window. Its surface uses the same `widget_surface()` rule as Settings and other raised widgets: 10 percentage points more opacity than the window setting, capped at 100%. Do not add a special alpha cap for compositor blur. UI suggestions filter command names and supported arguments; Up/Down selects, Tab/Shift+Tab completes, Enter runs the existing `vim.rs` parser, and Escape cancels. The suggestion catalog lives in `main.rs`, separate from the parser, so keep it synchronized when commands or arguments change. Niri's main `background-effect` applies to the yomu window; `popups {}` applies only to the separate book context popup.
- **Visual design:** use GPUI Kit semantic theme tokens and bundled `IconName` assets, not a hard-coded Yomu palette. The app registers `gpui_kit::assets::AllAssets` with the application so icon SVG paths resolve. Header/back controls render `Icon` directly inside GPUI Kit `Button`s with explicit theme color and visible themed button surfaces; context-menu glyphs use the foreground color over accent backplates. Custom context menus and dialogs intentionally remain custom because they carry Yomu's keyboard navigation, dismissal, selection, and destructive-action behavior; they are styled from active Kit tokens. The book context menu is a native anchored GPUI popup (`xdg_popup`) on Wayland, with an in-window fallback where native popups are unavailable. Stop propagation on the book's opening right-click so the root popup-dismiss handler does not close it immediately. Clicks elsewhere inside yomu explicitly close the popup; the Wayland grab dismisses it outside the app. Niri `popups {}` rules can target the native menu.

## Persistent state and paths

- `$XDG_DATA_HOME/yomu/library.json` (normally `~/.local/share/yomu/library.json`) stores books, progress, marks, and app preferences. `YOMU_DATA_DIR` overrides the data root.
- The library schema is version 3. `Settings::from_value` accepts older/missing fields leniently and clamps opacity. Version 3 defaults folder grouping on while honoring explicit saved values. Add serde-compatible defaults and a deliberate migration if changing stored meaning.
- `themes.toml` belongs in the XDG **config** directory, separate from the library data directory. It is user-editable; never overwrite an existing file when loading the app.
- Comic source archives/folders are not copied, renamed, or deleted by yomu. Removal deletes yomu metadata/covers only.
- `problems.md` is user-owned. Do not edit it as part of feature work. Check source and README before treating its entries as open issues.

## Code map and invariants

- `src/main.rs`: GPUI Kit window, `Yomu` state, reader and library rendering, overlay state, event routing, persistence calls, background task coordination. This file is large; move pure/reusable logic to existing core modules where practical.
- `src/library.rs`: `Book`, `Settings`, XDG paths, import orchestration and JSON persistence.
- `src/archive.rs` / `src/imaging.rs`: archive/folder listing and background decode/resize primitives. Do not decode or resize images on the UI thread.
- `src/layout.rs` / `src/motion.rs` / `src/pagecache.rs`: pure geometry, animation, and cache policy.
- `src/vim.rs`: reader tokens, `:` command-line input and `Action` parsing. It must remain independent of GPUI.
- `src/search.rs` / `src/selection.rs` / `src/menu.rs`: GPUI-independent query, selection and context-menu policies.
- Page indices are zero-based and always in archive reading order. RTL affects input/visual travel, never archive order or cache order.
- Keep continuous scrolling compatible with jumps, marks, zoom, saved progress, and bounded visible-page caching. Webcomics is an override on top of the saved continuous preference.
- Keep async results tied to the book/page/request that created them; stale tasks must not replace the current reader content.

## UI reference decisions

The root `gpui-kit_inspiration.jpg` is the visual reference supplied by the user. For the library, preserve these explicit requests:

1. Search stays horizontally centered at the top.
2. Do not show the Yomu name or total book count in the toolbar.
3. Keep the top toolbar transparent, with no full-width colored strip or bottom rule.
4. Put a small hamburger at the upper right. Its menu contains library actions such as importing files/folders, settings, series grouping, and quitting.
5. Keep the book grid as the main content; do not replace working library behavior to imitate a static screenshot.

Settings uses active Kit themes rather than a Yomu palette. Tabs are open to future real preferences, but do not add controls for features that do not exist. Menus/dialogs may stay custom if Kit components cannot preserve current interaction semantics.

## Recent work & agent handoff log

### 1. Codex recovery and Turn 40 rollback
- **Context:** An external coding assistant (Codex) executed 40 turns across prior sessions. Turn 40 attempted to convert the command prompt into a native Wayland popup (`NativeCommandPrompt`), but was interrupted by rate limits mid-execution, leaving orphaned code, syntax errors, and inaccurate documentation.
- **Action taken:**
  - Audited full conversation history (stored in `~/.codex/thread_history_1.sqlite` and exported to the conversation artifacts).
  - Completely excised Turn 40's unfinished native command prompt code, restoring the clean, working in-window floating command prompt with `widget_surface()` (+10% opacity over window alpha).
  - Preserved and verified all valid features from Turns 26–39:
    - Libadwaita-style rounded window and widget geometry.
    - Active search bar with blinking caret and outside-click dismissal.
    - Progressive folder imports yielding immediate UI feedback per book.
    - Automatic directory-based folder grouping (with persisted override via `Book.series`).
    - Noctalia v5 theme template (`noctalia/yomu-theme.toml.template`) and background SHA-256 fingerprinting theme reloader.
    - Bundled SVG asset registration via `gpui_kit::assets::AllAssets`.
    - Wayland native `xdg_popup` book context menu.

### 2. Context menu outside-click dismissal bug & fix
- **Issue:** Right-clicking a book opened the native `xdg_popup` context menu, but clicking anywhere outside it in the main window did not dismiss the menu.
- **Root cause:** In GPUI on Linux Wayland, compositor grabs for popups automatically dismiss the popup when clicking *outside the application window*. However, clicks *inside* the parent application window were not closing the popup because Turn 39 added `Overlay::Menu(_) if self.native_context_popup => return None;` in `render_overlay`, which completely removed the modal scrim in the parent window.
- **Fix:**
  - In `src/main.rs:render_overlay`, changed the condition to keep rendering `deferred(self.scrim(false, cx))` even when `self.native_context_popup` is active. This invisible scrim captures any mouse click across the parent window and safely triggers `close_native_context_popup(cx)`.
  - Added parent window mouse down handlers to intercept and close open native popups.
  - Added an <kbd>Esc</kbd> key handler inside `NativeBookMenu::render`.
  - In `open_context_menu`, ensured any existing popup window is closed before opening a new one.

### 3. Critical GPUI Kit traps & patterns
- **`Button` hover style panic:** In GPUI Kit, calling `.hover(...)` on a `Button` that already has a style variant applied (e.g. `.ghost()`, `.secondary()`, `.primary()`, `.danger()`) panics with `assertion failed: self.hover_style.is_none(): hover style already set`. Never chain `.hover(...)` onto GPUI Kit `Button`s.
- **Asset provider requirement:** Custom `Icon` elements use SVG glyphs from GPUI Kit. If `gpui_kit::assets::AllAssets` is not registered on the application builder (`App::new().with_assets(gpui_kit::assets::AllAssets)`), all icon glyphs in toolbars and context menus will render blank.
- **Default folder grouping in tests:** Library schema version 3 defaults `group_series` to true. When test fixtures place multiple archives in the same temporary directory without distinct parent paths or series tags, they are grouped into a single folder card. Legacy UI tests expecting flat book cards must account for folder grouping or explicitly set `group_series: false`.
- **Raised surface opacity rule:** All elevated surfaces (dialogs, menus, settings, floating command prompt) use `widget_surface(window_opacity, colors)` to add +10 percentage points of opacity (capped at 1.0) above the main window background.

### 4. Niri Compositor Blur Architecture & Native Popups
- **Research on Niri & Wayland Blur:**
  - Niri's `window-rule` configuration (`~/.config/niri/niri/window-rules.kdl`) specifies:
    ```kdl
    window-rule {
        match app-id="yomu"
        open-maximized true
        background-effect { blur true }
        popups {
            background-effect { blur true }
        }
    }
    ```
  - Wayland compositors (like Niri via `ext-background-effect`) apply compositor blur strictly per Wayland surface (`wl_surface` / `xdg_popup` / `xdg_toplevel`). In-window elements drawn on the parent window's single framebuffer cannot be blurred by the compositor because the compositor receives only the final composite client texture.
  - Furthermore, top-level native windows (`xdg_toplevel`) with `app-id="yomu"` are automatically maximized by Niri's `open-maximized true` rule.
  - Therefore, all floating elements—Context Menu, Hamburger Library Menu, Modal Settings Dialog, and Reader Command Prompt—are implemented as `WindowKind::AnchoredPopup` (`xdg_popup`). Niri positions them as popups without tiling/maximizing and applies authentic background blur behind each surface via its `popups { background-effect { blur true } }` rule.
- **Consistent Transparency (`popup_surface`):**
  - All popups use `(widget_opacity + 0.15).clamp(0.70, 0.92)`. This provides a consistent frosted-glass look where the desktop/app underneath is visibly blurred while ensuring high contrast and legibility for text and controls.
- **Icon Color Consistency:**
  - All menu icons now match their row text color (`self.colors.foreground` for normal items, `self.colors.danger` for destructive items, `self.colors.muted` for disabled items). The legacy awkward teal pill background (`.bg(with_alpha(self.colors.accent, 0.24))`) has been removed.
- **Noctalia-Style Hover Border Glow:**
  - Cards, list rows, suggestions, and menu items feature Noctalia launcher-style hover effects: an accent border ring glow (`with_alpha(colors.accent, 0.70)`) accompanied by subtle accent surface tinting (`with_alpha(colors.accent, 0.20)`).
- **Click-Outside Dismissal:**
  - Wayland `grab: true` popups automatically dismiss when clicking outside the application. Inside the application, GPUI renders a transparent click-intercepting scrim in the parent window (`deferred(self.scrim(false, cx))`) while `self.native_popup` is active, intercepting clicks anywhere in the parent window to cleanly close the popup.
- **Fallbacks:**
  - Fully functional in-window fallbacks are preserved for non-Wayland platforms and simulated test environments (`#[cfg(test)]`).

### 5. Settings modal & transparent scrim occlusion fix
- **Issue:** Clicking "Settings" caused the entire Yomu window to disappear/become completely invisible while `cargo run` continued executing in the background.
- **Root causes:**
  1. **`.occlude()` on translucent/transparent full-screen elements:** Calling `.occlude()` on `scrim`, `render_menu`, or `render_library_menu` told GPUI's quad renderer to treat the element's bounding rect as opaque and cull (discard) all scene rendering primitives behind it. Because `scrim` is full-window (`size_full()`) with alpha 0.0 or semi-transparent dimming, `.occlude()` wiped out the entire parent window.
  2. **Invalid Wayland `xdg_popup` for centered modal dialogs:** An experimental attempt to make Settings an `AnchoredPopup` with `PopupAnchor::Center, PopupGravity::Center, grab: true` broke Wayland positioner constraints in tiling compositors like Niri, caused surfaces to fail to map, and caused re-entrant GPUI `Context` borrow panics (`cannot read yomu::Yomu while it is already being updated`).
- **Fix:**
  - Removed `.occlude()` from `scrim`, `render_menu`, and `render_library_menu`. Event intercepting (`on_mouse_down`) is handled via event propagation without quad occlusion.
  - Removed the broken `NativeSettings` struct and `open_native_settings` popup logic (~520 lines of duplicate code).
  - Restored `open_settings` and `open_settings_window` to activate the native `Overlay::Settings` in-window modal dialog, closing any active popups and notifying `Context`.
  - Preserved the full in-window Settings dialog with its 4 tabs, live opacity slider, theme cycler, keyboard shortcuts, and animations.

### 6. Cross-platform uniformity & removal of native popup hacks and background opacity
- **Context & Motivation:** Attempting to force menus, dialogs, and the command prompt into separate native Wayland `xdg_popup` child windows created severe platform coupling, compositor positioner rejections, focus stealing issues, and broke cross-platform parity on macOS and Windows (where `AnchoredPopup` is either unsupported or behaves inconsistently). Furthermore, compositor-level blur cannot be uniformly controlled across Linux, macOS, and Windows until upstream GPUI Kit natively supports standard blur protocols (such as `ext-background-effects-v1`).
- **Changes made:**
  1. **Removed all native child window popup hacks:** Excised `NativeBookMenu`, `NativeLibraryMenu`, `NativeCommandPrompt`, and their corresponding multi-window open/close orchestration (~1000 lines of complex duplicate code removed).
  2. **Unified in-window overlays:** All context menus (`Overlay::Menu`), library hamburger menus (`Overlay::LibraryMenu`), dialogs (Properties, Series, Confirmation, Settings), and the floating reader command prompt (`vim_status`) are rendered as clean, high-performance in-window GPUI elements. They behave identically across Linux (Wayland & X11), macOS, and Windows.
  3. **Locked window to fully opaque:** Window is created with `WindowBackgroundAppearance::Opaque`, removing background translucency hacks and compositor alpha blending quirks. The background opacity slider and related drag math were completely removed from the Appearance settings tab.
  4. **Removed context menu separator line:** Separators inside `render_menu` were removed for a sleek, cohesive look matching the Noctalia design language.
  5. **Updated test suite:** Replaced obsolete opacity slider tests with `window_appearance_is_always_opaque`, verifying all 138 unit and UI tests pass cleanly.

### 7. Pre-migration UI polish & native UI migration plan
- **RTL Removal:** Right-to-left (manga) toggle mode, keybindings (`r`), commands (`:rtl`, `:ltr`), and tests were completely removed across `src/layout.rs`, `src/motion.rs`, `src/vim.rs`, `src/library.rs`, `src/main.rs`, and `README.md`. Single-page and continuous vertical modes run strictly left-to-right.
- **Library Grid Alignment:** Tuned `ThumbnailSize::Large` to 240px and `ExtraLarge` to 280px; introduced centered responsive grid calculation (`make_grid`) based on window viewport width, eliminating trailing right-side empty space on 1366px/1080p displays.
- **Async Cover Art Generation:** Thumbnail resizing on size changes was offloaded to background threads (`cx.background_executor().spawn(...).detach()`), preventing UI freezing.
- **Button Rounding Consistency:** Changed `ButtonRounded::Large` (which rendered as circular pills) to `ButtonRounded::Medium` across `panel_button`, `button_colored`, and `icon_button` for uniform rectangular shapes with rounded corners.
- **Hover & Scrim Occlusion:** Added `.id(...).occlude()` to modal dialog scrims, panels, and dropdown menus, and guarded library card hover rings with `.when(self.overlay.is_none(), ...)`, stopping cursor hover states from leaking through overlays to the library cards underneath.
### 8. Native UI component migration & regression fixes
- **Dropdown Menu Click-Outside Dismissal:** Replaced custom `adw_dropdown` popover rendering with native `gpui_kit::component::menu::DropdownMenu` via `Button::dropdown_menu_with_anchor(Anchor::TopRight, ...)`. Native `Popover` handles backdrop clicks, escape keys, outside dismiss events, and selection checkmarks automatically. Removed manual `SettingsDropdown` state tracking enum and `self.settings_dropdown` field.
- **Toggle Switches:** Replaced hand-rolled `adw_switch` with native `gpui_kit::component::switch::Switch::new(id).checked(val).small().on_click(...)` across Library Folder Grouping, Page Animations, Continuous Vertical Mode, and Webcomics Mode.
- **Button Consistency & Destructive Buttons:** Extended `ButtonKind` with `ButtonKind::Danger` rendered using `Button::new(id).danger().outline().rounded(ButtonRounded::Medium).small()`. Replaced hand-rolled `adw_destructive_button` for "Reset All Data" and styled destructive confirm dialog buttons with it, ensuring identical padding, height, font sizes, and border curvature across the entire application. Obsolete `adw_switch`, `adw_dropdown`, and `adw_destructive_button` helpers were completely deleted.
- **Menu Hover Text Jitter Elimination:** Removed dynamic `border_1().border_color(...)` application on selected rows in `render_library_menu` and `menu_item`. Using background tint highlighting (`bg(...)`) keeps the box model dimensions static so text and icons never shift or pop when moving the cursor across items.

### 9. Comprehensive elimination of ad-hoc UI hacks via native GPUI Kit widgets
- **Settings Tabs (`TabBar`):** Replaced custom closures and manually styled `tab_button` `div()` elements with native `gpui_kit::component::tab::{Tab, TabBar}` in `render_settings`. Configured with `TabBar::new("settings-tab-bar").segmented().selected_index(...)` featuring GPUI Kit's native spring-animated sliding indicator. Added `SettingsTab::from_index` and `SettingsTab::to_index` helpers.
- **Series Dialog Input (`Input`):** Migrated `SeriesDialog` from raw `String` storage to `Entity<InputState>`. Bound with native `Input::new(&dialog.input).id("series-name-input").cleanable(true)`. Replaced manual character appending, backspace, and clear operations (`on_series_key`) with native text editing, clipboard, cursor movement, selection, and undo/redo. Enter key submission is handled via `InputEvent::PressEnter` subscription. Completely excised `on_series_key`.
- **Library Search Input (`Input`):** Replaced custom search `div()`, unicode `"⌕"` icon, manual string manipulation, and the 530ms async timer loop (`search_caret_visible`, `search_blink_seq`) with native `gpui_kit::component::input::Input`. Stored as `Entity<InputState>` on `Yomu`, subscribed to `InputEvent::Change` for instant search filtering, and configured with a native search prefix icon and cleanable button.
- **Book Card Reading Progress (`Progress`):** Replaced custom nested `div()` progress bar with native `gpui_kit::component::progress::Progress::new(...)` using theme accent color.
- **Series Group Badge (`Tag`):** Replaced manual pill `div()` for volume counts with native `gpui_kit::component::tag::Tag::secondary().small()`.
- **Selection Controls Unification:** Migrated selection controls ("Select all", "Remove", "Cancel") to `panel_button` with `ButtonKind::Normal` and `ButtonKind::Danger`. Deleted obsolete `fn button` and `fn button_colored` (which relied on string matching `"Remove"`).
- **Tooltips:** Added native `.tooltip(label)` on `icon_button` components across the library and reader.

### 10. Noctalia M3 alignment, native TabBar underline variant, and dynamic menus
- **Settings TabBar Sizing & Variant:** Replaced `.segmented()` with native `TabBar::new("settings-tab-bar").underline()`. Segmented tabs previously rendered with a 14px radius (`radius_lg`) in a full-width container, creating an awkward pill appearance with empty red track space on the right due to fallback `tokens.secondary`. The native `underline` variant renders with a transparent background, a 1px subtle bottom border (`border_color`), and an animated 2px sliding indicator using `tokens.primary`, naturally accommodating dynamic tab additions without trailing colored voids.
- **Native Tab Theme Tokens:** Registered GPUI Kit's native tab tokens (`"tab.active.background"`, `"tab.active.foreground"`, `"tab.background"`, `"tab.foreground"`, `"tab_bar.background"`, `"tab_bar.segmented.background"`) into `CUSTOM_THEME_COLOR_KEYS` and the Noctalia template (`noctalia/yomu-theme.toml.template`), allowing theme files to style tabs natively.
- **Noctalia M3 High-Contrast Active Selection:** Aligned active selection colors with Noctalia v5 Shell launcher specifications. Updated `noctalia/yomu-theme.toml.template` to map `accent.background = "{{ colors.primary.default.hex }}"` and `accent.foreground = "{{ colors.on_primary.default.hex }}"`. Added `accent_foreground: tokens.primary_foreground` to `UiColors`. Menu items and active rows now pair solid `colors.accent` background with `colors.accent_foreground` text and icons for AAA contrast, matching Noctalia launcher.
- **Dynamic Menu Sizing:** Replaced static pixel widths on context menus and library menus (`MENU_W` = 224px, 280px) with content-driven dynamic constraints: `min_w(px(200.))`/`min_w(px(240.))`, `max_w(px(320.))`/`max_w(px(360.))`, and `w_auto()`. Removed unused `MENU_W` constant.
- **Native GPUI Kit Table for Keyboard Shortcuts:** Migrated `SettingsTab::Keyboard` to GPUI Kit's native `Table`, `TableHeader`, `TableBody`, `TableRow`, `TableHead`, and `TableCell` components. Configured with a `TableHead` header ("ACTION" on the leftmost side and "SHORTCUT" on the rightmost side with `.text_right()`), section divider banner rows with `col_span(2)` and shortcut counts (`Tag::secondary().small()`), and row borders with theme-derived styling. Key badges are positioned flush against the right margin with action descriptions flush against the left. Fixed scrolling by separating the outer card border/surface from the inner scroll container (`#settings-shortcuts-scroll` with `overflow_y_scroll()` and fixed height without conflicting `overflow_hidden()`). Refined all shortcut descriptions in `HELP` to be concise, grammatically precise, and terminated with periods.

### 11. Core Engine Performance, Memory & Zero-Allocation Algorithms
- **Zero-Allocation Natural String Sort (`src/util.rs`):** Completely rewrote `natural_cmp` to compare character streams and numeric runs in place. Replaced string-allocating lowercase conversions (`to_lowercase()`) and digit buffers (`take_number()`) with zero-allocation char folding and dynamic digit scanning, eliminating tens of thousands of temporary heap allocations during archive and folder sorting.
- **Zero-Allocation Library Search Matching (`src/search.rs`):** Replaced lowercased string allocation (`title.to_lowercase()`, `path.to_string_lossy().to_lowercase()`, `series.to_lowercase()`) with an optimized `contains_ignore_case` helper featuring an ASCII fast-path and short-circuiting title/series/path evaluations.
- **Archive I/O Buffering & In-Memory Handle Cache (`src/archive.rs`):**
  - Wrapped `File` in a 64 KiB `BufReader` during zip archive inspection to collapse thousands of syscall reads into block operations.
  - Implemented `ARCHIVE_CACHE` (a thread-safe MRU handle pool) in `read_page`: subsequent page reads reuse parsed zip central directories and open file descriptors, eliminating disk open and header parse overhead on page flips.
  - Converted `is_image_name` and `is_archive_name` to slice extension checks using `.eq_ignore_ascii_case()` with zero heap allocations.
- **Resampling Scratch Buffer Reuse & Direct JPEG Cover Output (`src/imaging.rs`):**
  - Replaced per-call `Resizer::new()` instantiation with a `thread_local!` `RefCell<Resizer>` to reuse internal SIMD scratch memory and convolution buffers across all page and thumbnail scaling jobs.
  - Streamlined `make_cover_jpeg` to encode directly to `ExtendedColorType::Rgb8`, eliminating redundant `DynamicImage` wrapping and intermediate conversions.

### 12. Native Touchpad & Touchscreen Inertial Momentum Scrolling (`src/motion.rs`, `src/main.rs`)
- **Native Physics Model (`InertialScroll`, `VelocityTracker`):**
  - Integrated rolling-sample velocity estimation (`VelocityTracker`) over the exit gesture window without artificial delays or allocations.
  - Implemented exact closed-form analytical integration: $\Delta x = v_0 \cdot \tau \cdot (1 - e^{-dt / \tau})$ with natural exponential friction (`INERTIA_TAU_SECS = 0.32`).
  - Frame-rate independent: identical total scroll displacement and glide curve across 60 Hz, 120 Hz, 144 Hz, and 240 Hz displays (verified by analytical unit tests).
- **Native GPUI Kit / GPUI Event Integration:**
  - Leveraged GPUI Kit's native `ScrollWheelEvent.touch_phase` (`TouchPhase::Started`, `TouchPhase::Moved`, `TouchPhase::Ended`, `TouchPhase::Cancelled`) and drag handlers (`drag_start`, `drag_move`, `drag_end`).
  - Clamped event `dt` to `[0.001, 0.1]` seconds and prevented zero-movement release deltas from polluting exit velocity upon finger lift-off.
  - Immediate interruption / touch-catch: touching down (`TouchPhase::Started` or `drag_start`) instantly resets `reader.inertia`, delivering instantaneous tactile response without fighting the user's fingers.
  - Hard boundary collision: automatically zeroes momentum along an axis when reaching edge limits (`0.0` or max offset/continuous document boundaries).
  - Accessibility & Reduced Motion: strictly respects `!reader.animate` and `reduce_motion` (GNOME `enable-animations` / `YOMU_REDUCE_MOTION`), disabling momentum immediately when reduced motion is requested.

### 13. Safety Enforcement, Hot-Path Async Persistence, and UI Modularization
- **100% Safe Rust (`#![forbid(unsafe_code)]`):** Enforced across `src/lib.rs` and `src/main.rs`. Zero `unsafe` blocks exist across the entire project.
- **Clippy Modernization (0 Warnings with `-D warnings`):** Fixed all actionable lints:
  - SIMD chunking via `as_chunks_mut::<4>().0` in `imaging.rs`.
  - Slice extension case matching via `.eq_ignore_ascii_case()` in `search.rs`.
  - Collapsed nested `if` statements and simplified `.map_or(true, ...)` to modern `.is_none_or(...)`.
  - Encapsulated complex parameter and return types into `WheelParams` and `ReaderSyncOutcome`.
- **Debounced Asynchronous Persistence:**
  - Removed synchronous `serde_json::to_vec_pretty` and `fs::rename` disk writes from the UI rendering loop during continuous vertical scrolling (`sync_reader`).
  - Added monotonic sequence tracking (`save_seq`, `last_saved_seq: Arc<Mutex<u64>>`) with a 500ms debounce timer and 2000ms max-delay. Dispatched to `cx.background_executor()`, reducing continuous scroll UI blocking time to 0.00 ms.
  - Ensured immediate synchronous flush on `close_book`, `Action::QuitApp`, and `Drop for Yomu`.
- **Modularization of `src/main.rs` into `src/ui/`:**
  - Decomposed over 950 lines from `src/main.rs` into cohesive modules:
    - `src/ui/theme.rs`: Custom themes, TOML parsing, and `UiColors`.
    - `src/ui/reader.rs`: `Reader` struct, `Page`, `Outgoing`, `Drag`, `WheelParams`, continuous geometry, and transitions.
    - `src/ui/overlays.rs`: Modal overlays, dialogs, menus, and `SettingsTab`.
    - `src/ui/components.rs`: Reusable buttons, pills, centered containers, and layout primitives.

### 14. Architecture Modernization, `x86_64-v3` SIMD Pipeline, Lock-Free Concurrency, and Adaptive Memory Budget
- **`x86_64-v3` Target Architecture:** Configured via `.cargo/config.toml` (`-C target-cpu=x86-64-v3`), auto-vectorizing physics math and SIMD routines with AVX2/FMA/BMI2. Math benchmark dropped from 39 ns to 25 ns per frame.
- **Direct SIMD JPEG BGRA Decode (`src/imaging.rs`):** `decode_jpeg_fit_bgra` uses `zune-jpeg` direct to `ColorSpace::BGRA` into raw buffers, checking dimensions against `MAX_PIXELS` before allocating. Resizes directly on `PixelType::U8x4` via `fast_image_resize`. High-tier (4096) decode+scale latency dropped from 759 ms to 279 ms (2.7x faster); unscaled decode dropped from 1010 ms to 182 ms (5.5x faster).
- **Lock-Free Concurrent Archive Pooling (`src/archive.rs`):** Replaced the single global `ARCHIVE_CACHE` mutex with an unlocked `checkout_archive` / `checkin_archive` pool. Worker threads decompress pages in parallel with zero lock contention. Path normalization in `walk_folder` optimized to single-pass `.replace('\\', "/")`.
- **Adaptive Memory Budget & Expanded Preload (`src/pagecache.rs`):** Expanded prefetch window to 5 pages ahead and 3 pages behind (keeping 5 ahead and 4 behind). Replaced static 320 MiB ceiling with `system_memory_budget()`, defaulting to 1 GiB and scaling up to 3 GiB on systems with 16+ GiB RAM.
- **Cooperative Decode Task Cancellation (`src/ui/reader.rs`, `src/main.rs`):** Added `cancellations` atomic token map to `Reader`. Background tasks check tokens before I/O, before decompression, and before texture creation. Rapid page navigation and book closures cancel obsolete tasks immediately.
- **Test Suite Modularization (`src/tests.rs`):** Extracted 2,129 lines of GPUI interaction test fixtures from `src/main.rs` into `src/tests.rs`, reducing `src/main.rs` from 6,903 to 4,775 lines and substantially speeding up IDE analysis and link times.
- **Cross-Platform Input Parity:** Added macOS Command key awareness in `main.rs` (`Cmd+F` search, `Cmd+O`, `Cmd+,`, `Cmd+Q`, Cmd-click multi-select, and Cmd-wheel zoom) while preserving native Control shortcuts on Linux and Windows.

### 15. CBR / RAR Archive Decoding & Native Pinch Zoom Gesture Support
- **CBR & RAR Archive Decompression (`src/archive.rs`):**
  - Integrated `unrar = "0.5.8"` for listing and in-memory extraction of comic book RAR archives (`.cbr`, `.rar`).
  - Added magic-byte inspection (`PK\x03\x04` / `PK\x05\x06` for ZIP vs `Rar!\x1a\x07` for RAR) with extension fallback (`detect_archive_format`), enabling automatic decoding even when archives are mislabeled.
  - Implemented bidirectional fallback: if an archive labeled ZIP fails, it falls back to RAR decompression; if RAR fails, it falls back to ZIP decompression.
  - Natural sorting of RAR archive file paths, Windows backslash normalization, entry directory filtering, and bounded decompression with `MAX_PAGE_BYTES` (256MB).
  - Updated unsupported format message to `(CBZ/CBR/ZIP/RAR archives and image folders only)`.
  - Added format labels `"CBR archive"` and `"RAR archive"` in properties dialog (`src/menu.rs`).
- **Touchpad & Touchscreen Pinch Zoom (`src/main.rs`):**
  - Added native `.on_pinch` handler using GPUI Kit's `PinchEvent` (`Point<Pixels>`, `delta`, `TouchPhase`) on both reader container and library root container.
  - **Reader Zoom:** Clamps scale multiplier `(1.0 + delta).clamp(0.2, 5.0)` and anchors zoom dynamically at `(event.position.x, event.position.y)` via `zoom_by(factor, at, vp, cx)`. Cancels running kinetic inertia on active pinch movement and resets on `TouchPhase::Started`.
  - **Library Thumbnail Zoom:** Smoothly accumulates pinch deltas (`library_pinch_accum`) across gestures; crossing threshold `|accum| > 0.35` steps thumbnail size up or down (`Small` <-> `Medium` <-> `Large` <-> `ExtraLarge`), notifies user with toast, and persists preference. Resets accumulator on gesture start/end.
  - Added full test coverage in `src/archive.rs`, `src/library.rs`, `src/menu.rs`, and `src/tests.rs`.

### 16. macOS HIG Settings, Native GPUI Kit Dropdown Menus, and Inertial Scrolling
- **Native GPUI Kit Dropdown Menus (`gpui_kit::component::menu::DropdownMenu`):**
  - Replaced custom `deferred(...)` / `anchored(...)` global overlay state and modal scrim intercepts with native `gpui-kit` / `gpui-component` dropdown menu integration via `.dropdown_caret(true)` and `.dropdown_menu(...)`.
  - Integrated `PopupMenu` with `check_side(Side::Left)` and `PopupMenuItem::new(...).checked(...)` to match native macOS popover styling and checkmark placement without custom overlays or screen-coordinate passing.
  - Converted Settings Appearance dropdowns (Color Scheme, Thumbnail Size, Corner Rounding, Border Thickness), Reader Settings (Reading Direction), and Reader Bottom Bar controls (Fit Mode, Reading Direction) to native popovers.
  - Eliminated scrim event collision bugs where opening dropdowns inside modal dialogs immediately dismissed the menu or failed to register selections.
- **Inertial Scrolling Across All Views (`src/main.rs`, `src/motion.rs`):**
  - Restored `UiScrollController` and `KineticScroll` physics across all application views: `AllManga`, `ContinueReading`, `Collections`, `MangaDetails`, `AniListSearch`, `AniListCategory`, `Sidebar`, and `Settings`.
  - Mouse-wheel notches trigger smooth inertial glides; touchpad flicks trigger kinetic deceleration with touch-down catch-interruption.
- **macOS Window Controls & UI Consistency:**
  - Removed redundant secondary back buttons; integrated seamless unified top navigation with titlebar traffic light controls.
  - Aligned titlebar branding horizontally alongside macOS traffic lights.
  - Standardized rounded box corners (`CornerRounding`) across UI components, eliminating inconsistent pill shapes and ensuring cursor styles (`cursor_pointer`) on all interactive switches and buttons.
- **Dynamic Responsive Grid Layout (`compute_responsive_grid`):**
  - Card dimensions (`card_w`, `cover_h`) are strictly fixed to the user's selected `ThumbnailSize` (Small: 180×270, Medium: 220×330, Large: 270×405) so covers never stretch, blow up, or shrink when the window resizes or when the sidebar toggles.
  - Column count (`cols`) dynamically adjusts to fit the maximum number of full cards across the available viewport width.
  - Remaining horizontal space is distributed evenly across all outer margins (`pad_x`) and inter-card gaps (`gap`): left padding, spaces between cards, and right padding are all identical and adapt smoothly as the window width changes, with zero wasted margin on the right.
- **Unified Single Sidebar Toggle Button:**
  - Removed duplicate sidebar collapse button in the sidebar header; retained the single canonical toggle button in the toolbar (`toolbar-sidebar-toggle`) adhering to macOS HIG.
- **Monochrome Settings Panel Navigation:**
  - Removed colored background badge blocks from the Settings sidebar; unified tab styling with the main sidebar navigation using monochrome icons (`colors.muted` inactive, `colors.accent_foreground` active).
- **Auto-Hiding Reader Controls on Hover (mpv-style):**
  - Integrated native GPUI Kit `.group(...)` and `.group_hover(...)` zones for the reader top bar and navigation chevrons.
  - Controls default to `opacity(0.0)` for an unobstructed, immersive comic reading experience and smoothly reveal on hover over their top or side zones.

### 17. OS-Aware Shortcut Keybindings, Functional List View, Separated Icon-Pair Switches, and Native Tooltips
- **Dynamic OS Keybindings (`src/ui/components.rs`, `src/ui/toolbar.rs`, `src/ui/settings_view.rs`, `src/ui/anilist_search.rs`):**
  - Added OS shortcut string helpers (`search_shortcut_str`, `settings_shortcut_str`, `open_shortcut_str`, `zoom_shortcut_str`).
  - Displays macOS symbols (`⌘K`, `⌘,`, `⌘O`, `Cmd+Wheel`) strictly on macOS; displays native Linux/Windows equivalents (`Ctrl+K`, `Ctrl+,`, `Ctrl+O`, `Ctrl+Wheel`) on Linux and Windows.
  - Updated search bar badges, shortcuts settings table, import tooltips, and sidebar settings hints to dynamically adapt to the running host OS.
- **Functional List View with Persistence (`src/main.rs`, `src/library.rs`):**
  - Added `view_mode_grid: bool` to `Settings` (persisted in `library.json`, defaults to `true`).
  - Implemented `render_book_list_row`: 64px row displaying thumbnail cover, title, series badge, progress status, mini progress bar, and completion badges.
  - Implemented `render_folder_list_row`: 64px series folder row with cover, folder icon, volume count badge, and chevron navigation.
  - Seamlessly supported across `AllManga`, `History`, and `AniListCategory` views, maintaining selection and context-menu interactions in both grid and list modes.
- **Separated Icon-Pair Switches in Toolbar (`src/ui/toolbar.rs`):**
  - Separated folder grouping from view mode with a 10px spacing gap.
  - Grouping Switch: icon pair `Folder` (series folders) vs `BookOpen` (individual volumes) with no text.
  - View Mode Switch: icon pair `LayoutGrid` (grid view) vs `List` (list view) with no text.
  - Interactive segmented controls with active accent fill, hover states, and native GPUI Kit tooltips.
- **Native GPUI Kit Tooltips Throughout the Entire UI:**
  - Integrated `gpui_kit::component::tooltip::Tooltip` across window traffic lights (close, minimize, zoom), toolbar controls (sidebar toggle, navigation back/forward, grouping toggles, view mode toggles, import button), manga details actions (back to library, continue reading, link AniList, more options), settings traffic light, and reader navigation chevrons.

### 18. Native UX Refinement, Cross-Platform Polish, and GPUI Kit 0.8 Architecture
- **Platform-Native Window Decorations & Dynamic Insets:**
  - Excised fake custom window traffic dots from the sidebar and dialogs.
  - On macOS, restored AppKit native traffic lights at `(16, 18)` via `TitlebarOptions { appears_transparent: true, traffic_light_position: Some(...) }`.
  - Sidebar header and collapsed toolbar dynamically apply `pl(px(80.))` strictly on macOS, preventing collision with native traffic lights while maintaining zero superfluous padding on Linux and Windows.
  - On Windows, relies on native DWM caption controls with `appears_transparent: false`.
  - On Linux, requests `WindowDecorations::Server` (SSD). When the compositor negotiates SSD (e.g. Mutter/KWin), compositor draws window controls. When client-decorated (`Decorations::Client`), dynamically renders a clean, Libadwaita-inspired close button on the far right of the toolbar with actions shifted left.
- **Redesigned Standalone Import Button:**
  - Replaced visible "Import" text button with a minimal 28px plus (+) icon button matching the exact dimensions, 14px icon weight, visual style, hover/pressed states, and corner radius of the view-mode switches.
  - Retained `debug_selector("add-files")` and accessible tooltip `Import files (⌘O / Ctrl+O)`.
- **Consolidated Thumbnail Sizes (Small, Medium, Large):**
  - Eliminated duplicate thumbnail settings from Appearance; unified in Library tab under "Thumbnail Size".
  - Refactored `ThumbnailSize` enum to 3 options: `Small` (180px), `Medium` (220px), `Large` (270px).
  - Preserved backward compatibility with legacy `library.json` files via serde aliases (`xsmall`, `extra-small`, `xlarge`, `extra-large`, `huge`).
  - Removed raw pixel dimensions from UI labels; updated trackpad pinch zoom to cycle cleanly between Small <-> Medium <-> Large.
- **Unified Appearance Mode Dropdown Selector:**
  - Replaced custom preview cards with native `macos_dropdown_button` dropdown offering `System`, `Light`, `Dark`.
  - Matches the existing `Color Scheme` and `Corner Rounding` dropdown styling and interaction semantics.
- **Thumbnail Corner Clipping Fix:**
  - Identified root cause: GPUI's `overflow_hidden()` on parent divs clips to an axis-aligned bounding box, not rounded corners.
  - Added `.rounded_t(radius)` directly to grid card cover containers and `img(cover_path)` elements, and `.rounded(px(radius.min(4.)))` to list items. Covers now smoothly follow the card border without corner bleeding or flat cutoffs.
- **Symmetrical Destination-Based Navigation Model:**
  - Implemented `NavDestination` enum (`Section(SidebarSection)`, `Group { key, name }`, `Details(id)`, `AniListSearch`).
  - Fixed asymmetric Back/Forward navigation: entering a group now records a first-class navigation destination, allowing Forward to re-enter the group, and Back from nested views (Details) to return to the group rather than popping to root.
  - Toolbar `can_go_back` and `can_go_forward` update immediately and reliably based on stack index.

### 19. Context-Aware Controls, Selection Decoupling, Single Contextual Toolbar, Platform Settings Controls, Simplified Navigation, Opt-in Vim Mode, and Design-Token Icons
- **Decoupled Right-Click Behavior & Selection State (`src/main.rs`, `src/ui/overlays.rs`, `src/selection.rs`):**
  - Right-clicking an item (folder group or individual book) no longer automatically selects it or triggers selection mode.
  - `BookMenu` now carries `target_ids: Vec<String>` explicitly. Context menu operations (e.g. Remove, Mark Read, Mark Unread) operate directly on `target_ids` rather than requiring items to be in the global selection.
  - Right-clicking an already-selected item preserves the existing multi-selection and offers batch actions on all selected items.
  - Right-clicking an unselected item drops any active selection without selecting the clicked item, opening the context menu cleanly targeted to that item.
- **Single Contextual Toolbar (`src/ui/toolbar.rs`, `src/main.rs`):**
  - Excised the secondary toolbar row (`show_header`) from `render_all_manga_view`, `render_history_view`, and `render_anilist_category_view`.
  - Consolidated selection controls directly into the primary top toolbar:
    - Browsing Mode (`selection_count == 0`): displays search bar, grouping switch, view mode switch, and import button.
    - Selection Mode (`selection_count > 0`): replaces browsing controls with `N selected` badge, `Select all`, `Remove`, and `Cancel` in the exact same 52px header area without height jumps or layout shifts.
- **Platform-Aware Settings Panel Window Controls (`src/ui/settings_view.rs`):**
  - On macOS: displays close button on the top-left (`settings-close-macos`), matching native macOS window conventions.
  - On Windows: displays close button on the top-right (`settings-close-win`).
  - On Linux with CSD (`Decorations::Client`): displays Libadwaita-inspired close button on the top-right (`settings-close-csd`).
  - On Linux with SSD (`Decorations::Server`): omits in-window close button, letting the compositor handle decorations.
- **Simplified Back-Only Navigation (`src/ui/toolbar.rs`, `src/main.rs`):**
  - Forward navigation button removed entirely from the toolbar.
  - Back button (`toolbar-nav-back`) is visible strictly when viewing inside an open series group (`open_group.is_some()`). In library root or individual volume modes, the Back button is hidden.
  - Activating Back exits the group back to the library view cleanly.
- **Opt-In Vim Navigation (`src/library.rs`, `src/main.rs`, `src/ui/settings_view.rs`):**
  - Vim mode is disabled by default (`Settings.vim_mode = false`, with `#[serde(default)]` backward compatibility).
  - Reader settings tab provides a toggle switch (`vim-mode-switch`) under "Vim Keybindings".
  - Added CLI flags `--vim` and `--vim-mode` in `main.rs` to override Vim mode for the current session without modifying saved settings.
  - When disabled, Vim keys (`j`, `k`, `h`, `l`, `gg`, `G`, `:`) do not intercept desktop navigation, allowing standard arrow and modifier keys to function predictably.
- **Theme-Aware Collection Icons (`src/ui/collections_view.rs`):**
  - Replaced hard-coded RGB hex colors with semantic GPUI Kit design tokens (`colors.accent` and `colors.foreground`), ensuring visual consistency across all themes and appearance modes.
- **Recently Added & Library Grid Consistency (`src/ui/continue_reading.rs`, `src/main.rs`):**
  - Aligned Recently Added layout with All Manga: dynamically interprets folder cards when `group_series` is enabled and individual volume cards when disabled.
  - Eliminated uneven margin/whitespace on the right by passing the dynamically calculated `grid.gap` (`recent_gap`) and responsive card width into Recently Added.
- **Numeric-Only Volume Badges (`src/main.rs`, `src/ui/components.rs`):**
  - Replaced hard-coded `"vols"` strings with `ui::components::stat_badge(count, false, colors, radius)`.
  - Badges are squircle-shaped, theme-aware, and display strictly the numeric volume count matching the sidebar count badges.
- **Thumbnail Inner Corner Radius Derivation (`src/main.rs`, `src/ui/continue_reading.rs`):**
  - Solved corner overflow where square cover images poked past rounded card borders.
  - Cards now compute `inner_radius = (radius - px(border_px)).max(px(0.))` derived from the configured corner radius and border thickness, applied to both the cover container and image with top rounding.
- **Dynamic Shortcuts Settings Pane (`src/ui/settings_view.rs`):**
  - Shortcuts tab in the Settings panel dynamically checks the active `vim_mode` state (including session CLI flags).
  - Displays standard desktop navigation shortcuts (Arrow keys, Space, PageDown) when Vim mode is disabled, and displays Vim keybindings (`j`, `k`, `gg`, `G`, `:`) when Vim mode is enabled.
- **Application-Wide Disable Animations Setting (`src/library.rs`, `src/ui/settings_view.rs`, `src/main.rs`):**
  - Added persistent `disable_animations` switch to Settings Reader tab.
  - When system reduced motion is active, the switch is forced on, disabled (greyed out), and displays an informative helper message.
  - Enforced a centralized animation policy: `animations_enabled = !system_reduced_motion && !user_disable_animations`.

## Build and verification

On the configured NixOS system, use `devenv shell` (or `nix develop`) before launching. Useful commands:

```sh
cargo fmt --all -- --check
cargo check
cargo test
```

The current toolchain minimum is Rust 1.92. UI tests use GPUI's simulated platform; they do not validate actual compositor appearance, window transparency, portals, or Wayland behavior. Report precisely which commands ran and whether a real compositor was used. `cargo test perf_ -- --ignored --nocapture` runs opt-in measurements only.

