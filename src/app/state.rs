//! Root application state, view router, and lifecycle initialization.

#![forbid(unsafe_code)]

use std::{
    cell::Cell,
    collections::HashMap,
    fs,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64},
    },
    time::{Duration, Instant},
};

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollbarHandle;
use gpui_kit::component::{Theme as KitTheme, ThemeConfig};
use gpui_kit::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, Focusable, Pixels, Point, RenderImage,
    ScrollHandle, SharedString, Size, Window, point, px, size,
};
#[cfg(not(test))]
use yomu::motion;
use yomu::{
    library::{Book, CornerRounding, DataDirs, Library},
    selection::Selection,
    vim::Vim,
};

use crate::ui;
use crate::ui::overlays::{Overlay, SettingsTab};
use crate::ui::reader::Reader;
#[cfg(not(test))]
use crate::ui::theme::custom_theme_fingerprint;
use crate::ui::theme::{
    DEFAULT_THEME_NAME, UiColors, built_in_theme, built_in_theme_names, custom_theme_path,
    ensure_custom_theme_file, parse_custom_themes, resolve_theme_name,
};

/// How far `j`/`k`/`h`/`l` scroll, in logical pixels.
pub const LINE_STEP: f32 = 64.0;
/// A wheel notch scrolls this many pixels (and is one zoom step with Ctrl).
pub const WHEEL_LINE: f32 = 96.0;
/// Slack when deciding whether the page is already scrolled to its edge.
pub const EDGE_EPS: f32 = 1.0;
/// Mouse travel before a press counts as a drag instead of a click.
pub const DRAG_THRESHOLD: f32 = 4.0;
pub const HUD_SECS: u64 = 2;

/// Clicks this soon after a menu or dialog closes are ignored, so double-clicking a menu item
/// cannot also click the book underneath it.
#[allow(dead_code)]
pub const CLICK_GUARD: Duration = Duration::from_millis(300);
/// Clicks that caught / stopped an active inertial scroll glide are suppressed from opening cards.
#[allow(dead_code)]
pub const SCROLL_CATCH_GUARD: Duration = Duration::from_millis(150);

#[derive(Clone, Copy)]
pub enum Side {
    Left,
    Right,
}

pub enum ImportEvent {
    Book(Book),
    Finished(yomu::library::ImportReport),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavDestination {
    Section(ui::sidebar::SidebarSection),
    Group { key: String, name: String },
    Details(String),
}

#[derive(Clone)]
pub struct ReaderScrollHandle {
    pub bounds: Bounds<Pixels>,
    pub offset_y: f32,
    pub content_height: f32,
    pub sync: Rc<Cell<Option<f32>>>,
    pub max_offset: f32,
}

impl ScrollbarHandle for ReaderScrollHandle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    fn offset(&self) -> Point<Pixels> {
        point(px(0.), px(-self.offset_y))
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        let target_y = (-f32::from(offset.y)).clamp(0.0, self.max_offset);
        self.sync.set(Some(target_y));
    }

    fn content_size(&self) -> Size<Pixels> {
        size(
            self.bounds.size.width,
            px(self.content_height.max(f32::from(self.bounds.size.height))),
        )
    }
}

pub struct Yomu {
    pub(crate) focus_handle: FocusHandle,
    pub(crate) dirs: Arc<DataDirs>,
    pub(crate) library: Library,
    pub(crate) colors: UiColors,
    pub(crate) custom_themes: HashMap<String, Rc<ThemeConfig>>,
    pub(crate) custom_theme_path: PathBuf,
    pub(crate) theme_status: Option<String>,
    pub(crate) reader: Option<Reader>,
    /// Images that are no longer shown. Their GPU textures are freed on the next render,
    /// because that is where we have access to the `Window`.
    pub(crate) garbage: Vec<Arc<RenderImage>>,
    pub(crate) toast: Option<SharedString>,
    pub(crate) toast_seq: u64,
    pub(crate) hud: Option<SharedString>,
    pub(crate) hud_seq: u64,
    pub(crate) importing: bool,
    pub(crate) importing_title: Option<String>,
    pub(crate) imported_count: usize,
    pub(crate) import_cancel: Option<Arc<AtomicBool>>,
    pub(crate) system_dark: bool,
    /// The desktop asks for reduced motion (see `motion::detect_reduced_motion`).
    pub(crate) reduce_motion: bool,
    /// When the previous animated frame was drawn; `None` while nothing is animating.
    pub(crate) last_frame: Option<Instant>,
    /// Smoothed frame interval for display vsync scheduling jitter mitigation.
    pub(crate) dt_smoothed: Option<f32>,
    /// Tests drive the animation clock by hand instead of using real time.
    #[cfg(test)]
    pub frame_dt: Option<f32>,
    pub(crate) overlay: Option<Overlay>,
    pub(crate) settings_tab: SettingsTab,
    pub(crate) command_selection: usize,
    pub(crate) library_vim: Vim,
    pub(crate) theme_before_preview: Option<String>,
    pub(crate) overlay_closed_at: Option<Instant>,
    /// Books picked for bulk actions in the library.
    pub(crate) selection: Selection,
    /// Transient library filter; it is not part of persisted reader settings.
    pub(crate) search_query: String,
    pub(crate) search_active: bool,
    pub(crate) search_input: Option<Entity<InputState>>,
    pub(crate) open_group: Option<String>,
    /// Which background appearance the window currently has (`None` before the first frame).
    pub(crate) applied_opaque: Option<bool>,
    pub(crate) library_scroll: ScrollHandle,
    pub(crate) group_scroll: ScrollHandle,
    pub(crate) continue_reading_scroll: ScrollHandle,
    pub(crate) collections_scroll: ScrollHandle,
    pub(crate) details_scroll: ScrollHandle,
    pub(crate) settings_scroll: ScrollHandle,
    pub(crate) sidebar_scroll: ScrollHandle,
    pub(crate) reader_scroll_sync: Rc<Cell<Option<f32>>>,
    pub(crate) library_scroll_controller: ui::scroll::PlatformScrollController,
    #[allow(dead_code)]
    pub(crate) library_pinch_accum: f32,
    pub(crate) cursor_pos: Option<(f32, f32)>,
    pub(crate) save_seq: u64,
    pub(crate) last_saved_seq: Arc<AtomicU64>,
    pub(crate) write_lock: Arc<Mutex<()>>,
    pub(crate) save_pending: bool,
    pub(crate) save_pending_since: Option<Instant>,
    pub(crate) is_quitting: bool,
    // Two-pane redesign fields:
    pub(crate) active_section: ui::sidebar::SidebarSection,
    pub(crate) active_view: ui::sidebar::MainView,
    pub(crate) selected_book_id: Option<String>,
    pub(crate) details_tab_index: usize,
    pub(crate) details_synopsis_expanded: bool,
    pub(crate) nav_history: Vec<NavDestination>,
    pub(crate) nav_history_idx: usize,
    pub(crate) view_mode_grid: bool,
    pub(crate) selected_collection_id: Option<String>,
    pub(crate) sidebar_collapsed: bool,
    pub vim_cli_override: bool,
}

impl Yomu {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dirs = DataDirs::new().expect("cannot create the yomu data directory");
        Self::with_dirs(dirs, window, cx)
    }

    pub fn with_dirs(dirs: DataDirs, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _ = yomu::pagecache::system_memory_budget();
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
                .unwrap_or(std::path::Path::new("."))
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
        let resolved = resolve_theme_name(&scheme, mode, system_dark, &custom_themes);
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
            import_cancel: None,
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
            reader_scroll_sync: Rc::new(Cell::new(None)),
            library_scroll_controller: ui::scroll::PlatformScrollController::new(),
            library_pinch_accum: 0.0,
            cursor_pos: None,
            save_seq: 0,
            last_saved_seq: Arc::new(AtomicU64::new(0)),
            write_lock: Arc::new(Mutex::new(())),
            save_pending: false,
            save_pending_since: None,
            is_quitting: false,
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

    pub fn apply_corner_rounding(rounding: CornerRounding, cx: &mut App) {
        KitTheme::update(cx, |theme| {
            theme.radius = px(rounding.radius_pixels());
            theme.radius_lg = px(rounding.radius_lg_pixels());
        });
    }

    /// Whether page turns and scrolling should animate right now.
    pub fn animations_enabled(&self) -> bool {
        !self.reduce_motion && !self.library.settings().disable_animations
    }

    pub fn is_vim_enabled(&self) -> bool {
        if self.vim_cli_override {
            return true;
        }
        self.library.settings().vim_mode
    }

    pub fn activate_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_active = true;
        if let Some(input) = &self.search_input {
            let focus_handle = input.read(cx).focus_handle(cx);
            window.focus(&focus_handle, cx);
        }
        cx.notify();
    }

    pub fn deactivate_search(&mut self, cx: &mut Context<Self>) {
        self.search_active = false;
        cx.notify();
    }

    pub fn navigate_to(&mut self, view: ui::sidebar::MainView, cx: &mut Context<Self>) {
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

    pub fn navigate_to_dest(&mut self, dest: NavDestination, cx: &mut Context<Self>) {
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

    pub fn apply_destination(&mut self, dest: &NavDestination, cx: &mut Context<Self>) {
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

    pub fn is_sidebar_collapsed(&self, window: &Window) -> bool {
        self.sidebar_collapsed || window.viewport_size().width < px(720.)
    }

    pub fn current_scroll_handle(&self) -> ScrollHandle {
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

    pub fn is_current_view_scrolled(&self) -> bool {
        self.current_scroll_handle().offset().y < px(-2.0)
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    pub fn navigate_back(&mut self, cx: &mut Context<Self>) {
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
    pub fn navigate_forward(&mut self, cx: &mut Context<Self>) {
        if self.nav_history_idx + 1 < self.nav_history.len() {
            self.nav_history_idx += 1;
            let dest = self.nav_history[self.nav_history_idx].clone();
            self.apply_destination(&dest, cx);
        }
    }

    #[allow(dead_code)]
    pub fn current_view_title(&self) -> &'static str {
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

    /// Watch the generated custom theme file so Noctalia palette changes apply without a manual
    /// reload. Hashing is done on the background executor; the small TOML is parsed on change.
    #[cfg(not(test))]
    pub fn watch_custom_theme_file(&self, cx: &mut Context<Self>) {
        let path = self.custom_theme_path.clone();
        cx.spawn(async move |this, cx| {
            let mut last_hash = custom_theme_fingerprint(&path);
            loop {
                let is_quitting = this
                    .read_with(cx, |this, _| this.is_quitting)
                    .unwrap_or(true);
                if is_quitting {
                    break;
                }
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
}
