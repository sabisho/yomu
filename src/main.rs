//! yomu: a minimal, GPU-accelerated, Wayland-native manga reader.

#![forbid(unsafe_code)]

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use anyhow::Result;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Side as MenuSide, Sizable, Theme as KitTheme, ThemeConfig};
#[allow(unused_imports)]
use gpui_kit::popup::{PopupAnchor, PopupConstraintAdjustment, PopupGravity, PopupOptions};
#[allow(unused_imports)]
use gpui_kit::{
    Anchor, Animation, AnimationExt, AnyElement, AnyWindowHandle, App, Bounds, ClickEvent, Context,
    Decorations, ElementId, Entity, ExternalPaths, FocusHandle, Focusable, FontWeight, KeyBinding,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit,
    PathPromptOptions, PinchEvent, Pixels, Point, RenderImage, Rgba, ScrollDelta, ScrollHandle,
    ScrollWheelEvent, SharedString, Styled, Subscription, TitlebarOptions, TouchPhase, Window,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowKind, WindowOptions,
    actions, anchored, application, canvas, deferred, div, ease_out_quint, img, point, prelude::*,
    px, relative, size,
};
use image::Frame;
use yomu::{
    archive, imaging,
    layout::{self, Arrive, FitMode, Viewport},
    library::{
        self, Book, BorderSize, CornerRounding, DataDirs, Library, Settings, ThemeModePreference,
        ThumbnailSize,
    },
    menu::{self, BookAction, Entry},
    motion, pagecache,
    selection::Selection,
    util::{format_size, now_secs},
    vim::{self, Action, Amount, Dir, Vim},
};

actions!(
    yomu,
    [
        ToggleFullscreen,
        ImportFiles,
        ImportFolder,
        OpenSettings,
        Quit
    ]
);

mod ui;

use ui::components::{centered, page_layer, pill, with_alpha};
use ui::library_view::{
    BookCardProps, LibraryGroup, directory_groups, render_book_card, render_book_list_row,
    render_folder_card, render_folder_list_row,
};
use ui::overlays::{
    BookMenu, ConfirmAction, ConfirmDialog, LibraryMenu, Overlay, PropertiesDialog, SeriesDialog,
    SettingsTab, command_suggestions,
};
use ui::reader::{Drag, Page, Reader, ReaderSyncOutcome, WheelParams, viewport_of};
#[allow(unused_imports)]
use ui::theme::custom_theme_fingerprint;
use ui::theme::{
    BUILT_IN_THEMES, DEFAULT_THEME_NAME, UiColors, built_in_theme, built_in_theme_names,
    custom_theme_path, ensure_custom_theme_file, load_custom_themes, parse_custom_themes,
};

/// How far `j`/`k`/`h`/`l` scroll, in logical pixels.
const LINE_STEP: f32 = 64.0;
/// A wheel notch scrolls this many pixels (and is one zoom step with Ctrl).
const WHEEL_LINE: f32 = 96.0;
/// Slack when deciding whether the page is already scrolled to its edge.
const EDGE_EPS: f32 = 1.0;
/// Mouse travel before a press counts as a drag instead of a click.
const DRAG_THRESHOLD: f32 = 4.0;
const HUD_SECS: u64 = 2;
const AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(500);
const AUTOSAVE_MAX_DELAY: Duration = Duration::from_millis(2000);

/// Clicks this soon after a menu or dialog closes are ignored, so double-clicking a menu item
/// cannot also click the book underneath it.
#[allow(dead_code)]
const CLICK_GUARD: Duration = Duration::from_millis(300);
/// Clicks that caught / stopped an active inertial scroll glide are suppressed from opening cards.
#[allow(dead_code)]
const SCROLL_CATCH_GUARD: Duration = Duration::from_millis(150);

#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

enum ImportEvent {
    Book(Book),
    Finished(library::ImportReport),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavDestination {
    Section(ui::sidebar::SidebarSection),
    Group { key: String, name: String },
    Details(String),
}

struct Yomu {
    focus_handle: FocusHandle,
    dirs: Arc<DataDirs>,
    library: Library,
    colors: UiColors,
    custom_themes: HashMap<String, Rc<ThemeConfig>>,
    #[allow(dead_code)]
    custom_theme_path: PathBuf,
    #[allow(dead_code)]
    theme_status: Option<String>,
    reader: Option<Reader>,
    /// Images that are no longer shown. Their GPU textures are freed on the next render,
    /// because that is where we have access to the `Window`.
    garbage: Vec<Arc<RenderImage>>,
    toast: Option<SharedString>,
    toast_seq: u64,
    hud: Option<SharedString>,
    hud_seq: u64,
    importing: bool,
    importing_title: Option<String>,
    imported_count: usize,
    system_dark: bool,
    /// The desktop asks for reduced motion (see `motion::detect_reduced_motion`).
    reduce_motion: bool,
    /// When the previous animated frame was drawn; `None` while nothing is animating.
    last_frame: Option<Instant>,
    /// Smoothed frame interval for display vsync scheduling jitter mitigation.
    dt_smoothed: Option<f32>,
    /// Tests drive the animation clock by hand instead of using real time.
    #[cfg(test)]
    frame_dt: Option<f32>,
    overlay: Option<Overlay>,
    settings_tab: SettingsTab,
    command_selection: usize,
    library_vim: Vim,
    theme_before_preview: Option<String>,
    overlay_closed_at: Option<Instant>,
    /// Books picked for bulk actions in the library.
    selection: Selection,
    /// Transient library filter; it is not part of persisted reader settings.
    search_query: String,
    search_active: bool,
    search_input: Option<Entity<InputState>>,
    open_group: Option<String>,
    /// Which background appearance the window currently has (`None` before the first frame).
    applied_opaque: Option<bool>,
    library_scroll: ScrollHandle,
    group_scroll: ScrollHandle,
    continue_reading_scroll: ScrollHandle,
    collections_scroll: ScrollHandle,
    details_scroll: ScrollHandle,
    settings_scroll: ScrollHandle,
    sidebar_scroll: ScrollHandle,
    library_scroll_controller: yomu::scroll::PlatformScrollController,
    #[allow(dead_code)]
    library_pinch_accum: f32,
    cursor_pos: Option<(f32, f32)>,
    save_seq: u64,
    last_saved_seq: Arc<Mutex<u64>>,
    save_pending: bool,
    save_pending_since: Option<Instant>,
    // Two-pane redesign fields:
    active_section: ui::sidebar::SidebarSection,
    active_view: ui::sidebar::MainView,
    selected_book_id: Option<String>,
    details_tab_index: usize,
    details_synopsis_expanded: bool,
    nav_history: Vec<NavDestination>,
    nav_history_idx: usize,
    view_mode_grid: bool,
    selected_collection_id: Option<String>,
    sidebar_collapsed: bool,
    pub vim_cli_override: bool,
}

impl Yomu {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dirs = DataDirs::new().expect("cannot create the yomu data directory");
        Self::with_dirs(dirs, window, cx)
    }

    fn with_dirs(dirs: DataDirs, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dirs = Arc::new(dirs);
        let (library, toast) = match Library::load(&dirs) {
            Ok(library) => (library, None),
            Err(err) => (
                Library::default(),
                Some(SharedString::from(format!("{err:#}"))),
            ),
        };
        let custom_theme_path = custom_theme_path(&dirs).unwrap_or_else(|error| {
            eprintln!("yomu: cannot determine custom theme path: {error:#}");
            dirs.library_file()
                .parent()
                .unwrap_or(Path::new("."))
                .join("themes.toml")
        });
        let mut theme_status = None;
        let mut custom_themes = HashMap::new();
        match ensure_custom_theme_file(&custom_theme_path).and_then(|()| {
            let contents = fs::read_to_string(&custom_theme_path)?;
            let built_in_names = built_in_theme_names(cx);
            parse_custom_themes(&contents, &built_in_names)
        }) {
            Ok(themes) => custom_themes = themes,
            Err(error) => theme_status = Some(format!("Custom theme file: {error:#}")),
        }
        let system_dark = true;
        let scheme = library.settings().color_scheme.clone();
        let mode = library.settings().theme_mode;
        let resolved = ui::theme::resolve_theme_name(&scheme, mode, system_dark, &custom_themes);
        let selected_theme = resolved.as_str();
        let selected_config = custom_themes
            .get(selected_theme)
            .cloned()
            .or_else(|| built_in_theme(selected_theme));
        if let Some(config) = selected_config {
            KitTheme::update(cx, |theme| theme.apply_config(&config));
        } else if selected_theme != DEFAULT_THEME_NAME
            && let Some(config) = built_in_theme(DEFAULT_THEME_NAME)
        {
            KitTheme::update(cx, |theme| theme.apply_config(&config));
            theme_status.get_or_insert_with(|| {
                format!("Theme “{selected_theme}” was unavailable; using {DEFAULT_THEME_NAME}")
            });
        }
        Self::apply_corner_rounding(library.settings().corner_rounding, cx);
        let mut colors = UiColors::from_app(cx);
        colors.widget_opacity = library.settings().opacity;
        let view_mode_grid = library.settings().view_mode_grid;

        // Clean up obsolete tracking persistence file if present
        let obsolete_tracking_file = dirs.root().join("anilist.json");
        if obsolete_tracking_file.exists() {
            let _ = std::fs::remove_file(obsolete_tracking_file);
        }

        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search title, path, or series…"));
        cx.subscribe(
            &search_input,
            |this, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.search_query = input.read(cx).value().to_string();
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.deactivate_search(cx);
                }
                _ => {}
            },
        )
        .detach();

        let state = Self {
            focus_handle: cx.focus_handle(),
            dirs,
            library,
            colors,
            custom_themes,
            custom_theme_path,
            theme_status,
            reader: None,
            garbage: Vec::new(),
            toast,
            toast_seq: 0,
            hud: None,
            hud_seq: 0,
            importing: false,
            importing_title: None,
            imported_count: 0,
            system_dark,
            reduce_motion: false,
            last_frame: None,
            dt_smoothed: None,
            #[cfg(test)]
            frame_dt: None,
            overlay: None,
            settings_tab: SettingsTab::Appearance,
            command_selection: 0,
            library_vim: Vim::new(),
            theme_before_preview: None,
            overlay_closed_at: None,
            selection: Selection::new(),
            search_query: String::new(),
            search_active: false,
            search_input: Some(search_input),
            open_group: None,
            applied_opaque: None,
            library_scroll: ScrollHandle::new(),
            group_scroll: ScrollHandle::new(),
            continue_reading_scroll: ScrollHandle::new(),
            collections_scroll: ScrollHandle::new(),
            details_scroll: ScrollHandle::new(),
            settings_scroll: ScrollHandle::new(),
            sidebar_scroll: ScrollHandle::new(),
            library_scroll_controller: yomu::scroll::PlatformScrollController::new(),
            library_pinch_accum: 0.0,
            cursor_pos: None,
            save_seq: 0,
            last_saved_seq: Arc::new(Mutex::new(0)),
            save_pending: false,
            save_pending_since: None,
            active_section: ui::sidebar::SidebarSection::AllManga,
            active_view: ui::sidebar::MainView::AllManga,
            selected_book_id: None,
            details_tab_index: 0,
            details_synopsis_expanded: false,
            nav_history: vec![NavDestination::Section(
                ui::sidebar::SidebarSection::AllManga,
            )],
            nav_history_idx: 0,
            view_mode_grid,
            selected_collection_id: None,
            sidebar_collapsed: false,
            vim_cli_override: false,
        };

        #[cfg(not(test))]
        state.watch_custom_theme_file(cx);

        // Asking the desktop about reduced motion can take a moment (it may spawn `gsettings`),
        // so do it off the UI thread. Tests skip it to stay independent of the machine.
        #[cfg(not(test))]
        cx.spawn(async move |this, cx| {
            let reduce = cx
                .background_executor()
                .spawn(async { motion::detect_reduced_motion() })
                .await;
            let _ = this
                .update(cx, |this, cx| {
                    this.reduce_motion = reduce;
                    cx.notify();
                })
                .ok();
        })
        .detach();

        state
    }

    /// Whether page turns and scrolling should animate right now.
    fn animations_enabled(&self) -> bool {
        !self.reduce_motion && !self.library.settings().disable_animations
    }

    pub fn is_vim_enabled(&self) -> bool {
        if self.vim_cli_override {
            return true;
        }
        self.library.settings().vim_mode
    }

    fn toggle_vim_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.library.settings_mut().vim_mode = enabled;
        self.save_library();
        cx.notify();
    }

    pub fn toggle_group_series(&mut self, group: bool, cx: &mut Context<Self>) {
        self.library.settings_mut().group_series = group;
        if !group {
            self.open_group = None;
            self.group_scroll.set_offset(Point::default());
        }
        self.save_library();
        cx.notify();
    }

    fn widget_surface(&self, color: gpui_kit::Hsla) -> gpui_kit::Hsla {
        color
    }

    fn menu_surface(&self) -> gpui_kit::Hsla {
        self.colors.surface
    }

    /// Watch the generated custom theme file so Noctalia palette changes apply without a manual
    /// reload. Hashing is done on the background executor; the small TOML is parsed on change.
    #[cfg(not(test))]
    fn watch_custom_theme_file(&self, cx: &mut Context<Self>) {
        let path = self.custom_theme_path.clone();
        cx.spawn(async move |this, cx| {
            let mut last_hash = custom_theme_fingerprint(&path);
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(750))
                    .await;
                let check_path = path.clone();
                let next_hash = cx
                    .background_executor()
                    .spawn(async move { custom_theme_fingerprint(&check_path) })
                    .await;
                if next_hash.is_none() || next_hash == last_hash {
                    continue;
                }

                // Noctalia may replace the file atomically; debounce until the new contents
                // remain stable so a partial write is never applied.
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let check_path = path.clone();
                let stable_hash = cx
                    .background_executor()
                    .spawn(async move { custom_theme_fingerprint(&check_path) })
                    .await;
                if stable_hash != next_hash {
                    continue;
                }
                if this
                    .update(cx, |this, cx| this.reload_custom_themes(cx))
                    .is_err()
                {
                    break;
                }
                last_hash = stable_hash;
            }
        })
        .detach();
    }

    /// An input just started an animation: start the frame clock now so the first frame already
    /// accounts for the input-to-frame latency.
    fn wake(&mut self) {
        if self.last_frame.is_none() {
            self.last_frame = Some(Instant::now());
        }
    }

    /// Seconds since the previous animated frame, with display VSync low-pass filtering.
    fn frame_delta(&mut self, now: Instant) -> f32 {
        #[cfg(test)]
        {
            if let Some(dt) = self.frame_dt {
                return dt;
            }
        }
        let Some(previous) = self.last_frame else {
            return 0.0;
        };
        let raw_dt = now.duration_since(previous).as_secs_f32();
        if raw_dt <= 0.0 || !raw_dt.is_finite() {
            return 0.0;
        }

        // Display VSync frame-delta filter:
        // Wayland compositor frame callbacks fluctuate by ±1-3ms due to process scheduling latency,
        // even when scanout is locked to the physical display's fixed hardware refresh cycle.
        // Feeding raw jittery CPU timestamps into inertial momentum causes visible micro-jitter.
        // When consecutive frames arrive within normal scheduling variance (0.6x to 1.6x of the
        // running frame interval), apply a low-pass filter to absorb timer noise.
        // True frame drops (>1.6x) or rate changes immediately update the interval without lag.
        let dt = match self.dt_smoothed {
            Some(smoothed) if raw_dt >= smoothed * 0.6 && raw_dt <= smoothed * 1.6 => {
                let filtered = smoothed * 0.8 + raw_dt * 0.2;
                self.dt_smoothed = Some(filtered);
                filtered
            }
            _ => {
                self.dt_smoothed = Some(raw_dt);
                raw_dt
            }
        };

        motion::clamp_frame_dt(dt)
    }

    // ------------------------------------------------------------------------------------
    // Housekeeping
    // ------------------------------------------------------------------------------------

    fn save_library(&mut self) {
        self.save_seq += 1;
        let seq = self.save_seq;
        self.save_pending = false;
        self.save_pending_since = None;
        let mut guard = self.last_saved_seq.lock().unwrap();
        if let Err(err) = self.library.save(&self.dirs) {
            eprintln!("yomu: failed to save library: {err:#}");
        } else {
            *guard = seq;
        }
    }

    fn save_library_debounced(&mut self, cx: &mut Context<Self>) {
        self.save_seq += 1;
        let now = Instant::now();
        let pending_since = *self.save_pending_since.get_or_insert(now);

        if now.duration_since(pending_since) >= AUTOSAVE_MAX_DELAY {
            self.save_pending = false;
            self.save_pending_since = None;
            self.flush_save_background(cx);
            return;
        }

        if !self.save_pending {
            self.save_pending = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(AUTOSAVE_DEBOUNCE).await;
                let _ = this
                    .update(cx, |this, cx| {
                        if this.save_pending {
                            this.save_pending = false;
                            this.save_pending_since = None;
                            this.flush_save_background(cx);
                        }
                    })
                    .ok();
            })
            .detach();
        }
    }

    fn flush_save_background(&mut self, cx: &mut Context<Self>) {
        let seq = self.save_seq;
        let library = self.library.clone();
        let dirs = Arc::clone(&self.dirs);
        let last_saved = Arc::clone(&self.last_saved_seq);

        cx.background_executor()
            .spawn(async move {
                let mut guard = last_saved.lock().unwrap();
                if seq > *guard {
                    if let Err(err) = library.save(&dirs) {
                        eprintln!("yomu: failed to save library: {err:#}");
                    } else {
                        *guard = seq;
                    }
                }
            })
            .detach();
    }

    fn show_toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast_seq += 1;
        let seq = self.toast_seq;
        self.toast = Some(message.into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let _ = this
                .update(cx, |this, cx| {
                    if this.toast_seq == seq {
                        this.toast = None;
                        cx.notify();
                    }
                })
                .ok();
        })
        .detach();
    }

    /// A short message at the top of the reader (zoom level, fit mode, errors).
    fn show_hud(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        if self.reader.is_none() {
            self.show_toast(message, cx);
            return;
        }
        self.hud_seq += 1;
        let seq = self.hud_seq;
        self.hud = Some(message.into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(HUD_SECS))
                .await;
            let _ = this
                .update(cx, |this, cx| {
                    if this.hud_seq == seq {
                        this.hud = None;
                        cx.notify();
                    }
                })
                .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------------------------------
    // Menus and dialogs
    // ------------------------------------------------------------------------------------

    /// Close the current overlay without touching anything else.
    fn dismiss(&mut self) {
        if self.overlay.take().is_some() {
            self.overlay_closed_at = Some(Instant::now());
        }
    }

    /// Escape / click outside. Cancelling the "reset settings?" question goes back to Settings.
    fn close_overlay(&mut self, cx: &mut Context<Self>) {
        let back_to_settings = matches!(
            &self.overlay,
            Some(Overlay::Confirm(dialog)) if matches!(dialog.action, ConfirmAction::ResetSettings)
        );
        self.dismiss();
        if back_to_settings {
            self.overlay = Some(Overlay::Settings);
        }
        cx.notify();
    }

    #[allow(dead_code)]
    fn overlay_recently_closed(&self) -> bool {
        self.overlay_closed_at
            .is_some_and(|closed| closed.elapsed() < CLICK_GUARD)
    }

    fn open_book_menu(&mut self, id: &str, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(book) = self.library.get(id) else {
            return;
        };
        let entries = menu::book_menu(book, book.path.exists());
        self.overlay = Some(Overlay::Menu(BookMenu {
            book_id: id.to_string(),
            target_ids: vec![id.to_string()],
            entries,
            position,
            selected: None,
        }));
        cx.notify();
    }

    fn open_series_dialog(
        &mut self,
        book_ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if book_ids.is_empty() {
            return;
        }
        let books: Vec<&Book> = book_ids
            .iter()
            .filter_map(|id| self.library.get(id))
            .collect();
        let existing = books
            .first()
            .and_then(|first| first.series.clone())
            .filter(|name| {
                books.iter().all(|book| {
                    book.series
                        .as_deref()
                        .is_some_and(|other| other.eq_ignore_ascii_case(name))
                })
            });
        let common_parent = books
            .first()
            .and_then(|book| book.path.parent().map(PathBuf::from))
            .filter(|parent| {
                books
                    .iter()
                    .all(|book| book.path.parent() == Some(parent.as_path()))
            })
            .and_then(|parent| {
                parent
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            });
        let value = existing.or(common_parent).unwrap_or_default();
        self.dismiss();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value)
                .placeholder("Type a series name…")
        });
        let focus_handle = input.read(cx).focus_handle(cx);
        window.focus(&focus_handle, cx);
        cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.save_series_dialog(cx);
            }
        })
        .detach();
        self.overlay = Some(Overlay::Series(SeriesDialog { book_ids, input }));
        cx.notify();
    }

    fn save_series_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(Overlay::Series(dialog)) = self.overlay.take() else {
            return;
        };
        self.overlay_closed_at = Some(Instant::now());
        let count = dialog.book_ids.len();
        let value = dialog.input.read(cx).value().to_string();
        let series = (!value.trim().is_empty()).then_some(value);

        self.library.set_series(&dialog.book_ids, series.clone());
        self.prune_open_group();
        self.save_library();
        let message = match (&series, count) {
            (Some(name), 1) => format!("Added to series “{name}”"),
            (Some(name), _) => format!("Assigned {count} books to “{name}”"),
            (None, 1) => "Removed from series".to_string(),
            (None, _) => format!("Removed {count} books from their series"),
        };
        self.show_toast(message, cx);
        cx.notify();
    }

    fn menu_hover(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(Overlay::Menu(menu)) = self.overlay.as_mut()
            && menu.selected != Some(ix)
        {
            menu.selected = Some(ix);
            cx.notify();
        }
    }

    /// Run a menu action, asking first if it would throw away progress.
    fn run_book_action(
        &mut self,
        action: BookAction,
        id: &str,
        target_ids: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            BookAction::Open => {
                if let Some(group_key) = id.strip_prefix("group:") {
                    self.enter_group(group_key.to_string(), cx);
                    self.dismiss();
                    return;
                }
            }
            BookAction::SetSeries => {
                let ids = if !target_ids.is_empty() {
                    target_ids.to_vec()
                } else if id.starts_with("group:")
                    || (self.selection.contains(id) && self.selection.len() > 1)
                {
                    self.selected_ids()
                } else {
                    vec![id.to_string()]
                };
                self.open_series_dialog(ids, window, cx);
                return;
            }
            BookAction::MarkRead => {
                let ids = if !target_ids.is_empty() {
                    target_ids.to_vec()
                } else {
                    self.selected_ids()
                };
                if ids.len() > 1 || id.starts_with("group:") {
                    for bid in &ids {
                        if let Some(book) = self.library.get_mut(bid) {
                            book.mark_read();
                        }
                    }
                    self.save_library();
                    for bid in &ids {
                        self.selection.remove(bid);
                    }
                    self.dismiss();
                    self.show_toast(format!("Marked {} books as read", ids.len()), cx);
                    return;
                }
            }
            BookAction::MarkUnread => {
                let ids = if !target_ids.is_empty() {
                    target_ids.to_vec()
                } else {
                    self.selected_ids()
                };
                if ids.len() > 1 || id.starts_with("group:") {
                    for bid in &ids {
                        if let Some(book) = self.library.get_mut(bid) {
                            book.reset_progress();
                        }
                    }
                    self.save_library();
                    for bid in &ids {
                        self.selection.remove(bid);
                    }
                    self.dismiss();
                    self.show_toast(format!("Marked {} books as unread", ids.len()), cx);
                    return;
                }
            }
            BookAction::RemoveSelected => {
                if id.starts_with("group:") || target_ids.len() > 1 {
                    self.request_remove_books(target_ids, cx);
                    return;
                }
                self.request_remove_selected(cx);
                return;
            }
            BookAction::ClearSelection => {
                self.dismiss();
                self.selection.clear();
                cx.notify();
                return;
            }
            _ => {}
        }
        let Some(book) = self.library.get(id).cloned() else {
            self.dismiss();
            cx.notify();
            return;
        };
        if let Some(confirmation) = menu::confirmation(action, &book) {
            self.overlay = Some(Overlay::Confirm(ConfirmDialog {
                title: confirmation.title.into(),
                message: confirmation.message.into(),
                confirm_label: confirmation.confirm_label,
                action: ConfirmAction::Book(action, id.to_string()),
            }));
            cx.notify();
            return;
        }
        self.perform_book_action(action, id, window, cx);
    }

    fn perform_book_action(
        &mut self,
        action: BookAction,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss();
        let Some(title) = self.library.get(id).map(|book| book.title.clone()) else {
            cx.notify();
            return;
        };
        match action {
            BookAction::Open => self.open_book(id, window, cx),
            BookAction::StartOver => {
                if let Some(book) = self.library.get_mut(id) {
                    book.start_over();
                }
                self.save_library();
                self.open_book(id, window, cx);
            }
            BookAction::MarkRead => {
                if let Some(book) = self.library.get_mut(id) {
                    book.mark_read();
                }
                self.save_library();
                self.show_toast(format!("Marked “{title}” as read"), cx);
            }
            BookAction::MarkUnread | BookAction::ResetProgress => {
                if let Some(book) = self.library.get_mut(id) {
                    book.reset_progress();
                }
                self.save_library();
                let message = if action == BookAction::ResetProgress {
                    format!("Progress reset for “{title}”")
                } else {
                    format!("Marked “{title}” as unread")
                };
                self.show_toast(message, cx);
            }
            BookAction::RevealFile => match self.library.get(id).map(|book| book.path.clone()) {
                Some(path) if path.exists() => cx.reveal_path(&path),
                _ => self.show_toast(format!("“{title}” is missing from disk"), cx),
            },
            BookAction::Properties => {
                if let Some(book) = self.library.get(id) {
                    self.overlay = Some(Overlay::Properties(PropertiesDialog {
                        title: book.title.clone().into(),
                        rows: menu::properties(book),
                    }));
                }
            }
            BookAction::SetSeries => {}
            BookAction::Remove => self.remove_book(id, cx),
            // Handled in `run_book_action`; they never reach here.
            BookAction::RemoveSelected | BookAction::ClearSelection => {}
        }
        cx.notify();
    }

    fn confirm_accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Overlay::Confirm(dialog)) = self.overlay.take() else {
            return;
        };
        self.overlay_closed_at = Some(Instant::now());
        match dialog.action {
            ConfirmAction::Book(action, id) => self.perform_book_action(action, &id, window, cx),
            ConfirmAction::RemoveBooks(ids) => self.remove_books(&ids, cx),
            ConfirmAction::RemoveSelected => self.remove_selected(cx),
            ConfirmAction::ResetSettings => self.reset_settings(cx),
            ConfirmAction::ClearThumbnailCache => self.clear_thumbnail_cache(cx),
            ConfirmAction::ClearAllData => self.clear_all_data(cx),
            ConfirmAction::DeleteCollection(id) => {
                self.library.delete_collection(&id);
                self.save_library();
                self.show_toast("Collection deleted", cx);
            }
        }
    }

    // ------------------------------------------------------------------------------------
    // Settings
    // ------------------------------------------------------------------------------------

    fn open_settings(&mut self, cx: &mut Context<Self>) {
        match self.overlay {
            None => self.overlay = Some(Overlay::Settings),
            Some(Overlay::Settings) => self.close_overlay(cx),
            Some(_) => {}
        }
        cx.notify();
    }

    fn open_settings_window(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(cx);
    }

    fn set_thumbnail_size(&mut self, size: ThumbnailSize, cx: &mut Context<Self>) {
        if self.library.settings().thumbnail_size == size {
            return;
        }
        self.library.settings_mut().thumbnail_size = size;
        self.save_library();
        cx.notify();

        let library = self.library.clone();
        let dirs = (*self.dirs).clone();
        cx.background_executor()
            .spawn(async move {
                library.ensure_covers_for_size(&dirs, size);
            })
            .detach();
    }

    fn border_px(&self) -> f32 {
        self.library.settings().border_size.pixels()
    }

    fn radius_px(&self) -> f32 {
        self.library.settings().corner_rounding.radius_pixels()
    }

    fn radius_lg_px(&self) -> f32 {
        self.library.settings().corner_rounding.radius_lg_pixels()
    }

    fn radius_xl_px(&self) -> f32 {
        self.library.settings().corner_rounding.radius_xl_pixels()
    }

    #[allow(dead_code)]
    fn set_border_size(&mut self, size: BorderSize, cx: &mut Context<Self>) {
        if self.library.settings().border_size == size {
            return;
        }
        self.library.settings_mut().border_size = size;
        self.save_library();
        cx.notify();
    }

    #[allow(dead_code)]
    fn set_corner_rounding(&mut self, rounding: CornerRounding, cx: &mut Context<Self>) {
        if self.library.settings().corner_rounding == rounding {
            return;
        }
        self.library.settings_mut().corner_rounding = rounding;
        Self::apply_corner_rounding(rounding, cx);
        self.save_library();
        cx.notify();
    }

    fn apply_corner_rounding(rounding: CornerRounding, cx: &mut App) {
        KitTheme::update(cx, |theme| {
            theme.radius = px(rounding.radius_pixels());
            theme.radius_lg = px(rounding.radius_lg_pixels());
        });
    }

    fn request_clear_thumbnail_cache(&mut self, cx: &mut Context<Self>) {
        self.overlay = Some(Overlay::Confirm(ConfirmDialog {
            title: "Clear thumbnail cache?".into(),
            message: "All cached cover thumbnails will be deleted and regenerated on demand. Comics and reading progress remain intact.".into(),
            confirm_label: "Clear Cache",
            action: ConfirmAction::ClearThumbnailCache,
        }));
        cx.notify();
    }

    fn clear_thumbnail_cache(&mut self, cx: &mut Context<Self>) {
        let _ = self.library.clear_cover_cache(&self.dirs);
        let size = self.library.settings().thumbnail_size;
        let library = self.library.clone();
        let dirs = (*self.dirs).clone();
        cx.background_executor()
            .spawn(async move {
                library.ensure_covers_for_size(&dirs, size);
            })
            .detach();
        self.overlay = Some(Overlay::Settings);
        cx.notify();
    }

    fn request_clear_all_data(&mut self, cx: &mut Context<Self>) {
        self.overlay = Some(Overlay::Confirm(ConfirmDialog {
            title: "Reset all application data?".into(),
            message: "This will permanently remove all books, reading progress, marks, settings, and cached thumbnails.".into(),
            confirm_label: "Reset Everything",
            action: ConfirmAction::ClearAllData,
        }));
        cx.notify();
    }

    fn clear_all_data(&mut self, cx: &mut Context<Self>) {
        self.reader = None;
        self.selection.clear();
        self.open_group = None;
        self.group_scroll.set_offset(Point::default());
        let _ = self.library.clear_all_data(&self.dirs);
        self.overlay = Some(Overlay::Settings);
        cx.notify();
    }

    fn request_reset_settings(&mut self, cx: &mut Context<Self>) {
        if *self.library.settings() == Settings::default() {
            return;
        }
        self.overlay = Some(Overlay::Confirm(ConfirmDialog {
            title: "Reset settings?".into(),
            message: "All preferences go back to their defaults.".into(),
            confirm_label: "Reset",
            action: ConfirmAction::ResetSettings,
        }));
        cx.notify();
    }

    fn reset_settings(&mut self, cx: &mut Context<Self>) {
        *self.library.settings_mut() = Settings::default();
        let defaults = self.library.settings().clone();
        let resolved = ui::theme::resolve_theme_name(
            &defaults.color_scheme,
            defaults.theme_mode,
            self.system_dark,
            &self.custom_themes,
        );
        self.apply_theme(&resolved, cx);
        if let Some(reader) = self.reader.as_mut() {
            reader.fit = defaults.fit;
            reader.zoom = 1.0;
            reader.continuous = defaults.continuous;
            reader.webcomics = defaults.webcomics;
            reader.continuous |= reader.webcomics;
            reader.arrive = Some(Arrive::Start);
        }
        self.save_library();
        self.overlay = Some(Overlay::Settings);
        cx.notify();
    }

    fn apply_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        let config = self
            .custom_themes
            .get(name)
            .cloned()
            .or_else(|| built_in_theme(name));
        let Some(config) = config else {
            self.show_toast(format!("Theme “{name}” is unavailable"), cx);
            return;
        };
        KitTheme::update(cx, |theme| theme.apply_config(&config));
        Self::apply_corner_rounding(self.library.settings().corner_rounding, cx);
        self.library.settings_mut().theme_name = name.to_string();
        if name.starts_with("Catppuccin") {
            self.library.settings_mut().color_scheme = "Catppuccin".to_string();
        } else if name.starts_with("Gruvbox") {
            self.library.settings_mut().color_scheme = "Gruvbox".to_string();
        } else if self.custom_themes.contains_key(name) {
            let base = name
                .strip_suffix(" Dark")
                .or_else(|| name.strip_suffix(" Light"))
                .unwrap_or(name);
            self.library.settings_mut().color_scheme = base.to_string();
        }
        self.colors = UiColors::from_app(cx);
        self.colors.widget_opacity = self.library.settings().opacity;
        self.theme_before_preview = None;
        self.save_library();
        cx.notify();
    }

    fn preview_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        let config = self
            .custom_themes
            .get(name)
            .cloned()
            .or_else(|| built_in_theme(name));
        let Some(config) = config else {
            return;
        };
        KitTheme::update(cx, |theme| theme.apply_config(&config));
        Self::apply_corner_rounding(self.library.settings().corner_rounding, cx);
        self.colors = UiColors::from_app(cx);
        self.colors.widget_opacity = self.library.settings().opacity;
        cx.notify();
    }

    fn restore_theme_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(prev) = self.theme_before_preview.take()
            && self.library.settings().theme_name != prev
        {
            self.preview_theme(&prev, cx);
        }
    }

    fn sync_theme_preview(&mut self, draft: &str, cx: &mut Context<Self>) {
        let trimmed = draft.trim_start();
        let words = trimmed.split_whitespace();
        let first_word = words.into_iter().next().unwrap_or_default();
        if matches!(first_word, "theme" | "colorscheme" | "colo" | "colors") {
            if self.theme_before_preview.is_none() {
                self.theme_before_preview = Some(self.library.settings().theme_name.clone());
            }
            let suggestions = command_suggestions(draft, &self.custom_themes);
            if let Some((_, label, _)) = suggestions.get(self.command_selection) {
                self.preview_theme(label, cx);
            }
        } else {
            self.restore_theme_preview(cx);
        }
    }

    fn in_cmdline(&self) -> bool {
        if let Some(reader) = &self.reader {
            reader.vim.in_cmdline()
        } else {
            self.library_vim.in_cmdline()
        }
    }

    fn cmdline_text(&self) -> Option<&str> {
        if let Some(reader) = &self.reader {
            reader.vim.cmdline_text()
        } else {
            self.library_vim.cmdline_text()
        }
    }

    fn replace_cmdline(&mut self, text: String) {
        if let Some(reader) = self.reader.as_mut() {
            reader.vim.replace_cmdline(text);
        } else {
            self.library_vim.replace_cmdline(text);
        }
    }

    fn cancel_cmdline(&mut self, cx: &mut Context<Self>) {
        if let Some(reader) = self.reader.as_mut() {
            reader.vim.cancel_cmdline();
        } else {
            self.library_vim.cancel_cmdline();
        }
        self.command_selection = 0;
        self.restore_theme_preview(cx);
        cx.notify();
    }

    fn cycle_theme(&mut self, delta: isize, cx: &mut Context<Self>) {
        let mut names: Vec<String> = BUILT_IN_THEMES.keys().cloned().collect();
        names.extend(self.custom_themes.keys().cloned());
        names.sort_by_key(|name| name.to_lowercase());
        if names.is_empty() {
            return;
        }
        let current = self.library.settings().theme_name.as_str();
        let index = names.iter().position(|name| name == current).unwrap_or(0) as isize;
        let next = (index + delta).rem_euclid(names.len() as isize) as usize;
        self.apply_theme(&names[next], cx);
    }

    fn set_theme_mode(&mut self, mode: ThemeModePreference, cx: &mut Context<Self>) {
        self.library.settings_mut().theme_mode = mode;
        let scheme = self.library.settings().color_scheme.clone();
        let resolved =
            ui::theme::resolve_theme_name(&scheme, mode, self.system_dark, &self.custom_themes);
        self.apply_theme(&resolved, cx);
    }

    #[allow(dead_code)]
    fn cycle_color_scheme(&mut self, cx: &mut Context<Self>) {
        let schemes = ui::theme::available_color_schemes(&self.custom_themes);
        if schemes.is_empty() {
            return;
        }
        let current = self.library.settings().color_scheme.as_str();
        let index = schemes.iter().position(|s| s == current).unwrap_or(0);
        let next = (index + 1) % schemes.len();
        let next_scheme = schemes[next].clone();
        self.library.settings_mut().color_scheme = next_scheme.clone();
        let mode = self.library.settings().theme_mode;
        let resolved = ui::theme::resolve_theme_name(
            &next_scheme,
            mode,
            self.system_dark,
            &self.custom_themes,
        );
        self.apply_theme(&resolved, cx);
    }

    fn set_color_scheme(&mut self, scheme: &str, cx: &mut Context<Self>) {
        self.library.settings_mut().color_scheme = scheme.to_string();
        let mode = self.library.settings().theme_mode;
        let resolved =
            ui::theme::resolve_theme_name(scheme, mode, self.system_dark, &self.custom_themes);
        self.apply_theme(&resolved, cx);
    }

    #[allow(dead_code)]
    fn reload_custom_themes(&mut self, cx: &mut Context<Self>) {
        let themes = match ensure_custom_theme_file(&self.custom_theme_path)
            .and_then(|()| load_custom_themes(&self.custom_theme_path, cx))
        {
            Ok(themes) => themes,
            Err(error) => {
                self.theme_status = Some(format!("Custom theme file error: {error:#}"));
                cx.notify();
                return;
            }
        };

        let selected = self.library.settings().theme_name.clone();
        let was_custom = self.custom_themes.contains_key(&selected);
        self.custom_themes = themes;
        self.theme_status = Some(format!(
            "Loaded {} custom theme(s) from {}",
            self.custom_themes.len(),
            self.custom_theme_path.display()
        ));

        if let Some(config) = self.custom_themes.get(&selected).cloned() {
            KitTheme::update(cx, |theme| theme.apply_config(&config));
            Self::apply_corner_rounding(self.library.settings().corner_rounding, cx);
            self.colors = UiColors::from_app(cx);
            self.colors.widget_opacity = self.library.settings().opacity;
        } else if was_custom {
            self.apply_theme(DEFAULT_THEME_NAME, cx);
        }
        cx.notify();
    }

    /// Fully opaque windows use the opaque appearance, which lets the compositor skip blending;
    /// anything lower needs real alpha transparency.
    fn sync_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let is_dark = window.appearance() == gpui_kit::WindowAppearance::Dark;
        if self.system_dark != is_dark {
            self.system_dark = is_dark;
            if self.library.settings().theme_mode == ThemeModePreference::System {
                let scheme = self.library.settings().color_scheme.clone();
                let resolved = ui::theme::resolve_theme_name(
                    &scheme,
                    ThemeModePreference::System,
                    is_dark,
                    &self.custom_themes,
                );
                if self.library.settings().theme_name != resolved {
                    self.apply_theme(&resolved, cx);
                }
            }
        }
        if self.applied_opaque == Some(true) {
            return;
        }
        self.applied_opaque = Some(true);
        window.defer(cx, move |window, _| {
            window.set_background_appearance(WindowBackgroundAppearance::Opaque)
        });
    }

    fn on_overlay_key(&mut self, token: &str, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(token, "enter" | "space")
            && let Some(Overlay::LibraryMenu(menu)) = self.overlay.as_ref()
        {
            let selected = menu.selected;
            self.run_library_menu_action(selected, window, cx);
            return;
        }
        match self.overlay.as_mut() {
            Some(Overlay::Menu(menu)) => match token {
                "escape" => self.close_overlay(cx),
                "down" | "j" | "tab" | "C-n" => {
                    menu.selected = menu::step(&menu.entries, menu.selected, true);
                    cx.notify();
                }
                "up" | "k" | "S-tab" | "C-p" => {
                    menu.selected = menu::step(&menu.entries, menu.selected, false);
                    cx.notify();
                }
                "home" => {
                    menu.selected = menu::step(&menu.entries, None, true);
                    cx.notify();
                }
                "end" => {
                    menu.selected = menu::step(&menu.entries, None, false);
                    cx.notify();
                }
                "enter" | "space" => {
                    let picked = menu.selected.and_then(|ix| match &menu.entries[ix] {
                        Entry::Item(item) if item.enabled => Some(item.action),
                        _ => None,
                    });
                    let id = menu.book_id.clone();
                    let target_ids = menu.target_ids.clone();
                    if let Some(action) = picked {
                        self.run_book_action(action, &id, &target_ids, window, cx);
                    }
                }
                _ => {}
            },
            Some(Overlay::LibraryMenu(menu)) => match token {
                "escape" => self.close_overlay(cx),
                "down" | "j" | "tab" | "C-n" => {
                    menu.selected = (menu.selected + 1) % 5;
                    cx.notify();
                }
                "up" | "k" | "S-tab" | "C-p" => {
                    menu.selected = (menu.selected + 4) % 5;
                    cx.notify();
                }
                "home" => {
                    menu.selected = 0;
                    cx.notify();
                }
                "end" => {
                    menu.selected = 4;
                    cx.notify();
                }
                _ => {}
            },
            Some(Overlay::Confirm(_)) => match token {
                "enter" | "y" => self.confirm_accept(window, cx),
                "escape" | "n" | "q" => self.close_overlay(cx),
                _ => {}
            },
            Some(Overlay::Properties(_)) => {
                if matches!(token, "escape" | "enter" | "space" | "q") {
                    self.close_overlay(cx);
                }
            }
            Some(Overlay::Series(_)) => {}
            Some(Overlay::Collection(_)) => {
                if matches!(token, "escape") {
                    self.close_overlay(cx);
                }
            }
            Some(Overlay::Settings) => {
                if matches!(token, "escape" | "enter" | "q") {
                    self.close_overlay(cx);
                }
            }
            None => {}
        }
    }

    fn run_library_menu_action(
        &mut self,
        selected: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overlay = None;
        match selected {
            0 => self.import_dialog(false, cx),
            1 => self.import_dialog(true, cx),
            2 => self.open_settings_window(_window, cx),
            3 => {
                self.library.settings_mut().group_series = !self.library.settings().group_series;
                self.selection.clear();
                self.open_group = None;
                self.group_scroll.set_offset(Point::default());
                self.save_library();
                cx.notify();
            }
            4 => {
                self.save_library();
                cx.quit();
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------------------------
    // Import
    // ------------------------------------------------------------------------------------

    fn import_dialog(&mut self, directories: bool, cx: &mut Context<Self>) {
        if self.importing || self.overlay.is_some() {
            return;
        }
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: !directories,
            directories,
            multiple: true,
            prompt: Some("Import".into()),
        });
        cx.spawn(async move |this, cx| match rx.await {
            Ok(Ok(Some(paths))) => {
                this.update(cx, |this, cx| this.import_paths(paths, cx))
                    .ok();
            }
            Ok(Err(err)) => {
                let message = format!("{err:#}");
                this.update(cx, |this, cx| this.show_toast(message, cx))
                    .ok();
            }
            _ => {}
        })
        .detach();
    }

    fn import_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.importing || paths.is_empty() {
            return;
        }
        self.importing = true;
        self.imported_count = 0;
        self.importing_title = None;
        cx.notify();

        let dirs = self.dirs.clone();
        let existing = self.library.ids();
        let (sender, receiver) = mpsc::channel();
        cx.background_executor()
            .spawn(async move {
                let report =
                    library::import_paths_with_progress(&paths, &dirs, &existing, |book| {
                        let _ = sender.send(ImportEvent::Book(book.clone()));
                    });
                let _ = sender.send(ImportEvent::Finished(report));
            })
            .detach();

        cx.spawn(async move |this, cx| {
            loop {
                match receiver.try_recv() {
                    Ok(ImportEvent::Book(book)) => {
                        let _ = this
                            .update(cx, |this, cx| {
                                let title = book.title.clone();
                                if this.library.add(book) {
                                    this.save_library();
                                    this.imported_count += 1;
                                }
                                this.importing_title = Some(title);
                                cx.notify();
                            })
                            .ok();
                        // Leave a frame for the newly inserted cover/title before showing the
                        // next completed volume, even when several finish almost together.
                        cx.background_executor()
                            .timer(Duration::from_millis(16))
                            .await;
                    }
                    Ok(ImportEvent::Finished(report)) => {
                        let _ = this
                            .update(cx, |this, cx| {
                                this.importing = false;
                                this.importing_title = None;
                                this.show_toast(report.summary(), cx);
                            })
                            .ok();
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        cx.background_executor()
                            .timer(Duration::from_millis(40))
                            .await;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        let _ = this
                            .update(cx, |this, cx| {
                                this.importing = false;
                                this.importing_title = None;
                                cx.notify();
                            })
                            .ok();
                        break;
                    }
                }
            }
        })
        .detach();
    }

    fn remove_book(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(title) = self.library.get(id).map(|b| b.title.clone()) else {
            return;
        };
        self.library.remove(id, &self.dirs);
        self.prune_selection();
        self.prune_open_group();
        self.save_library();
        self.show_toast(format!("Removed “{title}” from the library"), cx);
    }

    // ------------------------------------------------------------------------------------
    // Selection and bulk removal
    // ------------------------------------------------------------------------------------

    /// Book ids in library order.
    fn library_order(&self) -> Vec<String> {
        self.library
            .books()
            .iter()
            .map(|book| book.id.clone())
            .collect()
    }

    /// Book ids in the currently viewed scope (the open group, or the full library).
    fn current_view_order(&self) -> Vec<String> {
        if self.library.settings().group_series
            && let Some(open_key) = self.open_group.as_deref()
        {
            let groups = directory_groups(self.library.books());
            if let Some(group) = groups.into_iter().find(|g| g.key == open_key) {
                return group.books.iter().map(|book| book.id.clone()).collect();
            }
        }
        self.library_order()
    }

    fn prune_open_group(&mut self) {
        if self.open_group.is_some() {
            let still_exists = self.library.settings().group_series
                && self.open_group.as_deref().is_some_and(|open_key| {
                    directory_groups(self.library.books())
                        .iter()
                        .any(|g| g.key == open_key)
                });
            if !still_exists {
                self.open_group = None;
                self.group_scroll.set_offset(Point::default());
            }
        }
    }

    fn selected_ids(&self) -> Vec<String> {
        self.selection.in_order(&self.current_view_order())
    }

    fn prune_selection(&mut self) {
        let existing = self.library.ids();
        self.selection.retain_existing(&existing);
    }

    /// A click on a book card with the given modifiers. Returns `false` if it is a plain click
    /// that should open the book instead.
    fn select_click(&mut self, id: &str, ctrl: bool, shift: bool, cx: &mut Context<Self>) -> bool {
        let order = self.current_view_order();
        let handled = self.selection.click(&order, id, ctrl, shift);
        if handled {
            cx.notify();
        }
        handled
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        let order = self.current_view_order();
        self.selection.select_all(&order);
        cx.notify();
    }

    /// Right-click on a book: a book inside an existing multi-selection preserves the multi-selection
    /// and gets the bulk menu; right-clicking an unselected book drops previous selection (if any),
    /// does not select the book or trigger selection mode, and opens the book's individual menu.
    fn open_context_menu(
        &mut self,
        id: &str,
        position: Point<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_ids = self.selected_ids();
        let (entries, target_ids) =
            if selected_ids.contains(&id.to_string()) && selected_ids.len() > 1 {
                (menu::selection_menu(selected_ids.len()), selected_ids)
            } else {
                self.selection.clear();
                let Some(book) = self.library.get(id) else {
                    return;
                };
                (
                    menu::book_menu(book, book.path.exists()),
                    vec![id.to_string()],
                )
            };
        let menu = BookMenu {
            book_id: id.to_string(),
            target_ids,
            entries,
            position,
            selected: None,
        };
        self.overlay = Some(Overlay::Menu(menu));
        cx.notify();
    }

    fn open_group_context_menu(
        &mut self,
        group_key: &str,
        group_name: &str,
        book_ids: &[String],
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        // Right-clicking a folder or group opens its context menu for that folder's contents,
        // without selecting all books or activating selection mode/toolbar.
        self.selection.clear();
        let entries = menu::group_menu(group_name, book_ids.len());
        let menu = BookMenu {
            book_id: format!("group:{}", group_key),
            target_ids: book_ids.to_vec(),
            entries,
            position,
            selected: None,
        };
        self.overlay = Some(Overlay::Menu(menu));
        cx.notify();
    }

    #[allow(dead_code)]
    fn open_library_menu(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Some(Overlay::LibraryMenu(_))) {
            self.close_overlay(cx);
            return;
        }
        self.overlay = Some(Overlay::LibraryMenu(LibraryMenu::default()));
        cx.notify();
    }

    /// Remove books from the library, asking first only if progress or marks would be lost.
    fn request_remove_books(&mut self, ids: &[String], cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let confirmation = {
            let books: Vec<&Book> = ids.iter().filter_map(|id| self.library.get(id)).collect();
            menu::bulk_remove_confirmation(&books)
        };
        match confirmation {
            Some(confirmation) => {
                self.overlay = Some(Overlay::Confirm(ConfirmDialog {
                    title: confirmation.title.into(),
                    message: confirmation.message.into(),
                    confirm_label: confirmation.confirm_label,
                    action: ConfirmAction::RemoveBooks(ids.to_vec()),
                }));
                cx.notify();
            }
            None => self.remove_books(ids, cx),
        }
    }

    fn remove_books(&mut self, ids: &[String], cx: &mut Context<Self>) {
        self.dismiss();
        for id in ids {
            self.library.remove(id, &self.dirs);
            self.selection.remove(id);
        }
        self.prune_open_group();
        self.save_library();
        let message = if ids.len() == 1 {
            "Removed 1 book from the library".to_string()
        } else {
            format!("Removed {} books from the library", ids.len())
        };
        self.show_toast(message, cx);
    }

    /// Remove every selected book from the library, asking first only if progress or marks
    /// would be lost. The comic files are never touched.
    fn request_remove_selected(&mut self, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
        self.request_remove_books(&ids, cx);
    }

    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
        self.remove_books(&ids, cx);
    }

    fn on_library_key(
        &mut self,
        token: &str,
        _key_char: Option<&str>,
        _control: bool,
        _alt: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(token, "C-f" | "/" | "slash" | "C-k") {
            self.activate_search(window, cx);
            return;
        }
        if self.search_active {
            match token {
                "escape" => {
                    if self.search_query.is_empty() {
                        self.deactivate_search(cx);
                    } else {
                        self.search_query.clear();
                        if let Some(input) = &self.search_input {
                            input.update(cx, |state, cx| {
                                state.set_value("", window, cx);
                            });
                        }
                        cx.notify();
                    }
                }
                "enter" => {
                    self.deactivate_search(cx);
                }
                _ => {}
            }
            return;
        }
        match token {
            "C-a" => {
                let order = self.current_view_order();
                self.selection.select_all(&order);
                cx.notify();
            }
            "escape" | "backspace" => {
                if !self.selection.is_empty() {
                    self.selection.clear();
                    cx.notify();
                } else if self.open_group.is_some() {
                    self.exit_group(cx);
                }
            }
            "delete" => self.request_remove_selected(cx),
            _ => {}
        }
    }

    #[allow(dead_code)]
    fn enter_group(&mut self, key: String, cx: &mut Context<Self>) {
        let name = key.rsplit(['/', '\\']).next().unwrap_or(&key).to_string();
        self.enter_group_named(key, name, cx);
    }

    fn enter_group_named(&mut self, key: String, name: String, cx: &mut Context<Self>) {
        self.selection.clear();
        self.group_scroll.set_offset(Point::default());
        self.navigate_to_dest(NavDestination::Group { key, name }, cx);
    }

    fn exit_group(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
        self.group_scroll.set_offset(Point::default());
        if matches!(
            self.nav_history.get(self.nav_history_idx),
            Some(NavDestination::Group { .. })
        ) {
            self.navigate_back(cx);
        } else {
            self.open_group = None;
            self.navigate_to_dest(NavDestination::Section(self.active_section), cx);
        }
    }

    fn activate_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_active = true;
        if let Some(input) = &self.search_input {
            let focus_handle = input.read(cx).focus_handle(cx);
            window.focus(&focus_handle, cx);
        }
        cx.notify();
    }

    fn deactivate_search(&mut self, cx: &mut Context<Self>) {
        self.search_active = false;
        cx.notify();
    }

    // ------------------------------------------------------------------------------------
    // Opening and closing books
    // ------------------------------------------------------------------------------------

    fn open_book(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(book) = self.library.get(id).cloned() else {
            return;
        };
        if !book.path.exists() {
            self.show_toast(format!("“{}” is missing from disk", book.title), cx);
            return;
        }

        self.selection.clear();
        self.search_active = false;
        self.search_query.clear();
        if let Some(input) = &self.search_input {
            input.update(cx, |state, cx| {
                state.set_value("", window, cx);
            });
        }
        window.set_window_title(&book.title);
        window.focus(&self.focus_handle, cx);

        let settings = self.library.settings().clone();
        self.reader = Some(Reader::new(&book, &settings));

        if let Some(entry) = self.library.get_mut(id) {
            entry.opened = true;
            entry.last_read = Some(now_secs());
        }
        self.save_library();

        // Listing a big archive's pages touches the disk, so keep it off the UI thread.
        let (path, kind, book_id) = (book.path, book.kind, book.id);
        let task = cx
            .background_executor()
            .spawn(async move { archive::list_pages(&path, kind) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| this.pages_listed(book_id, result, cx))
                .ok();
        })
        .detach();

        cx.notify();
    }

    fn pages_listed(
        &mut self,
        book_id: String,
        result: Result<Vec<String>>,
        cx: &mut Context<Self>,
    ) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };

        if reader.book_id != book_id {
            return;
        }
        match result {
            Ok(pages) if pages.is_empty() => reader.error = Some("no pages found".into()),
            Ok(pages) => {
                reader.total = pages.len();
                reader.page = reader.page.min(pages.len() - 1);
                let (page, total) = (reader.page, pages.len());
                reader.pages = Some(Arc::new(pages));
                reader.orig_sizes = vec![None; total];
                if let Some(book) = self.library.get_mut(&book_id) {
                    book.page_count = total;
                    book.current_page = page;
                }
                self.save_library();
            }
            Err(err) => reader.error = Some(format!("{err:#}")),
        }
        cx.notify();
    }

    fn close_book(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(reader) = self.reader.take() {
            for token in reader.cancellations.values() {
                token.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            self.garbage.extend(reader.into_images());
        }
        self.last_frame = None;
        self.hud = None;
        self.cursor_pos = None;
        window.set_window_title("yomu");
        window.focus(&self.focus_handle, cx);
        self.save_library();
        cx.notify();
    }

    // ------------------------------------------------------------------------------------
    // Page turning
    // ------------------------------------------------------------------------------------

    fn goto_page(&mut self, target: usize, arrive: Arrive, cx: &mut Context<Self>) {
        let animate = self.animations_enabled();
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        if reader.pages.is_none() || reader.total == 0 {
            return;
        }
        let target = target.min(reader.total - 1);
        if target == reader.page {
            return;
        }
        // The reading position changes immediately; the slide is only a visual that catches up.
        reader.begin_transition(target > reader.page, animate && !reader.continuous);
        reader.page = target;
        reader.arrive = Some(arrive);
        reader.drag = None;
        let book_id = reader.book_id.clone();
        if let Some(book) = self.library.get_mut(&book_id) {
            book.current_page = target;
            book.opened = true;
            book.last_read = Some(now_secs());
        }
        self.save_library();
        self.wake();
        cx.notify();
    }

    /// A jump that `''` can undo (`gg`, `G`, `12G`, `:12`, marks).
    fn jump_to(&mut self, target: usize, cx: &mut Context<Self>) {
        if let Some(reader) = self.reader.as_mut() {
            let clamped = target.min(reader.total.saturating_sub(1));
            if clamped != reader.page {
                reader.back_page = Some(reader.page);
            }
        }
        self.goto_page(target, Arrive::Start, cx);
    }

    fn turn_by(&mut self, forward: bool, count: usize, arrive: Arrive, cx: &mut Context<Self>) {
        let Some(page) = self.reader.as_ref().map(|r| r.page) else {
            return;
        };
        let target = if forward {
            page.saturating_add(count)
        } else {
            page.saturating_sub(count)
        };
        self.goto_page(target, arrive, cx);
    }

    /// Screen-edge clicks: left goes back, right advances.
    fn step(&mut self, side: Side, cx: &mut Context<Self>) {
        if self.reader.is_none() {
            return;
        }
        let forward = matches!(side, Side::Right);
        let arrive = if forward { Arrive::Start } else { Arrive::End };
        self.turn_by(forward, 1, arrive, cx);
    }

    // ------------------------------------------------------------------------------------
    // Scrolling, zooming, fit modes
    // ------------------------------------------------------------------------------------

    /// `j`/`k`/`h`/`l`/space/etc. Scrolls the page; when the page cannot scroll any further in
    /// that direction (or does not overflow at all) it turns the page instead.
    fn scroll(
        &mut self,
        dir: Dir,
        amount: Amount,
        count: usize,
        vp: Viewport,
        cx: &mut Context<Self>,
    ) {
        let count = count.max(1);
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let Some(display) = reader.settle(&vp) else {
            return;
        };
        let max = if reader.continuous {
            let doc_height = reader.continuous_total_height(&vp);
            (
                (display.0 - vp.width).max(0.0),
                (doc_height - vp.height).max(0.0),
            )
        } else {
            layout::max_offset(display, &vp)
        };
        let vertical = matches!(dir, Dir::Up | Dir::Down);
        let positive = matches!(dir, Dir::Down | Dir::Right);
        // Edge checks and repeated presses work on where the scroll is *heading*, so pressing
        // `j` several times quickly accumulates instead of restarting from the glide's position.
        let (axis_max, view, current) = if vertical {
            (max.1, vp.height, reader.scroll_target.1)
        } else {
            (max.0, vp.width, reader.scroll_target.0)
        };

        // Which way a page turn goes when the page cannot scroll: down/right is forward.
        let turn_forward = positive;
        let arrive = if turn_forward {
            Arrive::Start
        } else {
            Arrive::End
        };

        if axis_max <= EDGE_EPS && !(reader.continuous && vertical) {
            self.turn_by(turn_forward, count, arrive, cx);
            return;
        }
        let at_end = if positive {
            current >= axis_max - EDGE_EPS
        } else {
            current <= EDGE_EPS
        };
        if at_end && !(reader.continuous && vertical) {
            self.turn_by(turn_forward, 1, arrive, cx);
            return;
        }

        let step = match amount {
            Amount::Line => LINE_STEP,
            Amount::HalfPage => view * 0.5,
            Amount::Page => view * 0.9,
        };
        let delta = step * count as f32 * if positive { 1.0 } else { -1.0 };
        let target = (current + delta).clamp(0.0, axis_max);
        if vertical {
            reader.scroll_target.1 = target;
        } else {
            reader.scroll_target.0 = target;
        }
        if !reader.animate {
            reader.off = reader.scroll_target;
        }
        self.wake();
        cx.notify();
    }

    fn scroll_vertical_to(&mut self, fraction: f32, vp: Viewport, cx: &mut Context<Self>) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let Some(display) = reader.settle(&vp) else {
            return;
        };
        let max = if reader.continuous {
            (reader.continuous_total_height(&vp) - vp.height).max(0.0)
        } else {
            layout::max_offset(display, &vp).1
        };
        reader.scroll_target.1 = max * fraction;
        self.wake();
        cx.notify();
    }

    fn zoom_to(&mut self, zoom: f32, anchor: (f32, f32), vp: Viewport, cx: &mut Context<Self>) {
        let message = {
            let Some(reader) = self.reader.as_mut() else {
                return;
            };
            let Some(old_display) = reader.settle(&vp) else {
                return;
            };
            if reader.continuous {
                let old_zoom = reader.zoom;
                let old_doc = reader.continuous_total_height(&vp);
                let zoom = layout::clamp_zoom(zoom);
                reader.zoom = zoom;
                let new_doc = reader.continuous_total_height(&vp);

                if old_zoom > 1e-4 {
                    let zoom_ratio = zoom / old_zoom;
                    let target_y = (reader.off.1 + anchor.1) * zoom_ratio - anchor.1;
                    let max_y = (new_doc - vp.height).max(0.0);
                    reader.scroll_target.1 = target_y.clamp(0.0, max_y);
                } else if old_doc > vp.height {
                    reader.scroll_target.1 = reader.scroll_target.1 / (old_doc - vp.height)
                        * (new_doc - vp.height).max(0.0);
                } else {
                    reader.scroll_target.1 = 0.0;
                }

                let new_width = vp.width * zoom;
                let max_x = (new_width - vp.width).max(0.0);
                if max_x > 0.0 && old_display.0 > 0.0 {
                    let old_left = (vp.width - old_display.0) / 2.0 - reader.off.0;
                    let fraction_x = (anchor.0 - old_left) / old_display.0;
                    let new_left = anchor.0 - fraction_x * new_width;
                    let target_x = (vp.width - new_width) / 2.0 - new_left;
                    reader.scroll_target.0 = target_x.clamp(0.0, max_x);
                } else {
                    reader.scroll_target.0 = 0.0;
                }

                reader.off = reader.scroll_target;
                reader.cancel_transition();
                format!("continuous · {}%", (zoom * 100.0).round() as i32)
            } else {
                let Some(orig) = reader.current().map(Reader::orig_of) else {
                    return;
                };
                let zoom = layout::clamp_zoom(zoom);
                let old_origin = layout::origin(reader.off, old_display, &vp);
                let new_display = layout::display_size(reader.fit, zoom, orig, &vp);
                let off = layout::zoom_anchor_offset(old_origin, old_display, new_display, anchor);
                reader.cancel_transition();
                reader.zoom = zoom;
                reader.set_scroll(layout::clamp_offset(off, new_display, &vp));
                format!(
                    "{} · {}%",
                    reader.fit.label(),
                    (zoom * 100.0).round() as i32
                )
            }
        };
        if self.library.settings().remember_zoom
            && let Some(reader) = self.reader.as_ref()
        {
            self.library.settings_mut().zoom = reader.zoom;
            self.save_library_debounced(cx);
        }
        self.show_hud(message, cx);
    }

    fn zoom_by(&mut self, factor: f32, anchor: (f32, f32), vp: Viewport, cx: &mut Context<Self>) {
        let Some(zoom) = self.reader.as_ref().map(|r| r.zoom) else {
            return;
        };
        self.zoom_to(zoom * factor, anchor, vp, cx);
    }

    fn set_fit(&mut self, mode: FitMode, cx: &mut Context<Self>) {
        if let Some(reader) = self.reader.as_mut() {
            reader.cancel_transition();
            reader.fit = mode;
            reader.zoom = 1.0;
            reader.arrive = Some(Arrive::Start);
            self.show_hud(mode.label(), cx);
        }
        self.library.settings_mut().fit = mode;
        if self.library.settings().remember_zoom {
            self.library.settings_mut().zoom = 1.0;
        }
        self.save_library();
        cx.notify();
    }

    #[allow(dead_code)]
    fn cycle_fit(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = self.reader.as_ref().map(|r| r.fit.next()) {
            self.set_fit(next, cx);
        } else {
            let next = self.library.settings().fit.next();
            self.set_fit(next, cx);
        }
    }

    fn set_remember_zoom(&mut self, value: Option<bool>, cx: &mut Context<Self>) {
        let next = value.unwrap_or(!self.library.settings().remember_zoom);
        self.library.settings_mut().remember_zoom = next;
        if next && let Some(reader) = self.reader.as_ref() {
            self.library.settings_mut().zoom = reader.zoom;
        }
        self.save_library();
        self.show_hud(
            if next {
                "remember zoom on"
            } else {
                "remember zoom off"
            },
            cx,
        );
        cx.notify();
    }

    #[allow(dead_code)]
    fn reset_zoom(&mut self, cx: &mut Context<Self>) {
        self.library.settings_mut().zoom = 1.0;
        if let Some(reader) = self.reader.as_mut() {
            reader.zoom = 1.0;
            reader.cancel_transition();
            reader.arrive = Some(Arrive::Start);
        }
        self.save_library();
        self.show_hud("zoom reset (100%)", cx);
        cx.notify();
    }

    fn set_animations(&mut self, value: Option<bool>, cx: &mut Context<Self>) {
        let enabled = value.unwrap_or(!self.library.settings().animate);
        self.library.settings_mut().animate = enabled;
        self.library.settings_mut().disable_animations = !enabled;
        self.save_library();
        let message = match (enabled, self.reduce_motion) {
            (true, true) => "animations on (the desktop asks for reduced motion, so they stay off)",
            (true, false) => "animations on",
            (false, _) => "animations off",
        };
        self.show_hud(message, cx);
    }

    fn toggle_disable_animations(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.library.settings_mut().disable_animations = disabled;
        self.library.settings_mut().animate = !disabled;
        self.save_library();
        let message = match (!disabled, self.reduce_motion) {
            (true, true) => "animations on (the desktop asks for reduced motion, so they stay off)",
            (true, false) => "animations on",
            (false, _) => "animations off",
        };
        self.show_hud(message, cx);
        cx.notify();
    }

    fn set_continuous(&mut self, value: Option<bool>, cx: &mut Context<Self>) {
        let next = value.unwrap_or(!self.library.settings().continuous);
        self.library.settings_mut().continuous = next;
        if let Some(reader) = self.reader.as_mut() {
            reader.continuous = next || reader.webcomics;
            reader.zoom = 1.0;
            reader.off = (0.0, 0.0);
            reader.scroll_target = (0.0, 0.0);
            reader.arrive = Some(Arrive::Start);
            reader.cancel_transition();
        }
        if self.library.settings().remember_zoom {
            self.library.settings_mut().zoom = 1.0;
        }
        self.save_library();
        let webcomics = self.library.settings().webcomics;
        self.show_hud(
            if webcomics {
                "webcomics mode"
            } else if next {
                "continuous scrolling"
            } else {
                "single page"
            },
            cx,
        );
    }

    fn set_webcomics(&mut self, value: Option<bool>, cx: &mut Context<Self>) {
        let next = value.unwrap_or(!self.library.settings().webcomics);
        self.library.settings_mut().webcomics = next;
        if let Some(reader) = self.reader.as_mut() {
            reader.webcomics = next;
            reader.continuous = next || self.library.settings().continuous;
            reader.zoom = 1.0;
            reader.off = (0.0, 0.0);
            reader.scroll_target = (0.0, 0.0);
            reader.arrive = Some(Arrive::Start);
            reader.cancel_transition();
        }
        if self.library.settings().remember_zoom {
            self.library.settings_mut().zoom = 1.0;
        }
        self.save_library();
        self.show_hud(
            if next {
                "webcomics mode"
            } else if self.library.settings().continuous {
                "continuous scrolling"
            } else {
                "single page"
            },
            cx,
        );
    }

    fn on_wheel(&mut self, params: WheelParams, cx: &mut Context<Self>) {
        let WheelParams {
            delta,
            zoom,
            at,
            discrete,
            touch_phase,
            vp,
        } = params;
        let (dx, dy) = delta;
        let animations_enabled = self.animations_enabled();
        let Some(reader) = self.reader.as_mut() else {
            return;
        };

        let now = Instant::now();
        if touch_phase == TouchPhase::Started
            || touch_phase == TouchPhase::Cancelled
            || discrete
            || reader.kinetic.has_active_inertia()
        {
            reader.kinetic.reset();
        }

        let Some(display) = reader.settle(&vp) else {
            return;
        };

        if zoom {
            reader.kinetic.cancel_inertia();
            // Wheel up (positive) zooms in, anchored at the pointer.
            self.zoom_by(1.2f32.powf(dy / WHEEL_LINE), at, vp, cx);
            return;
        }

        let max = if reader.continuous {
            let doc_height = reader.continuous_total_height(&vp);
            (
                (display.0 - vp.width).max(0.0),
                (doc_height - vp.height).max(0.0),
            )
        } else {
            layout::max_offset(display, &vp)
        };

        if !discrete
            && (dx.abs() > 0.001 || dy.abs() > 0.001)
            && touch_phase != TouchPhase::Cancelled
        {
            // Delta displacement in offset space is (-dx, -dy)
            reader.kinetic.push_motion(now, -dx, -dy);
        }

        if touch_phase == TouchPhase::Ended
            && reader
                .kinetic
                .end_touch(reader.animate && animations_enabled)
        {
            self.wake();
            cx.notify();
            return;
        }

        if reader.continuous {
            let target = (reader.scroll_target.1 - dy).clamp(0.0, max.1);
            reader.scroll_target.1 = target;
            if !discrete || !reader.animate {
                reader.off.1 = target;
            }
        } else if max.1 > EDGE_EPS {
            // Wheel down gives a negative dy and should move further down the page.
            let at_bottom = reader.scroll_target.1 >= max.1 - EDGE_EPS;
            let at_top = reader.scroll_target.1 <= EDGE_EPS;
            if discrete && ((dy < 0.0 && at_bottom) || (dy > 0.0 && at_top)) {
                let forward = dy < 0.0;
                let arrive = if forward { Arrive::Start } else { Arrive::End };
                self.turn_by(forward, 1, arrive, cx);
                return;
            }
            let target = (reader.scroll_target.1 - dy).clamp(0.0, max.1);
            reader.scroll_target.1 = target;
            if !discrete {
                // Touchpads already send a smooth stream of small deltas: follow them 1:1.
                reader.off.1 = target;
            }
        } else if discrete && dy != 0.0 {
            let forward = dy < 0.0;
            let arrive = if forward { Arrive::Start } else { Arrive::End };
            self.turn_by(forward, 1, arrive, cx);
            return;
        }
        if max.0 > EDGE_EPS {
            let target = (reader.scroll_target.0 - dx).clamp(0.0, max.0);
            reader.scroll_target.0 = target;
            if !discrete {
                reader.off.0 = target;
            }
        }
        self.wake();
        cx.notify();
    }

    fn on_pinch(
        &mut self,
        at: (f32, f32),
        delta: f32,
        phase: TouchPhase,
        vp: Viewport,
        cx: &mut Context<Self>,
    ) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };

        if phase == TouchPhase::Started || phase == TouchPhase::Cancelled {
            reader.kinetic.reset();
            return;
        }

        if phase == TouchPhase::Ended {
            return;
        }

        if delta.abs() > 1e-5 {
            reader.kinetic.cancel_inertia();
            let factor = (1.0 + delta).clamp(0.2, 5.0);
            self.zoom_by(factor, at, vp, cx);
        }
    }

    #[allow(dead_code)]
    fn on_library_pinch(&mut self, delta: f32, phase: TouchPhase, cx: &mut Context<Self>) {
        if phase == TouchPhase::Started || phase == TouchPhase::Cancelled {
            self.library_pinch_accum = 0.0;
            return;
        }
        if phase == TouchPhase::Ended {
            self.library_pinch_accum = 0.0;
            return;
        }
        self.library_pinch_accum += delta;
        let current = self.library.settings().thumbnail_size;
        if self.library_pinch_accum > 0.35 {
            self.library_pinch_accum = 0.0;
            let next = match current {
                ThumbnailSize::Small => Some(ThumbnailSize::Medium),
                ThumbnailSize::Medium => Some(ThumbnailSize::Large),
                ThumbnailSize::Large => None,
            };
            if let Some(new_size) = next {
                self.library.settings_mut().thumbnail_size = new_size;
                self.save_library();
                self.show_toast(format!("Thumbnails: {}", new_size.label()), cx);
                cx.notify();
            }
        } else if self.library_pinch_accum < -0.35 {
            self.library_pinch_accum = 0.0;
            let prev = match current {
                ThumbnailSize::Large => Some(ThumbnailSize::Medium),
                ThumbnailSize::Medium => Some(ThumbnailSize::Small),
                ThumbnailSize::Small => None,
            };
            if let Some(new_size) = prev {
                self.library.settings_mut().thumbnail_size = new_size;
                self.save_library();
                self.show_toast(format!("Thumbnails: {}", new_size.label()), cx);
                cx.notify();
            }
        }
    }

    fn drag_start(&mut self, at: (f32, f32)) {
        // Prevent click/drag interaction when mouse is over the top bar
        if at.1 <= 56.0 {
            return;
        }
        if let Some(reader) = self.reader.as_mut() {
            reader.kinetic.reset();
            let off0 = reader.off;
            reader.drag = Some(Drag {
                start: at,
                off0,
                moved: false,
                last_at: at,
            });
        }
    }

    fn drag_move(&mut self, at: (f32, f32), pressed: bool, vp: Viewport, cx: &mut Context<Self>) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let Some(drag) = reader.drag else {
            return;
        };
        if !pressed {
            reader.drag = None;
            return;
        }
        let (dx, dy) = (at.0 - drag.start.0, at.1 - drag.start.1);
        if !drag.moved && dx.abs().max(dy.abs()) < DRAG_THRESHOLD {
            return;
        }
        let Some(display) = reader.settle(&vp) else {
            return;
        };
        let now = Instant::now();
        let (step_x, step_y) = (at.0 - drag.last_at.0, at.1 - drag.last_at.1);
        reader.kinetic.push_motion(now, -step_x, -step_y);

        reader.drag = Some(Drag {
            moved: true,
            last_at: at,
            ..drag
        });
        let target = (drag.off0.0 - dx, drag.off0.1 - dy);
        if reader.continuous {
            let doc_height = reader.continuous_total_height(&vp);
            let max = (
                (display.0 - vp.width).max(0.0),
                (doc_height - vp.height).max(0.0),
            );
            reader.set_scroll((target.0.clamp(0.0, max.0), target.1.clamp(0.0, max.1)));
        } else {
            reader.set_scroll(layout::clamp_offset(target, display, &vp));
        }
        cx.notify();
    }

    /// Mouse released: if it was a drag flick, launch inertial glide; otherwise click.
    fn drag_end(&mut self, x: f32, vp: Viewport, cx: &mut Context<Self>) {
        let animations_enabled = self.animations_enabled();
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let Some(drag) = reader.drag.take() else {
            return;
        };
        if drag.moved {
            if reader
                .kinetic
                .end_touch(reader.animate && animations_enabled)
            {
                self.wake();
                cx.notify();
            }
            return;
        }
        if x < vp.width / 3.0 {
            self.step(Side::Left, cx);
        } else if x > vp.width * 2.0 / 3.0 {
            self.step(Side::Right, cx);
        }
    }

    // ------------------------------------------------------------------------------------
    // Keyboard
    // ------------------------------------------------------------------------------------

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let mods = &keystroke.modifiers;
        if mods.platform && !cfg!(target_os = "macos") {
            return;
        }
        let primary_ctrl = mods.control || (cfg!(target_os = "macos") && mods.platform);
        let token = vim::key_token(
            &keystroke.key,
            keystroke.key_char.as_deref(),
            primary_ctrl,
            mods.alt,
            mods.shift,
        );

        // An open menu or dialog gets every key; nothing leaks through.
        if self.overlay.is_some() {
            if matches!(self.overlay, Some(Overlay::Series(_))) {
                if token == "escape" {
                    self.close_overlay(cx);
                }
                return;
            }
            self.on_overlay_key(&token, window, cx);
            return;
        }

        // Active command line prompt (works in both reader and library)
        if self.in_cmdline() {
            let draft = self.cmdline_text().map(str::to_owned).unwrap_or_default();
            if matches!(token.as_str(), "up" | "down" | "tab" | "S-tab") {
                let suggestions = command_suggestions(&draft, &self.custom_themes);
                if !suggestions.is_empty() {
                    let count = suggestions.len();
                    let backwards = token == "up" || token == "S-tab";
                    self.command_selection = if backwards {
                        self.command_selection.wrapping_add(count).wrapping_sub(1) % count
                    } else {
                        (self.command_selection + 1) % count
                    };
                    if token == "tab" || token == "S-tab" {
                        self.replace_cmdline(suggestions[self.command_selection].0.clone());
                        self.command_selection = 0;
                    }
                    self.sync_theme_preview(&draft, cx);
                }
                cx.notify();
                return;
            }

            if token == "escape" {
                self.cancel_cmdline(cx);
                return;
            }

            if token == "enter" {
                let suggestions = command_suggestions(&draft, &self.custom_themes);
                if !suggestions.is_empty() && self.command_selection < suggestions.len() {
                    self.replace_cmdline(suggestions[self.command_selection].0.clone());
                }
            }

            let vp = viewport_of(window);
            let actions = if let Some(reader) = self.reader.as_mut() {
                reader.vim.feed(&token)
            } else {
                self.library_vim.feed(&token)
            };
            self.command_selection = 0;
            if let Some(new_draft) = self.cmdline_text().map(str::to_owned) {
                self.sync_theme_preview(&new_draft, cx);
            } else {
                let trimmed = draft.trim_start();
                let words = trimmed.split_whitespace();
                let first_word = words.into_iter().next().unwrap_or_default();
                if !matches!(first_word, "theme" | "colorscheme" | "colo" | "colors") {
                    self.restore_theme_preview(cx);
                }
            }
            cx.notify();
            for action in actions {
                self.apply(action, vp, window, cx);
            }
            return;
        }

        let vim_enabled = self.is_vim_enabled();
        let Some(reader) = self.reader.as_mut() else {
            if vim_enabled && token == ":" && !self.search_active {
                self.library_vim.feed(":");
                self.command_selection = 0;
                cx.notify();
                return;
            }
            self.on_library_key(
                &token,
                keystroke.key_char.as_deref(),
                mods.control,
                mods.alt,
                window,
                cx,
            );
            return;
        };

        if reader.show_help && matches!(token.as_str(), "escape" | "?" | "q") {
            reader.show_help = false;
            cx.notify();
            return;
        }

        let vp = viewport_of(window);
        if vim_enabled {
            let actions = reader.vim.feed(&token);
            self.command_selection = 0;
            cx.notify();
            for action in actions {
                self.apply(action, vp, window, cx);
            }
        } else {
            let center = (vp.width / 2.0, vp.height / 2.0);
            let anchor = self.cursor_pos.unwrap_or(center);
            match token.as_str() {
                "escape" => {
                    self.close_book(window, cx);
                }
                "?" => {
                    reader.show_help = true;
                    cx.notify();
                }
                "right" | "space" => {
                    self.turn_by(true, 1, Arrive::Start, cx);
                }
                "left" | "S-space" => {
                    self.turn_by(false, 1, Arrive::Start, cx);
                }
                "down" => {
                    self.scroll(Dir::Down, Amount::Line, 1, vp, cx);
                }
                "up" => {
                    self.scroll(Dir::Up, Amount::Line, 1, vp, cx);
                }
                "pagedown" => {
                    self.scroll(Dir::Down, Amount::Page, 1, vp, cx);
                }
                "pageup" => {
                    self.scroll(Dir::Up, Amount::Page, 1, vp, cx);
                }
                "home" => {
                    self.jump_to(0, cx);
                }
                "end" => {
                    self.jump_to(usize::MAX, cx);
                }
                "+" | "=" | "C-=" | "C-plus" => {
                    self.zoom_by(layout::ZOOM_STEP, anchor, vp, cx);
                }
                "-" | "C-minus" => {
                    self.zoom_by(layout::ZOOM_STEP.recip(), anchor, vp, cx);
                }
                "0" | "C-0" => {
                    self.zoom_to(1.0, anchor, vp, cx);
                }
                "v" => {
                    let next = !self.library.settings().continuous;
                    self.set_continuous(Some(next), cx);
                }
                "w" => {
                    let next = !self.library.settings().webcomics;
                    self.set_webcomics(Some(next), cx);
                }
                "f" => {
                    let next = self
                        .reader
                        .as_ref()
                        .map(|r| r.fit.next())
                        .unwrap_or(FitMode::Page);
                    self.set_fit(next, cx);
                }
                _ => {}
            }
        }
    }

    fn apply(&mut self, action: Action, vp: Viewport, window: &mut Window, cx: &mut Context<Self>) {
        let center = (vp.width / 2.0, vp.height / 2.0);
        let anchor = self.cursor_pos.unwrap_or(center);
        match action {
            Action::Scroll { dir, amount, count } => self.scroll(dir, amount, count, vp, cx),
            Action::NextPage(n) => self.turn_by(true, n, Arrive::Start, cx),
            Action::PrevPage(n) => self.turn_by(false, n, Arrive::Start, cx),
            Action::GotoPage(n) => self.jump_to(n.saturating_sub(1), cx),
            Action::FirstPage => self.jump_to(0, cx),
            Action::LastPage => self.jump_to(usize::MAX, cx),
            Action::ZoomIn(n) => {
                self.zoom_by(layout::ZOOM_STEP.powi(n.min(64) as i32), anchor, vp, cx)
            }
            Action::ZoomOut(n) => {
                self.zoom_by(layout::ZOOM_STEP.powi(-(n.min(64) as i32)), anchor, vp, cx)
            }
            Action::ZoomReset => self.zoom_to(1.0, anchor, vp, cx),
            Action::ZoomSet(zoom) => self.zoom_to(zoom, anchor, vp, cx),
            Action::Fit(mode) => self.set_fit(mode, cx),
            Action::CycleFit => {
                if let Some(next) = self.reader.as_ref().map(|r| r.fit.next()) {
                    self.set_fit(next, cx);
                } else {
                    let next = self.library.settings().fit.next();
                    self.set_fit(next, cx);
                }
            }
            Action::ScrollTop => self.scroll_vertical_to(0.0, vp, cx),
            Action::ScrollCenter => self.scroll_vertical_to(0.5, vp, cx),
            Action::ScrollBottom => self.scroll_vertical_to(1.0, vp, cx),
            Action::SetMark(key) => {
                let Some((id, page)) = self.reader.as_ref().map(|r| (r.book_id.clone(), r.page))
                else {
                    return;
                };
                if let Some(book) = self.library.get_mut(&id) {
                    book.marks.insert(key.to_string(), page);
                }
                self.save_library();
                self.show_hud(format!("Mark {key} set on page {}", page + 1), cx);
            }
            Action::JumpMark(key) => {
                let Some(id) = self.reader.as_ref().map(|r| r.book_id.clone()) else {
                    return;
                };
                let target = self
                    .library
                    .get(&id)
                    .and_then(|book| book.marks.get(&key.to_string()).copied());
                match target {
                    Some(page) => self.jump_to(page, cx),
                    None => self.show_hud(format!("No mark {key}"), cx),
                }
            }
            Action::JumpBack => {
                let Some(reader) = self.reader.as_mut() else {
                    return;
                };
                match reader.back_page {
                    Some(previous) => {
                        reader.back_page = Some(reader.page);
                        self.goto_page(previous, Arrive::Start, cx);
                    }
                    None => self.show_hud("No previous position", cx),
                }
            }
            Action::SetAnimations(value) => self.set_animations(value, cx),
            Action::SetContinuous(value) => self.set_continuous(value, cx),
            Action::SetWebcomics(value) => self.set_webcomics(value, cx),
            Action::SetRememberZoom(value) => self.set_remember_zoom(value, cx),
            Action::ToggleInfo => {
                if let Some(reader) = self.reader.as_mut() {
                    reader.show_info = !reader.show_info;
                    cx.notify();
                }
            }
            Action::ToggleHelp => {
                if let Some(reader) = self.reader.as_mut() {
                    reader.show_help = !reader.show_help;
                    cx.notify();
                } else {
                    self.settings_tab = SettingsTab::Shortcuts;
                    self.open_settings(cx);
                }
            }
            Action::ToggleFullscreen => window.toggle_fullscreen(),
            Action::OpenSettings => self.open_settings_window(window, cx),
            Action::CloseBook => {
                if self.reader.is_some() {
                    self.close_book(window, cx);
                } else if self.open_group.is_some() {
                    self.exit_group(cx);
                } else if !self.selection.is_empty() {
                    self.selection.clear();
                    cx.notify();
                }
            }
            Action::QuitApp => {
                self.save_library();
                cx.quit();
            }
            Action::SetTheme(name) => {
                let name = name.trim();
                if name.is_empty() {
                    self.cycle_theme(1, cx);
                    self.show_toast(format!("Theme: {}", self.library.settings().theme_name), cx);
                } else {
                    let mut matched = self
                        .custom_themes
                        .keys()
                        .find(|k| k.eq_ignore_ascii_case(name))
                        .cloned();
                    if matched.is_none() {
                        matched = BUILT_IN_THEMES
                            .keys()
                            .find(|k| k.eq_ignore_ascii_case(name))
                            .cloned();
                    }
                    if matched.is_none() {
                        let lower = name.to_lowercase();
                        matched = self
                            .custom_themes
                            .keys()
                            .find(|k| k.to_lowercase().contains(&lower))
                            .or_else(|| {
                                BUILT_IN_THEMES
                                    .keys()
                                    .find(|k| k.to_lowercase().contains(&lower))
                            })
                            .cloned();
                    }
                    if let Some(theme_name) = matched {
                        self.apply_theme(&theme_name, cx);
                        self.show_toast(format!("Theme: {theme_name}"), cx);
                    } else {
                        self.show_toast(format!("Theme “{name}” is unavailable"), cx);
                    }
                }
                self.theme_before_preview = None;
            }
            Action::Message(text) => self.show_hud(text, cx),
        }
    }

    // ------------------------------------------------------------------------------------
    // Page decoding
    // ------------------------------------------------------------------------------------

    /// Called every render: settles the scroll position, frees far-away pages and decodes the
    /// current and next pages at the resolution the current zoom needs.
    fn sync_reader(&mut self, window: &mut Window, cx: &mut Context<Self>, dt: f32) -> bool {
        let vp = viewport_of(window);
        let animate = self.animations_enabled();

        let ReaderSyncOutcome {
            wanted,
            animating,
            progress_changed,
        } = {
            let Some(reader) = self.reader.as_mut() else {
                return false;
            };
            reader.viewport = vp;
            reader.animate = animate;
            let Some(pages) = reader.pages.clone() else {
                return false;
            };
            let previous_page = reader.page;
            reader.settle(&vp);
            let animating = reader.advance_animations(dt, &vp);
            if reader.continuous {
                reader.settle(&vp);
            }
            let progress_changed = if reader.continuous && reader.page != previous_page {
                let id = reader.book_id.clone();
                let page = reader.page;
                if let Some(book) = self.library.get_mut(&id) {
                    book.current_page = page;
                    book.opened = true;
                    book.last_read = Some(now_secs());
                }
                Some((id, page))
            } else {
                None
            };

            // Keep a bounded window of decoded pages around the current one, and stay under the
            // memory budget even with very large pages.
            let entries: Vec<pagecache::Entry> = reader
                .images
                .iter()
                .map(|(index, page)| pagecache::Entry {
                    index: *index,
                    bytes: page.bytes,
                })
                .collect();
            let keep: HashSet<usize> = if reader.continuous {
                let low = reader.off.1;
                let high = low + vp.height;
                let (first, last) = reader.continuous_visible_range(low, high, &vp);
                (first.saturating_sub(1)..=(last + 2).min(pages.len().saturating_sub(1))).collect()
            } else {
                (reader.page.saturating_sub(pagecache::KEEP_BEHIND)
                    ..=reader.page + pagecache::KEEP_AHEAD)
                    .collect()
            };
            for ix in pagecache::plan_eviction_with_keep(
                &entries,
                reader.page,
                pagecache::system_memory_budget(),
                &keep,
            ) {
                if let Some(page) = reader.images.remove(&ix) {
                    // A page that is still sliding out is freed when its slide ends instead.
                    let leaving = reader
                        .outgoing
                        .as_ref()
                        .is_some_and(|outgoing| Arc::ptr_eq(&outgoing.image, &page.image));
                    if !leaving {
                        let _ = window.drop_image(page.image);
                    }
                }
            }

            // When zoom is retained, neighbours are pre-fetched at the reader's active zoom
            // so turning the page stays sharp without a secondary re-decode.
            let long_side = |display: (f32, f32)| display.0.max(display.1) * vp.scale;
            let (current_tier, neighbour_tier) = match reader.current().map(Reader::orig_of) {
                Some(orig) => {
                    let tier = layout::tier_for(long_side(layout::display_size(
                        reader.fit,
                        reader.zoom,
                        orig,
                        &vp,
                    )));
                    (tier, tier)
                }
                None => {
                    let tier = layout::tier_for(vp.width.max(vp.height) * vp.scale * reader.zoom);
                    (tier, tier)
                }
            };

            let wanted_indices: Vec<usize> = if reader.continuous {
                keep.iter()
                    .copied()
                    .filter(|ix| *ix < pages.len())
                    .collect()
            } else {
                pagecache::prefetch_order(reader.page, pages.len())
            };
            let wanted = wanted_indices
                .into_iter()
                .map(|ix| {
                    let tier = if reader.continuous {
                        let width = vp.width * reader.zoom * vp.scale;
                        let (w, h) = reader.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
                        layout::tier_for(width.max(width * h as f32 / w.max(1) as f32))
                    } else if ix == reader.page {
                        current_tier
                    } else {
                        neighbour_tier
                    };
                    (ix, tier)
                })
                .collect();
            ReaderSyncOutcome {
                wanted,
                animating,
                progress_changed,
            }
        };

        if progress_changed.is_some() {
            self.save_library_debounced(cx);
        }

        // Textures released by finished slides are freed on the next frame.
        if let Some(reader) = self.reader.as_mut() {
            self.garbage.append(&mut reader.released);
            // Cancel background prefetch tasks for pages that are no longer wanted:
            let wanted_set: HashSet<usize> = wanted.iter().map(|(ix, _)| *ix).collect();
            reader.cancellations.retain(|ix, token| {
                if !wanted_set.contains(ix) {
                    token.store(true, std::sync::atomic::Ordering::Relaxed);
                    false
                } else {
                    true
                }
            });
        }
        for (ix, tier) in wanted {
            self.request_page(ix, tier, cx);
        }
        animating
    }

    fn request_page(&mut self, ix: usize, tier: u32, cx: &mut Context<Self>) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let Some(pages) = reader.pages.clone() else {
            return;
        };
        if ix >= pages.len() || reader.pending.contains(&ix) || reader.failed.contains_key(&ix) {
            return;
        }
        if matches!(reader.images.get(&ix), Some(page) if page.tier >= tier) {
            return;
        }
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        reader.pending.insert(ix);
        reader.cancellations.insert(ix, cancel.clone());

        let book_id = reader.book_id.clone();
        let path = reader.path.clone();
        let kind = reader.kind;
        let name = pages[ix].clone();
        let cancel_token = cancel.clone();

        let task = cx.background_executor().spawn(async move {
            if cancel_token.load(std::sync::atomic::Ordering::Relaxed) {
                anyhow::bail!("page decode cancelled");
            }
            let bytes = archive::read_page(&path, kind, &name)?;
            if cancel_token.load(std::sync::atomic::Ordering::Relaxed) {
                anyhow::bail!("page decode cancelled");
            }
            let decoded = imaging::decode_fit_bgra(&bytes, tier, tier)?;
            if cancel_token.load(std::sync::atomic::Ordering::Relaxed) {
                anyhow::bail!("page decode cancelled");
            }
            let (width, height) = decoded.image.dimensions();
            anyhow::Ok((
                Arc::new(RenderImage::new(vec![Frame::new(decoded.image)])),
                decoded.orig,
                u64::from(width) * u64::from(height) * 4,
            ))
        });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this
                .update(cx, |this, cx| {
                    this.page_decoded(book_id, ix, tier, result, cx)
                })
                .ok();
        })
        .detach();
    }

    fn page_decoded(
        &mut self,
        book_id: String,
        ix: usize,
        tier: u32,
        result: Result<(Arc<RenderImage>, (u32, u32), u64)>,
        cx: &mut Context<Self>,
    ) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        if reader.book_id != book_id {
            return;
        }
        reader.pending.remove(&ix);
        reader.cancellations.remove(&ix);
        match result {
            Ok((image, orig, bytes)) => {
                if reader.continuous && ix < reader.page {
                    let width = (reader.viewport.width * reader.zoom).max(1.0);
                    let (old_w, old_h) =
                        reader.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
                    let old_height = width * old_h as f32 / old_w.max(1) as f32;
                    let new_height = width * orig.1 as f32 / orig.0.max(1) as f32;
                    let shift = new_height - old_height;
                    reader.off.1 = (reader.off.1 + shift).max(0.0);
                    reader.scroll_target.1 = (reader.scroll_target.1 + shift).max(0.0);
                }
                if let Some(size) = reader.orig_sizes.get_mut(ix) {
                    *size = Some(orig);
                }
                if let Some(old) = reader.images.insert(
                    ix,
                    Page {
                        image,
                        orig,
                        tier,
                        bytes,
                    },
                ) {
                    self.garbage.push(old.image);
                }
            }
            Err(err) => {
                reader.failed.insert(ix, format!("{err:#}"));
            }
        }
        cx.notify();
    }

    // ------------------------------------------------------------------------------------
    // Rendering
    // ------------------------------------------------------------------------------------

    fn render_overlay(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(match self.overlay.as_ref()? {
            Overlay::Menu(menu) => self.render_menu(menu, cx),
            Overlay::LibraryMenu(menu) => self.render_library_menu(menu, cx),
            Overlay::Confirm(dialog) => self.render_confirm(dialog, cx),
            Overlay::Properties(dialog) => self.render_properties(dialog, cx),
            Overlay::Series(dialog) => self.render_series_dialog(dialog, cx),
            Overlay::Collection(dialog) => self.render_collection_dialog(dialog, cx),
            Overlay::Settings => self.render_settings(window, cx),
        })
    }

    fn overlay_style(&self, surface_bg: gpui_kit::Hsla) -> ui::overlays::OverlayStyle {
        ui::overlays::OverlayStyle {
            colors: self.colors,
            border_px: self.border_px(),
            radius_px: self.radius_px(),
            rounding_lg: self.radius_lg_px(),
            rounding_xl: self.radius_xl_px(),
            surface_bg,
            animations_enabled: self.animations_enabled(),
        }
    }

    fn render_menu(&self, menu: &BookMenu, cx: &mut Context<Self>) -> AnyElement {
        let style = self.overlay_style(self.menu_surface());
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.close_overlay(cx));
        };

        let yomu_hover = yomu.clone();
        let on_hover = move |ix: usize, _window: &mut Window, cx: &mut App| {
            yomu_hover.update(cx, |this, cx| this.menu_hover(ix, cx));
        };

        let yomu_action = yomu.clone();
        let on_action = move |action: BookAction,
                              book_id: &str,
                              target_ids: &[String],
                              window: &mut Window,
                              cx: &mut App| {
            let book_id = book_id.to_string();
            let target_ids = target_ids.to_vec();
            yomu_action.update(cx, |this, cx| {
                this.run_book_action(action, &book_id, &target_ids, window, cx);
            });
        };

        ui::overlays::render_book_menu(menu, style, on_close, on_hover, on_action)
    }

    fn render_library_menu(&self, menu: &LibraryMenu, cx: &mut Context<Self>) -> AnyElement {
        let style = self.overlay_style(self.menu_surface());
        let group_series = self.library.settings().group_series;
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.close_overlay(cx));
        };

        let yomu_hover = yomu.clone();
        let on_hover = move |ix: usize, _window: &mut Window, cx: &mut App| {
            yomu_hover.update(cx, |this, cx| {
                if let Some(Overlay::LibraryMenu(menu)) = this.overlay.as_mut() {
                    menu.selected = ix;
                    cx.notify();
                }
            });
        };

        let yomu_action = yomu.clone();
        let on_action = move |ix: usize, window: &mut Window, cx: &mut App| {
            yomu_action.update(cx, |this, cx| {
                this.run_library_menu_action(ix, window, cx);
            });
        };

        ui::overlays::render_library_menu(menu, group_series, style, on_close, on_hover, on_action)
    }

    fn render_confirm(&self, dialog: &ConfirmDialog, cx: &mut Context<Self>) -> AnyElement {
        let style = self.overlay_style(self.widget_surface(self.colors.surface));
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.close_overlay(cx));
        };

        let yomu_accept = yomu.clone();
        let on_accept = move |window: &mut Window, cx: &mut App| {
            yomu_accept.update(cx, |this, cx| {
                this.confirm_accept(window, cx);
            });
        };

        ui::overlays::render_confirm_dialog(dialog, style, on_close, on_accept)
    }

    fn render_properties(&self, dialog: &PropertiesDialog, cx: &mut Context<Self>) -> AnyElement {
        let style = self.overlay_style(self.widget_surface(self.colors.surface));
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.close_overlay(cx));
        };

        ui::overlays::render_properties_dialog(dialog, style, on_close)
    }

    fn render_series_dialog(&self, dialog: &SeriesDialog, cx: &mut Context<Self>) -> AnyElement {
        let style = self.overlay_style(self.widget_surface(self.colors.surface));
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.close_overlay(cx));
        };

        let yomu_save = yomu.clone();
        let on_save = move |_window: &mut Window, cx: &mut App| {
            yomu_save.update(cx, |this, cx| this.save_series_dialog(cx));
        };

        ui::overlays::render_series_dialog(dialog, style, on_close, on_save)
    }

    fn render_collection_dialog(
        &self,
        dialog: &ui::overlays::CollectionDialog,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let style = self.overlay_style(self.colors.background);
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.close_overlay(cx));
        };

        let yomu_save = yomu.clone();
        let on_save =
            move |name: &str, editing_id: Option<&str>, _window: &mut Window, cx: &mut App| {
                let name = name.to_string();
                let editing_id = editing_id.map(|s| s.to_string());
                yomu_save.update(cx, |this, cx| {
                    if let Some(id) = &editing_id {
                        this.library.rename_collection(id, &name);
                    } else {
                        this.library.add_collection(&name);
                    }
                    this.save_library();
                    this.close_overlay(cx);
                });
            };

        ui::overlays::render_collection_dialog(dialog, style, on_close, on_save)
    }

    fn render_settings(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let border_px = self.border_px();
        let cache_bytes = self.dirs.cache_size_bytes();
        let yomu = cx.entity();

        let is_client_decorated = cfg!(target_os = "linux")
            && matches!(window.window_decorations(), Decorations::Client { .. });

        let props = ui::settings_view::SettingsViewProps {
            scroll_handle: &self.settings_scroll,
            current_tab: self.settings_tab,
            settings: self.library.settings(),
            available_schemes: ui::theme::available_color_schemes(&self.custom_themes),
            cache_size_str: format_size(cache_bytes),
            is_client_decorated,
            system_reduced_motion: self.reduce_motion,
            vim_mode: self.is_vim_enabled(),
            colors: self.colors,
            radius: px(self.radius_xl_px()),
        };

        let view = ui::settings_view::render_settings_view(
            props,
            {
                let yomu = yomu.clone();
                move |tab, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.settings_tab = tab;
                        cx.notify();
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.close_overlay(cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_continuous(Some(checked), cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_webcomics(Some(checked), cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.toggle_group_series(checked, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.toggle_disable_animations(checked, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.library.settings_mut().remember_zoom = checked;
                        this.save_library();
                        cx.notify();
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.toggle_vim_mode(checked, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |mode, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_theme_mode(mode, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |scheme, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_color_scheme(&scheme, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |size, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_thumbnail_size(size, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |rounding, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_corner_rounding(rounding, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |border, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_border_size(border, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.request_clear_thumbnail_cache(cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.request_reset_settings(cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.request_clear_all_data(cx);
                    });
                }
            },
        );

        let yomu_scrim = yomu.clone();
        ui::overlays::scrim(true, self.colors, move |_, cx| {
            yomu_scrim.update(cx, |this, cx| this.close_overlay(cx));
        })
        .child(
            div()
                .w(px(860.))
                .min_w(px(640.))
                .max_w(relative(0.92))
                .h(px(620.))
                .min_h(px(460.))
                .max_h(relative(0.90))
                .rounded(px(self.radius_xl_px()))
                .when(border_px > 0.0, |this| {
                    this.border(px(border_px)).border_color(self.colors.border)
                })
                .bg(self.colors.background)
                .shadow_xl()
                .overflow_hidden()
                .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                })
                .child(view),
        )
        .into_any_element()
    }

    fn render_two_pane_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors;
        let radius = px(self.radius_px());
        let yomu = cx.entity();

        let auto_collapse = window.viewport_size().width < px(720.);
        let is_collapsed = self.sidebar_collapsed || auto_collapse;

        // 1. Sidebar Props
        let continue_reading_count = self
            .library
            .books()
            .iter()
            .filter(|b| b.has_progress())
            .count();
        let all_manga_count = self.library.books().len();
        let collections_count = self.library.collections.len() + 4;
        let downloads_count = self
            .library
            .books()
            .iter()
            .filter(|b| b.path.exists())
            .count();

        let sidebar_props = ui::sidebar::SidebarProps {
            scroll_handle: &self.sidebar_scroll,
            current_section: self.active_section,
            continue_reading_count,
            all_manga_count,
            collections_count,
            downloads_count,
            colors,
            radius,
        };

        let sidebar = ui::sidebar::render_sidebar(
            sidebar_props,
            {
                let yomu = yomu.clone();
                move |sec, _, cx| {
                    yomu.update(cx, |this, cx| {
                        let view = match sec {
                            ui::sidebar::SidebarSection::ContinueReading => {
                                ui::sidebar::MainView::ContinueReading
                            }
                            ui::sidebar::SidebarSection::AllManga => {
                                ui::sidebar::MainView::AllManga
                            }
                            ui::sidebar::SidebarSection::Collections => {
                                ui::sidebar::MainView::Collections
                            }
                            ui::sidebar::SidebarSection::History => ui::sidebar::MainView::History,
                            ui::sidebar::SidebarSection::Downloads => {
                                ui::sidebar::MainView::Downloads
                            }
                        };
                        this.navigate_to(view, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.open_settings(cx);
                    });
                }
            },
        );

        // 2. Toolbar Props
        let is_client_decorated = cfg!(target_os = "linux")
            && matches!(window.window_decorations(), Decorations::Client { .. });
        let is_library_view = matches!(
            self.active_view,
            ui::sidebar::MainView::AllManga
                | ui::sidebar::MainView::History
                | ui::sidebar::MainView::Downloads
        );
        let is_browsing =
            is_library_view || matches!(self.active_view, ui::sidebar::MainView::ContinueReading);
        let show_search = is_library_view;
        let show_view_switches = is_library_view;
        let show_grouping = is_library_view;
        let show_import = is_browsing;

        let search_input = self.search_input.as_ref();
        let toolbar_props = ui::toolbar::MainToolbarProps {
            can_go_back: self.open_group.is_some(),
            sidebar_collapsed: is_collapsed,
            is_scrolled: self.is_current_view_scrolled(),
            show_search,
            search_input,
            show_view_switches,
            is_grid_mode: self.view_mode_grid,
            show_grouping,
            is_group_series: self.library.settings().group_series,
            show_import,
            is_client_decorated,
            selection_count: self.selection.len(),
            on_select_all: Some(Rc::new({
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.select_all(cx))
            })),
            on_remove_selected: Some(Rc::new({
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.request_remove_selected(cx))
            })),
            on_cancel_selection: Some(Rc::new({
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.selection.clear();
                        cx.notify();
                    })
                }
            })),
            colors,
            radius,
        };

        let toolbar = ui::toolbar::render_main_toolbar(
            toolbar_props,
            {
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.toggle_sidebar(cx))
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.exit_group(cx))
            },
            {
                let yomu = yomu.clone();
                move |grid, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.view_mode_grid = grid;
                        this.library.settings_mut().view_mode_grid = grid;
                        this.save_library();
                        cx.notify();
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |group, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.toggle_group_series(group, cx);
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.import_dialog(false, cx))
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.import_dialog(true, cx))
            },
            Some(Rc::new(|_, window, _| window.remove_window())),
        );

        // 3. Main Content
        let content: AnyElement = match &self.active_view {
            ui::sidebar::MainView::ContinueReading => self.render_continue_reading_view(window, cx),
            ui::sidebar::MainView::AllManga => self.render_all_manga_view(window, cx),
            ui::sidebar::MainView::Collections => self.render_collections_screen(window, cx),
            ui::sidebar::MainView::History => self.render_history_view(window, cx),
            ui::sidebar::MainView::Downloads => self.render_downloads_view(window, cx),
            ui::sidebar::MainView::Details(book_id) => {
                let id = book_id.clone();
                self.render_details_screen(&id, window, cx)
            }
            ui::sidebar::MainView::Settings => self.render_settings(window, cx),
        };

        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(colors.background)
            .when(!is_collapsed, |s| s.child(sidebar))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(toolbar)
                    .child(
                        div()
                            .id("main-view-content")
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .on_scroll_wheel(cx.listener(
                                |this, event: &ScrollWheelEvent, _, cx| {
                                    let scroll_handle = this.current_scroll_handle();
                                    let animations_enabled = this.animations_enabled();
                                    if this.library_scroll_controller.handle_wheel(
                                        event,
                                        &scroll_handle,
                                        animations_enabled,
                                    ) {
                                        this.wake();
                                        cx.notify();
                                    }
                                },
                            ))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, _| {
                                    this.library_scroll_controller.cancel_inertia();
                                }),
                            )
                            .child(content),
                    ),
            )
            .into_any_element()
    }

    fn render_continue_reading_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let radius = px(self.radius_px());
        let yomu = cx.entity();

        let books = self.library.books();
        let hero_book = books
            .iter()
            .filter(|b| b.has_progress())
            .max_by_key(|b| b.last_read)
            .or_else(|| books.first());

        let hero = hero_book.map(|book| {
            let cover_path = self.dirs.cover(&book.id);
            let display_title = book.title.as_str();
            let total = book.page_count;
            let current = if book.opened {
                book.current_page + 1
            } else {
                0
            };
            let progress = book.progress();
            let chapter_label = format!("Ch. {current} of {total}");
            let genres = book
                .series
                .as_ref()
                .map(|s| vec![s.clone()])
                .unwrap_or_default();

            ui::continue_reading::ContinueReadingItem {
                book,
                cover_path,
                display_title,
                chapter_label,
                progress,
                genres,
            }
        });

        let mut sorted_queue: Vec<&Book> = books.iter().filter(|b| b.has_progress()).collect();
        sorted_queue.sort_by_key(|a| std::cmp::Reverse(a.last_read));

        let queue: Vec<ui::continue_reading::ContinueReadingItem> = sorted_queue
            .iter()
            .skip(1)
            .take(3)
            .map(|book| {
                let cover_path = self.dirs.cover(&book.id);
                let display_title = book.title.as_str();
                let _total = book.page_count;
                let current = if book.opened {
                    book.current_page + 1
                } else {
                    0
                };
                let progress = book.progress();
                let chapter_label = format!("Ch. {current}");

                ui::continue_reading::ContinueReadingItem {
                    book,
                    cover_path,
                    display_title,
                    chapter_label,
                    progress,
                    genres: Vec::new(),
                }
            })
            .collect();

        let thumb_size = self.library.settings().thumbnail_size;
        let sidebar_collapsed = self.is_sidebar_collapsed(window);
        let grid = ui::components::compute_responsive_grid(
            f32::from(window.viewport_size().width),
            !sidebar_collapsed,
            thumb_size,
        );
        let card_w = grid.card_w;
        let cover_h = grid.cover_h;

        let mut cards: Vec<AnyElement> = Vec::new();
        if self.library.settings().group_series {
            let groups = directory_groups(books.iter());
            let mut grouped_ids = HashSet::new();
            for group in &groups {
                for b in &group.books {
                    grouped_ids.insert(b.id.clone());
                }
            }

            enum RecentEntry<'a> {
                Group(&'a LibraryGroup<'a>, u64),
                Book(&'a Book, u64),
            }

            let mut entries: Vec<RecentEntry> = Vec::new();
            for group in &groups {
                let latest_added = group.books.iter().map(|b| b.added_at).max().unwrap_or(0);
                entries.push(RecentEntry::Group(group, latest_added));
            }
            for book in books.iter().filter(|b| !grouped_ids.contains(&b.id)) {
                entries.push(RecentEntry::Book(book, book.added_at));
            }

            entries.sort_by(|a, b| {
                let a_time = match a {
                    RecentEntry::Group(_, t) => *t,
                    RecentEntry::Book(_, t) => *t,
                };
                let b_time = match b {
                    RecentEntry::Group(_, t) => *t,
                    RecentEntry::Book(_, t) => *t,
                };
                b_time.cmp(&a_time)
            });

            for entry in entries.into_iter().take(grid.cols) {
                match entry {
                    RecentEntry::Group(group, _) => {
                        let first = group.books[0];
                        let cover_path = self.dirs.cover_for_size(&first.id, thumb_size);
                        let book_ids: Vec<String> =
                            group.books.iter().map(|b| b.id.clone()).collect();
                        cards.push(self.render_folder_card(
                            &group.key,
                            &group.name,
                            &book_ids,
                            group.books.len(),
                            cover_path,
                            card_w,
                            cover_h,
                            radius,
                            colors,
                            &yomu,
                        ));
                    }
                    RecentEntry::Book(book, _) => {
                        cards.push(
                            self.render_book_card(book, card_w, cover_h, radius, colors, &yomu),
                        );
                    }
                }
            }
        } else {
            let mut sorted_books: Vec<&Book> = books.iter().collect();
            sorted_books.sort_by_key(|a| std::cmp::Reverse(a.added_at));
            for book in sorted_books.into_iter().take(grid.cols) {
                cards.push(self.render_book_card(book, card_w, cover_h, radius, colors, &yomu));
            }
        }

        let props = ui::continue_reading::ContinueReadingProps {
            scroll_handle: &self.continue_reading_scroll,
            hero,
            queue,
            recent_cards: cards,
            recent_gap: px(grid.gap),
            pad_x: px(grid.pad_x),
            colors,
            radius,
            border_px: self.border_px(),
        };

        ui::continue_reading::render_continue_reading_view(
            props,
            {
                let yomu = yomu.clone();
                move |id, window, cx| yomu.update(cx, |this, cx| this.open_book(id, window, cx))
            },
            {
                let yomu = yomu.clone();
                move |id, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.navigate_to(ui::sidebar::MainView::Details(id.to_string()), cx)
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.navigate_to(ui::sidebar::MainView::AllManga, cx)
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |id, pos, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.open_book_menu(id, point(px(pos.0), px(pos.1)), cx)
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.import_dialog(false, cx);
                    });
                }
            },
        )
    }

    fn render_collections_screen(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let radius = px(self.radius_px());
        let yomu = cx.entity();

        let favorites_count = self.library.favorites.len();
        let reading_count = self
            .library
            .books()
            .iter()
            .filter(|b| b.has_progress())
            .count();
        let want_to_read_count = self
            .library
            .books()
            .iter()
            .filter(|b| !b.has_progress())
            .count();
        let completed_count = self
            .library
            .books()
            .iter()
            .filter(|b| b.is_finished())
            .count();

        let props = ui::collections_view::CollectionsProps {
            scroll_handle: &self.collections_scroll,
            favorites_count,
            reading_count,
            want_to_read_count,
            completed_count,
            custom_collections: self.library.collections.as_slice(),
            colors,
            radius,
        };

        ui::collections_view::render_collections_view(
            props,
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.navigate_to(ui::sidebar::MainView::AllManga, cx);
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |id, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.selected_collection_id = Some(id.to_string());
                        this.navigate_to(ui::sidebar::MainView::AllManga, cx);
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |_, window, cx| {
                    yomu.update(cx, |this, cx| {
                        let input = cx
                            .new(|cx| InputState::new(window, cx).placeholder("Collection Name…"));
                        this.overlay = Some(ui::overlays::Overlay::Collection(
                            ui::overlays::CollectionDialog {
                                is_new: true,
                                collection_id: None,
                                input,
                            },
                        ));
                        cx.notify();
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |id, _pos, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.overlay = Some(ui::overlays::Overlay::Confirm(
                            ui::overlays::ConfirmDialog {
                                title: "Delete Collection?".into(),
                                message: "This collection will be deleted. Underlying manga files will not be affected.".into(),
                                confirm_label: "Delete",
                                action: ui::overlays::ConfirmAction::DeleteCollection(id.to_string()),
                            },
                        ));
                        cx.notify();
                    })
                }
            },
        )
    }

    fn render_history_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut history_books: Vec<&Book> = self
            .library
            .books()
            .iter()
            .filter(|b| b.has_progress())
            .collect();
        history_books.sort_by_key(|a| std::cmp::Reverse(a.last_read));

        let sidebar_collapsed = self.is_sidebar_collapsed(window);
        let props = ui::history_view::HistoryProps {
            scroll_handle: &self.library_scroll,
            history_books: &history_books,
            dirs: &self.dirs,
            colors: self.colors,
            radius: px(self.radius_px()),
            border_px: self.border_px(),
            radius_px: self.radius_px(),
            viewport_w: f32::from(window.viewport_size().width),
            sidebar_visible: !sidebar_collapsed,
            is_grid: self.view_mode_grid,
            thumb_size: self.library.settings().thumbnail_size,
        };

        let yomu = cx.entity();
        ui::history_view::render_history_view(props, move |book_id, _, cx| {
            yomu.update(cx, |this, cx| {
                this.navigate_to(ui::sidebar::MainView::Details(book_id), cx);
            });
        })
    }

    fn render_book_card(
        &self,
        book: &Book,
        card_w: f32,
        cover_h: f32,
        radius: Pixels,
        colors: UiColors,
        yomu: &Entity<Self>,
    ) -> AnyElement {
        let book_id = book.id.clone();
        let thumb_size = self.library.settings().thumbnail_size;
        let cover_path = self.dirs.cover_for_size(&book.id, thumb_size);
        let is_selected = self.selection.contains(&book.id);
        let border_px = self.border_px();

        let yomu_click = yomu.clone();
        let id_click = book_id.clone();
        let on_click = move |event: &ClickEvent, window: &mut Window, cx: &mut App| {
            let ctrl = event.modifiers().control || event.modifiers().platform;
            let shift = event.modifiers().shift;
            yomu_click.update(cx, |this, cx| {
                if this.select_click(&id_click, ctrl, shift, cx) {
                    return;
                }
                this.open_book(&id_click, window, cx);
            });
        };

        let yomu_right = yomu.clone();
        let id_right = book_id;
        let on_right_click = move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            yomu_right.update(cx, |this, cx| {
                this.open_context_menu(&id_right, event.position, window, cx);
            });
        };

        render_book_card(
            BookCardProps {
                book,
                cover_path,
                card_w,
                cover_h,
                radius,
                border_px,
                is_selected,
                colors,
            },
            on_click,
            on_right_click,
        )
    }

    fn render_book_list_row(
        &self,
        book: &Book,
        radius: Pixels,
        colors: UiColors,
        yomu: &Entity<Self>,
    ) -> AnyElement {
        let book_id = book.id.clone();
        let thumb_size = self.library.settings().thumbnail_size;
        let cover_path = self.dirs.cover_for_size(&book.id, thumb_size);
        let is_selected = self.selection.contains(&book.id);
        let border_px = self.border_px();

        let yomu_click = yomu.clone();
        let id_click = book_id.clone();
        let on_click = move |event: &ClickEvent, window: &mut Window, cx: &mut App| {
            let ctrl = event.modifiers().control || event.modifiers().platform;
            let shift = event.modifiers().shift;
            yomu_click.update(cx, |this, cx| {
                if this.select_click(&id_click, ctrl, shift, cx) {
                    return;
                }
                this.open_book(&id_click, window, cx);
            });
        };

        let yomu_right = yomu.clone();
        let id_right = book_id;
        let on_right_click = move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            yomu_right.update(cx, |this, cx| {
                this.open_context_menu(&id_right, event.position, window, cx);
            });
        };

        render_book_list_row(
            book,
            cover_path,
            radius,
            border_px,
            is_selected,
            colors,
            on_click,
            on_right_click,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_folder_list_row(
        &self,
        group_key: &str,
        group_name: &str,
        book_ids: &[String],
        count: usize,
        cover_path: PathBuf,
        radius: Pixels,
        colors: UiColors,
        border_px: f32,
        yomu: &Entity<Self>,
    ) -> AnyElement {
        let yomu_click = yomu.clone();
        let group_key_click = group_key.to_string();
        let on_click = move |_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
            let key = group_key_click.clone();
            yomu_click.update(cx, |this, cx| {
                this.enter_group(key, cx);
            });
        };

        let yomu_right = yomu.clone();
        let group_key_right = group_key.to_string();
        let group_name_right = group_name.to_string();
        let book_ids_right = book_ids.to_vec();
        let on_right_click = move |event: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
            let key = group_key_right.clone();
            let name = group_name_right.clone();
            let ids = book_ids_right.clone();
            yomu_right.update(cx, |this, cx| {
                this.open_group_context_menu(&key, &name, &ids, event.position, cx);
            });
        };

        render_folder_list_row(
            group_key,
            group_name,
            count,
            cover_path,
            radius,
            border_px,
            colors,
            on_click,
            on_right_click,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_folder_card(
        &self,
        group_key: &str,
        group_name: &str,
        book_ids: &[String],
        count: usize,
        cover_path: PathBuf,
        card_w: f32,
        cover_h: f32,
        radius: Pixels,
        colors: UiColors,
        yomu: &Entity<Self>,
    ) -> AnyElement {
        let border_px = self.border_px();
        let yomu_click = yomu.clone();
        let group_key_click = group_key.to_string();
        let on_click = move |_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
            let key = group_key_click.clone();
            yomu_click.update(cx, |this, cx| {
                this.enter_group(key, cx);
            });
        };

        let yomu_right = yomu.clone();
        let group_key_right = group_key.to_string();
        let group_name_right = group_name.to_string();
        let book_ids_right = book_ids.to_vec();
        let on_right_click = move |event: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
            let key = group_key_right.clone();
            let name = group_name_right.clone();
            let ids = book_ids_right.clone();
            yomu_right.update(cx, |this, cx| {
                this.open_group_context_menu(&key, &name, &ids, event.position, cx);
            });
        };

        render_folder_card(
            group_key,
            group_name,
            count,
            cover_path,
            card_w,
            cover_h,
            radius,
            border_px,
            colors,
            on_click,
            on_right_click,
        )
    }

    fn render_all_manga_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = self.colors;
        let radius = px(self.radius_px());
        let yomu = cx.entity();

        let order = self.current_view_order();
        let query = self.search_query.trim().to_lowercase();
        let books: Vec<&Book> = order
            .iter()
            .filter_map(|id| self.library.get(id))
            .filter(|b| {
                if query.is_empty() {
                    true
                } else {
                    b.title.to_lowercase().contains(&query)
                        || b.series
                            .as_ref()
                            .is_some_and(|s| s.to_lowercase().contains(&query))
                }
            })
            .collect();

        if books.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(colors.muted)
                .child(Icon::new(IconName::BookOpen).size(px(48.)))
                .child(div().text_base().child(if query.is_empty() {
                    "Your library is empty"
                } else {
                    "No matching manga found"
                }))
                .into_any_element();
        }

        let thumb_size = self.library.settings().thumbnail_size;
        let sidebar_collapsed = self.is_sidebar_collapsed(window);
        let grid = ui::components::compute_responsive_grid(
            f32::from(window.viewport_size().width),
            !sidebar_collapsed,
            thumb_size,
        );
        let card_w = grid.card_w;
        let cover_h = grid.cover_h;

        let border_px = self.border_px();
        let is_grid = self.view_mode_grid;
        let mut cards: Vec<AnyElement> = Vec::new();

        if self.library.settings().group_series && self.open_group.is_none() {
            let groups = directory_groups(books.iter().copied());
            let mut grouped_ids = HashSet::new();

            for group in &groups {
                for b in &group.books {
                    grouped_ids.insert(b.id.clone());
                }
                let first = group.books[0];
                let group_key = group.key.clone();
                let group_name = group.name.clone();
                let count = group.books.len();
                let cover_path = self.dirs.cover_for_size(&first.id, thumb_size);

                if is_grid {
                    let book_ids: Vec<String> = group.books.iter().map(|b| b.id.clone()).collect();
                    cards.push(self.render_folder_card(
                        &group_key,
                        &group_name,
                        &book_ids,
                        count,
                        cover_path,
                        card_w,
                        cover_h,
                        radius,
                        colors,
                        &yomu,
                    ));
                } else {
                    let book_ids: Vec<String> = group.books.iter().map(|b| b.id.clone()).collect();
                    cards.push(self.render_folder_list_row(
                        &group_key,
                        &group_name,
                        &book_ids,
                        count,
                        cover_path,
                        radius,
                        colors,
                        border_px,
                        &yomu,
                    ));
                }
            }

            for book in books.iter().filter(|b| !grouped_ids.contains(&b.id)) {
                if is_grid {
                    cards.push(self.render_book_card(book, card_w, cover_h, radius, colors, &yomu));
                } else {
                    cards.push(self.render_book_list_row(book, radius, colors, &yomu));
                }
            }
        } else {
            for book in &books {
                if is_grid {
                    cards.push(self.render_book_card(book, card_w, cover_h, radius, colors, &yomu));
                } else {
                    cards.push(self.render_book_list_row(book, radius, colors, &yomu));
                }
            }
        }

        let in_group = self.open_group.is_some();
        let scroll_handle = if in_group {
            &self.group_scroll
        } else {
            &self.library_scroll
        };

        div()
            .id(if in_group {
                "group-grid"
            } else {
                "all-manga-container"
            })
            .track_scroll(scroll_handle)
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .child(if is_grid {
                div()
                    .px(px(grid.pad_x))
                    .py_6()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(grid.gap))
                    .children(cards)
            } else {
                div()
                    .w_full()
                    .max_w(px(1040.))
                    .mx_auto()
                    .px(px(grid.pad_x.max(16.0)))
                    .py_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(cards)
            })
            .into_any_element()
    }

    fn render_downloads_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.render_all_manga_view(window, cx)
    }

    fn render_details_screen(
        &self,
        book_id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(book) = self.library.get(book_id) else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child("Manga not found")
                .into_any_element();
        };

        let colors = self.colors;
        let radius = px(self.radius_px());
        let yomu = cx.entity();
        let cover_path = self.dirs.cover(&book.id);

        let props = ui::details::MangaDetailsProps {
            scroll_handle: &self.details_scroll,
            book,
            cover_path,
            selected_tab: self.details_tab_index,
            synopsis_expanded: self.details_synopsis_expanded,
            colors,
            radius,
        };

        ui::details::render_manga_details(
            props,
            {
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.navigate_back(cx))
            },
            {
                let yomu = yomu.clone();
                let book_id = book.id.clone();
                move |_, window, cx| {
                    yomu.update(cx, |this, cx| this.open_book(&book_id, window, cx))
                }
            },
            {
                let yomu = yomu.clone();
                move |idx, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.details_tab_index = idx;
                        cx.notify();
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.details_synopsis_expanded = !this.details_synopsis_expanded;
                        cx.notify();
                    })
                }
            },
            {
                let yomu = yomu.clone();
                let book_id = book.id.clone();
                move |_, pos, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.open_book_menu(&book_id, point(px(pos.0), px(pos.1)), cx)
                    })
                }
            },
        )
    }

    fn navigate_to(&mut self, view: ui::sidebar::MainView, cx: &mut Context<Self>) {
        let dest = match view {
            ui::sidebar::MainView::ContinueReading => {
                NavDestination::Section(ui::sidebar::SidebarSection::ContinueReading)
            }
            ui::sidebar::MainView::AllManga => {
                NavDestination::Section(ui::sidebar::SidebarSection::AllManga)
            }
            ui::sidebar::MainView::Collections => {
                NavDestination::Section(ui::sidebar::SidebarSection::Collections)
            }
            ui::sidebar::MainView::History => {
                NavDestination::Section(ui::sidebar::SidebarSection::History)
            }
            ui::sidebar::MainView::Downloads => {
                NavDestination::Section(ui::sidebar::SidebarSection::Downloads)
            }
            ui::sidebar::MainView::Details(id) => NavDestination::Details(id),
            ui::sidebar::MainView::Settings => {
                self.library_scroll_controller.reset();
                self.active_view = ui::sidebar::MainView::Settings;
                cx.notify();
                return;
            }
        };
        self.navigate_to_dest(dest, cx);
    }

    fn navigate_to_dest(&mut self, dest: NavDestination, cx: &mut Context<Self>) {
        if let Some(current) = self.nav_history.get(self.nav_history_idx)
            && current == &dest
        {
            self.apply_destination(&dest, cx);
            return;
        }
        if self.nav_history_idx + 1 < self.nav_history.len() {
            self.nav_history.truncate(self.nav_history_idx + 1);
        }
        self.nav_history.push(dest.clone());
        self.nav_history_idx = self.nav_history.len() - 1;
        self.apply_destination(&dest, cx);
    }

    fn apply_destination(&mut self, dest: &NavDestination, cx: &mut Context<Self>) {
        match dest {
            NavDestination::Section(sec) => {
                self.open_group = None;
                self.active_section = *sec;
                self.active_view = match sec {
                    ui::sidebar::SidebarSection::ContinueReading => {
                        ui::sidebar::MainView::ContinueReading
                    }
                    ui::sidebar::SidebarSection::AllManga => ui::sidebar::MainView::AllManga,
                    ui::sidebar::SidebarSection::Collections => ui::sidebar::MainView::Collections,
                    ui::sidebar::SidebarSection::History => ui::sidebar::MainView::History,
                    ui::sidebar::SidebarSection::Downloads => ui::sidebar::MainView::Downloads,
                };
            }
            NavDestination::Group { key, .. } => {
                self.open_group = Some(key.clone());
                self.active_section = ui::sidebar::SidebarSection::AllManga;
                self.active_view = ui::sidebar::MainView::AllManga;
            }
            NavDestination::Details(id) => {
                self.selected_book_id = Some(id.clone());
                self.active_view = ui::sidebar::MainView::Details(id.clone());
            }
        }
        self.library_scroll_controller.reset();
        cx.notify();
    }

    fn is_sidebar_collapsed(&self, window: &Window) -> bool {
        self.sidebar_collapsed || window.viewport_size().width < px(720.)
    }

    fn current_scroll_handle(&self) -> ScrollHandle {
        if self.open_group.is_some() {
            self.group_scroll.clone()
        } else {
            match self.active_view {
                ui::sidebar::MainView::ContinueReading => self.continue_reading_scroll.clone(),
                ui::sidebar::MainView::AllManga => self.library_scroll.clone(),
                ui::sidebar::MainView::Collections => self.collections_scroll.clone(),
                ui::sidebar::MainView::History => self.library_scroll.clone(),
                ui::sidebar::MainView::Downloads => self.library_scroll.clone(),
                ui::sidebar::MainView::Details(_) => self.details_scroll.clone(),
                ui::sidebar::MainView::Settings => self.settings_scroll.clone(),
            }
        }
    }

    fn is_current_view_scrolled(&self) -> bool {
        self.current_scroll_handle().offset().y < px(-2.0)
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    fn navigate_back(&mut self, cx: &mut Context<Self>) {
        if self.nav_history_idx > 0 {
            self.nav_history_idx -= 1;
            let dest = self.nav_history[self.nav_history_idx].clone();
            self.apply_destination(&dest, cx);
        } else if self.open_group.is_some() {
            self.open_group = None;
            self.group_scroll.set_offset(Point::default());
            cx.notify();
        }
    }

    #[allow(dead_code)]
    fn navigate_forward(&mut self, cx: &mut Context<Self>) {
        if self.nav_history_idx + 1 < self.nav_history.len() {
            self.nav_history_idx += 1;
            let dest = self.nav_history[self.nav_history_idx].clone();
            self.apply_destination(&dest, cx);
        }
    }

    #[allow(dead_code)]
    fn current_view_title(&self) -> &'static str {
        match &self.active_view {
            ui::sidebar::MainView::ContinueReading => "Continue Reading",
            ui::sidebar::MainView::AllManga => "All Manga",
            ui::sidebar::MainView::Collections => "Collections",
            ui::sidebar::MainView::History => "History",
            ui::sidebar::MainView::Downloads => "Downloads",
            ui::sidebar::MainView::Details(_) => "Manga Details",
            ui::sidebar::MainView::Settings => "Settings",
        }
    }

    fn render_reader(&self, reader: &Reader, cx: &mut Context<Self>) -> AnyElement {
        let vp = reader.viewport;

        // Pages are drawn at a size and position we compute ourselves, so what you see is exactly
        // what the fit mode and zoom say, independent of the texture's pixel size. During a page
        // turn the outgoing and incoming pages are both drawn, shifted along the same strip.
        let mut content: Vec<AnyElement> = Vec::new();
        if let Some(error) = &reader.error {
            content.push(centered(
                format!("Couldn't open this book: {error}"),
                self.colors,
            ));
        } else if reader.continuous {
            let width = vp.width * reader.zoom;
            let page_w = width.max(1.0);
            let gap = if reader.webcomics {
                0.0
            } else {
                12.0 * reader.zoom
            };
            let left = (vp.width - width) / 2.0 - reader.off.0;
            let visible_top = reader.off.1;
            let visible_bottom = visible_top + vp.height;
            let mut top = 0.0;
            for ix in 0..reader.total {
                let (w, h) = reader.orig_sizes.get(ix).and_then(|v| *v).unwrap_or((2, 3));
                let height = page_w * h as f32 / w.max(1) as f32;
                let overlap = if reader.webcomics && ix + 1 < reader.total {
                    1.0
                } else {
                    0.0
                };
                if top > visible_bottom {
                    break;
                }
                if top + height + overlap >= visible_top {
                    if let Some(page) = reader.images.get(&ix) {
                        content.push(page_layer(
                            &page.image,
                            left,
                            top - reader.off.1,
                            (width, height + overlap),
                        ));
                    } else if let Some(error) = reader.failed.get(&ix) {
                        content.push(centered(
                            format!("Couldn't decode page {}: {error}", ix + 1),
                            self.colors,
                        ));
                    }
                }
                top += height + gap;
            }
        } else {
            let (shift_out, shift_in) = match &reader.transition {
                Some(transition) => (
                    transition.outgoing_shift() * vp.width,
                    transition.incoming_shift() * vp.width,
                ),
                None => (0.0, 0.0),
            };
            if let Some(outgoing) = &reader.outgoing {
                content.push(page_layer(
                    &outgoing.image,
                    outgoing.origin.0 + shift_out,
                    outgoing.origin.1,
                    outgoing.display,
                ));
            }
            if let Some(page) = reader.current() {
                let display =
                    layout::display_size(reader.fit, reader.zoom, Reader::orig_of(page), &vp);
                let (left, top) = layout::origin(reader.off, display, &vp);
                content.push(page_layer(&page.image, left + shift_in, top, display));
            } else if reader.outgoing.is_none() {
                if let Some(error) = reader.failed.get(&reader.page) {
                    content.push(centered(
                        format!("Couldn't decode page {}: {error}", reader.page + 1),
                        self.colors,
                    ));
                } else if reader.pages.is_none() {
                    content.push(centered("Opening…", self.colors));
                }
                // otherwise: blank for the moment it takes the first page to decode
            }
        }

        let fraction = (reader.page + 1) as f32 / reader.total.max(1) as f32;
        let progress_line = div()
            .absolute()
            .bottom_0()
            .h(px(2.))
            .w(relative(fraction))
            .bg(self.colors.accent);
        let progress_line = progress_line.left_0();

        let info = reader.show_info.then(|| {
            pill(
                div().absolute().bottom_4().left_0().right_0(),
                format!(
                    "{} / {}  ·  {}  ·  {}%  ·  {}",
                    reader.page + 1,
                    reader.total,
                    reader.fit.label(),
                    (reader.zoom * 100.0).round() as i32,
                    if reader.webcomics {
                        "webcomics"
                    } else if reader.continuous {
                        "continuous"
                    } else {
                        "single page"
                    }
                ),
                self.colors,
                px(self.radius_px()),
            )
        });
        let hud = self.hud.clone().map(|text| {
            pill(
                div().absolute().top_4().left_0().right_0(),
                text,
                self.colors,
                px(self.radius_px()),
            )
        });

        let vim_status = if reader.vim.in_cmdline() {
            None
        } else {
            reader.vim.status().map(|text| {
                pill(
                    div().absolute().bottom_4().left_0().right_0(),
                    text,
                    self.colors,
                    px(self.radius_px()),
                )
                .into_any_element()
            })
        };

        let yomu = cx.entity();
        let book_title = self
            .library
            .get(&reader.book_id)
            .map(|b| b.title.clone())
            .unwrap_or_default();
        let current_page_str = format!("{} / {}", reader.page + 1, reader.total);
        let fit_mode = reader.fit;

        let top_bar = div()
            .id("reader-top-hover-area")
            .group("reader-top")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(56.))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .group_hover("reader-top", |s| s.opacity(1.0))
                    .opacity(0.0)
                    .w_full()
                    .h(px(52.))
                    .px_4()
                    .bg(with_alpha(self.colors.background, 0.92))
                    .border_b_1()
                    .border_color(with_alpha(self.colors.border, 0.5))
                    .shadow_lg()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Button::new("reader-back-btn")
                                    .ghost()
                                    .small()
                                    .label("← Library")
                                    .tooltip("Back to library (Esc)")
                                    .rounded(ButtonRounded::Medium)
                                    .on_click({
                                        let yomu = yomu.clone();
                                        move |_, window, cx| {
                                            yomu.update(cx, |this, cx| this.close_book(window, cx));
                                        }
                                    }),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(self.colors.foreground)
                                    .child(book_title),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(self.colors.muted)
                            .child(current_page_str),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("reader-fit-toggle")
                                    .ghost()
                                    .small()
                                    .dropdown_caret(true)
                                    .rounded(ButtonRounded::Medium)
                                    .tooltip("Display fit mode")
                                    .label(fit_mode.label())
                                    .dropdown_menu({
                                        let yomu = yomu.clone();
                                        move |mut menu, _, _| {
                                            menu = menu.check_side(MenuSide::Left).min_w(px(140.));
                                            let options = [
                                                FitMode::Page,
                                                FitMode::Width,
                                                FitMode::Height,
                                                FitMode::Original,
                                            ];
                                            for &mode in &options {
                                                let yomu = yomu.clone();
                                                menu = menu.item(
                                                    PopupMenuItem::new(mode.label())
                                                        .checked(mode == fit_mode)
                                                        .on_click(move |_, _, cx| {
                                                            yomu.update(cx, |this, cx| {
                                                                this.set_fit(mode, cx);
                                                            });
                                                        }),
                                                );
                                            }
                                            menu
                                        }
                                    }),
                            )
                            .child(
                                Button::new("reader-fullscreen-toggle")
                                    .ghost()
                                    .small()
                                    .rounded(ButtonRounded::Medium)
                                    .tooltip("Toggle fullscreen (F11)")
                                    .icon(IconName::Maximize)
                                    .on_click(move |_, window, _| window.toggle_fullscreen()),
                            ),
                    ),
            );

        let left_chevron = div()
            .id("reader-prev-hover-area")
            .group("reader-prev")
            .absolute()
            .left_0()
            .top(relative(0.35))
            .bottom(relative(0.35))
            .w(px(72.))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .items_center()
            .pl_4()
            .child(
                div()
                    .id("reader-prev-chevron")
                    .group_hover("reader-prev", |s| s.opacity(1.0))
                    .opacity(0.0)
                    .size(px(40.))
                    .rounded(px(self.radius_px()))
                    .bg(with_alpha(self.colors.background, 0.85))
                    .border_1()
                    .border_color(with_alpha(self.colors.border, 0.5))
                    .shadow_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(with_alpha(self.colors.accent, 0.2)))
                    .tooltip(|window, cx| {
                        Tooltip::new("Previous page (Left or K)").build(window, cx)
                    })
                    .on_click({
                        let yomu = yomu.clone();
                        move |_, _, cx| {
                            yomu.update(cx, |this, cx| {
                                this.turn_by(false, 1, Arrive::Start, cx);
                            })
                        }
                    })
                    .child(
                        Icon::new(IconName::ChevronLeft)
                            .size(px(20.))
                            .text_color(self.colors.foreground),
                    ),
            );

        let right_chevron = div()
            .id("reader-next-hover-area")
            .group("reader-next")
            .absolute()
            .right_0()
            .top(relative(0.35))
            .bottom(relative(0.35))
            .w(px(72.))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .items_center()
            .justify_end()
            .pr_4()
            .child(
                div()
                    .id("reader-next-chevron")
                    .group_hover("reader-next", |s| s.opacity(1.0))
                    .opacity(0.0)
                    .size(px(40.))
                    .rounded(px(self.radius_px()))
                    .bg(with_alpha(self.colors.background, 0.85))
                    .border_1()
                    .border_color(with_alpha(self.colors.border, 0.5))
                    .shadow_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(with_alpha(self.colors.accent, 0.2)))
                    .tooltip(|window, cx| Tooltip::new("Next page (Right or J)").build(window, cx))
                    .on_click({
                        let yomu = yomu.clone();
                        move |_, _, cx| {
                            yomu.update(cx, |this, cx| {
                                this.turn_by(true, 1, Arrive::Start, cx);
                            })
                        }
                    })
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .size(px(20.))
                            .text_color(self.colors.foreground),
                    ),
            );

        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(self.colors.background)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, _| {
                    this.drag_start((f32::from(event.position.x), f32::from(event.position.y)));
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                let at = (f32::from(event.position.x), f32::from(event.position.y));
                this.cursor_pos = Some(at);
                let pressed = event.pressed_button == Some(MouseButton::Left);
                this.drag_move(at, pressed, viewport_of(window), cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.drag_end(f32::from(event.position.x), viewport_of(window), cx);
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                let delta = event.delta.pixel_delta(px(WHEEL_LINE));
                let discrete = matches!(event.delta, ScrollDelta::Lines(_));
                let control = event.modifiers.control || event.modifiers.platform;
                this.on_wheel(
                    WheelParams::new(
                        (f32::from(delta.x), f32::from(delta.y)),
                        control,
                        (f32::from(event.position.x), f32::from(event.position.y)),
                        discrete,
                        event.touch_phase,
                        viewport_of(window),
                    ),
                    cx,
                );
            }))
            .on_pinch(cx.listener(|this, event: &PinchEvent, window, cx| {
                let at = (f32::from(event.position.x), f32::from(event.position.y));
                this.on_pinch(at, event.delta, event.phase, viewport_of(window), cx);
            }))
            .children(content)
            .child(progress_line)
            .children(info)
            .children(hud)
            .children(vim_status)
            .child(top_bar)
            .child(left_chevron)
            .child(right_chevron)
            .children(reader.show_help.then(|| {
                ui::overlays::help_overlay(self.colors, self.border_px(), self.radius_xl_px())
            }))
            .into_any_element()
    }

    fn render_command_box(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let input = self.cmdline_text()?;
        let style = self.overlay_style(self.colors.surface);
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.cancel_cmdline(cx));
        };

        let yomu_select = yomu.clone();
        let on_select = move |index: usize, _window: &mut Window, cx: &mut App| {
            yomu_select.update(cx, |this, cx| {
                if this.command_selection != index {
                    this.command_selection = index;
                    if let Some(draft) = this.cmdline_text().map(str::to_owned) {
                        this.sync_theme_preview(&draft, cx);
                    }
                    cx.notify();
                }
            });
        };

        let yomu_submit = yomu.clone();
        let on_submit = move |completion: String, window: &mut Window, cx: &mut App| {
            let is_complete = !completion.ends_with(' ');
            yomu_submit.update(cx, |this, cx| {
                this.replace_cmdline(completion.clone());
                if is_complete {
                    let vp = viewport_of(window);
                    let actions = if let Some(reader) = this.reader.as_mut() {
                        reader.vim.feed("enter")
                    } else {
                        this.library_vim.feed("enter")
                    };
                    this.command_selection = 0;
                    this.theme_before_preview = None;
                    for action in actions {
                        this.apply(action, vp, window, cx);
                    }
                } else {
                    this.command_selection = 0;
                    if let Some(draft) = this.cmdline_text().map(str::to_owned) {
                        this.sync_theme_preview(&draft, cx);
                    }
                }
                cx.notify();
            });
        };

        Some(ui::overlays::render_command_box(
            input,
            self.command_selection,
            &self.custom_themes,
            style,
            on_close,
            on_select,
            on_submit,
        ))
    }
}

impl Render for Yomu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for image in self.garbage.drain(..) {
            let _ = window.drop_image(image);
        }
        // Animations are driven by the time between frames, never by a frame count, and only
        // ask for another frame while something is actually moving.
        let now = Instant::now();
        let dt = self.frame_delta(now);
        let reader_animating = self.sync_reader(window, cx, dt);
        let scroll_animating = if self.reader.is_none() {
            let scroll_handle = self.current_scroll_handle();
            let animations_enabled = self.animations_enabled();
            self.library_scroll_controller
                .check_stillness_timeout(now, animations_enabled);
            let stepped = self.library_scroll_controller.step(now, &scroll_handle);
            let awaiting = self.library_scroll_controller.is_awaiting_release();
            stepped || awaiting
        } else {
            false
        };
        let animating = reader_animating || scroll_animating;
        if animating {
            self.last_frame = Some(now);
            window.request_animation_frame();
        } else {
            self.last_frame = None;
            self.dt_smoothed = None;
        }
        // Textures that a finished slide just let go of are not drawn this frame, so free them
        // now rather than waiting for a frame that an idle window might never draw.
        for image in self.garbage.drain(..) {
            let _ = window.drop_image(image);
        }
        self.sync_appearance(window, cx);

        let reading = self.reader.is_some();
        let content = match self.reader.as_ref() {
            Some(reader) => self.render_reader(reader, cx),
            None => self.render_two_pane_shell(window, cx),
        };

        let root_bg = self.colors.background;
        let overlay = self.render_overlay(window, cx);
        let cmd_box = self.render_command_box(cx);

        div()
            .key_context(if reading { "Reader" } else { "Library" })
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .bg(root_bg)
            .text_color(self.colors.foreground)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    if this.search_active {
                        this.deactivate_search(cx);
                    }
                }),
            )
            .on_action(cx.listener(|_, _: &ToggleFullscreen, window, _| window.toggle_fullscreen()))
            .on_action(cx.listener(|this, _: &ImportFiles, _, cx| this.import_dialog(false, cx)))
            .on_action(cx.listener(|this, _: &ImportFolder, _, cx| this.import_dialog(true, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.open_settings_window(window, cx)
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                if this.overlay.is_none() {
                    this.import_paths(paths.paths().to_vec(), cx);
                }
            }))
            .child(content)
            .children(overlay)
            .children(cmd_box)
    }
}

impl Drop for Yomu {
    fn drop(&mut self) {
        if self.save_pending {
            self.save_pending = false;
            let mut guard = self.last_saved_seq.lock().unwrap();
            if self.save_seq > *guard {
                if let Err(err) = self.library.save(&self.dirs) {
                    eprintln!("yomu: failed to save library on exit: {err:#}");
                } else {
                    *guard = self.save_seq;
                }
            }
        }
    }
}

fn app_icon() -> Option<Arc<image::RgbaImage>> {
    let bytes = include_bytes!("../assets/logo.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let resized = image::imageops::resize(&img, 128, 128, image::imageops::FilterType::Lanczos3);
    Some(Arc::new(resized))
}

/// Installs desktop entry and application icons to the specified data directory (typically `$XDG_DATA_HOME`).
///
/// Returns `Ok(true)` if files were written or updated, or `Ok(false)` if all items were already present.
/// If `force` is false, existing user-customized `.desktop` files or icons are preserved.
#[cfg(target_os = "linux")]
pub fn install_desktop_integration_to(data_dir: &Path, force: bool) -> anyhow::Result<bool> {
    let mut updated = false;
    let logo_bytes = include_bytes!("../assets/logo.png");
    let pixmap_icon = data_dir.join("pixmaps/yomu.png");
    let pixmap_needs_write = force || !pixmap_icon.exists();
    if pixmap_needs_write {
        if let Some(parent) = pixmap_icon.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&pixmap_icon, logo_bytes)?;
        updated = true;
    }

    let sizes: &[(&str, &[u8])] = &[
        ("16x16", include_bytes!("../assets/icons/16x16.png")),
        ("24x24", include_bytes!("../assets/icons/24x24.png")),
        ("32x32", include_bytes!("../assets/icons/32x32.png")),
        ("48x48", include_bytes!("../assets/icons/48x48.png")),
        ("64x64", include_bytes!("../assets/icons/64x64.png")),
        ("128x128", include_bytes!("../assets/icons/128x128.png")),
        ("256x256", include_bytes!("../assets/icons/256x256.png")),
        ("512x512", include_bytes!("../assets/icons/512x512.png")),
    ];

    for (sz, bytes) in sizes {
        let path = data_dir.join(format!("icons/hicolor/{sz}/apps/yomu.png"));
        let icon_needs_write = force || !path.exists();
        if icon_needs_write {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, bytes)?;
            updated = true;
        }
    }

    let desktop_file = data_dir.join("applications/yomu.desktop");
    let desktop_content = include_str!("../packaging/yomu.desktop");
    let desktop_needs_write = force || !desktop_file.exists();
    if desktop_needs_write {
        if let Some(parent) = desktop_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&desktop_file, desktop_content)?;
        updated = true;
    }

    Ok(updated)
}

#[cfg(not(target_os = "linux"))]
pub fn install_desktop_integration_to(_data_dir: &Path, _force: bool) -> anyhow::Result<bool> {
    anyhow::bail!("Desktop integration installation is only supported on Linux.");
}

pub fn install_desktop_integration(force: bool) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine XDG_DATA_HOME directory"))?;

        let updated = install_desktop_integration_to(&data_dir, force)?;
        if updated {
            println!(
                "Installed yomu desktop integration in {}",
                data_dir.display()
            );
            let hicolor_dir = data_dir.join("icons/hicolor");
            match std::process::Command::new("gtk-update-icon-cache")
                .arg("-f")
                .arg("-t")
                .arg(&hicolor_dir)
                .output()
            {
                Ok(output) if output.status.success() => {
                    println!("Updated icon cache: {}", hicolor_dir.display());
                }
                Ok(output) => {
                    eprintln!(
                        "gtk-update-icon-cache notice (exit {}): {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                }
                Err(err) => {
                    eprintln!(
                        "gtk-update-icon-cache not available ({err}); skipping cache update."
                    );
                }
            }
        } else {
            println!(
                "Desktop integration already installed in {} (use --force to overwrite)",
                data_dir.display()
            );
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = force;
        anyhow::bail!("Desktop integration installation is only supported on Linux.");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args
        .iter()
        .any(|arg| arg == "--install-desktop" || arg == "--install-icons")
    {
        let force = args.iter().any(|arg| arg == "--force");
        if let Err(err) = install_desktop_integration(force) {
            eprintln!("yomu: desktop installation failed: {err:#}");
            std::process::exit(1);
        }
        return;
    }

    let vim_cli_override = args.iter().any(|arg| arg == "--vim" || arg == "--vim-mode");

    application()
        // GPUI Kit icons are SVG asset paths. Without registering this source,
        // custom-rendered toolbar and context-menu icons resolve to empty images.
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            let mut bindings = vec![
                KeyBinding::new("ctrl-o", ImportFiles, Some("Library")),
                KeyBinding::new("ctrl-shift-o", ImportFolder, Some("Library")),
                KeyBinding::new("f11", ToggleFullscreen, None),
                KeyBinding::new("ctrl-,", OpenSettings, None),
                KeyBinding::new("ctrl-q", Quit, None),
            ];
            if cfg!(target_os = "macos") {
                bindings.extend([
                    KeyBinding::new("cmd-o", ImportFiles, Some("Library")),
                    KeyBinding::new("cmd-shift-o", ImportFolder, Some("Library")),
                    KeyBinding::new("cmd-,", OpenSettings, None),
                    KeyBinding::new("cmd-q", Quit, None),
                ]);
            }
            cx.bind_keys(bindings);

            cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
            cx.on_window_closed(|cx, _window_id| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1180.), px(780.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_background: WindowBackgroundAppearance::Opaque,
                    titlebar: Some(TitlebarOptions {
                        title: Some("yomu".into()),
                        appears_transparent: cfg!(target_os = "macos"),
                        traffic_light_position: if cfg!(target_os = "macos") {
                            Some(point(px(16.), px(18.)))
                        } else {
                            None
                        },
                    }),
                    // Compositors match window rules (Niri, Hyprland, Sway…) on the app id.
                    app_id: Some("yomu".into()),
                    window_min_size: Some(size(px(480.), px(360.))),
                    window_decorations: Some(WindowDecorations::Server),
                    icon: app_icon(),
                    ..Default::default()
                },
                move |window, cx| {
                    let view = cx.new(|cx| Yomu::new(window, cx));
                    view.update(cx, |this, _| this.vim_cli_override = vim_cli_override);
                    let focus = view.read(cx).focus_handle.clone();
                    window.focus(&focus, cx);
                    view
                },
            )
            .expect("failed to open the window");

            cx.activate(true);
        });
}

#[cfg(test)]
mod tests;
