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
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Theme as KitTheme, ThemeConfig};
#[allow(unused_imports)]
use gpui_kit::popup::{PopupAnchor, PopupConstraintAdjustment, PopupGravity, PopupOptions};
#[allow(unused_imports)]
use gpui_kit::{
    Anchor, Animation, AnimationExt, AnyElement, AnyWindowHandle, App, Bounds, ClickEvent, Context,
    Entity, ExternalPaths, FocusHandle, Focusable, FontWeight, KeyBinding, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, PathPromptOptions,
    PinchEvent, Pixels, Point, RenderImage, Rgba, ScrollDelta, ScrollHandle, ScrollWheelEvent,
    SharedString, Styled, Subscription, TitlebarOptions, TouchPhase, Window,
    WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, actions, anchored,
    application, canvas, deferred, div, ease_out_quint, img, point, prelude::*, px, relative, size,
};
use image::Frame;
use yomu::{
    archive, imaging,
    layout::{self, Arrive, FitMode, Viewport},
    library::{self, Book, BorderSize, CornerRounding, DataDirs, Library, Settings, ThumbnailSize},
    menu::{self, BookAction, Entry, Item},
    motion::{self, KineticScroll},
    pagecache, search,
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

use ui::components::{
    ButtonKind, centered, overlay_shade, page_layer, panel_button, pill, with_alpha,
};
use ui::overlays::{
    BookMenu, ConfirmAction, ConfirmDialog, LibraryMenu, Overlay, PropertiesDialog, SeriesDialog,
    SettingsTab,
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
const CLICK_GUARD: Duration = Duration::from_millis(300);
/// Clicks that caught / stopped an active inertial scroll glide are suppressed from opening cards.
const SCROLL_CATCH_GUARD: Duration = Duration::from_millis(150);

#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

struct LibraryGroup<'a> {
    key: String,
    name: String,
    first_seen: usize,
    books: Vec<&'a Book>,
}

enum ImportEvent {
    Book(Book),
    Finished(library::ImportReport),
}

fn directory_groups(books: &[Book]) -> Vec<LibraryGroup<'_>> {
    let mut buckets: std::collections::BTreeMap<String, (String, usize, Vec<&Book>)> =
        std::collections::BTreeMap::new();
    for (index, book) in books.iter().enumerate() {
        let manual_series = book
            .series
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty());
        let (key, name) = if let Some(series) = manual_series {
            (
                format!("series:{}", series.to_lowercase()),
                series.to_string(),
            )
        } else {
            let parent = book.path.parent().unwrap_or_else(|| Path::new("."));
            let display = parent
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| parent.display().to_string());
            (format!("directory:{}", parent.display()), display)
        };
        buckets
            .entry(key)
            .and_modify(|(_, _, members)| members.push(book))
            .or_insert_with(|| (name, index, vec![book]));
    }

    let mut groups: Vec<_> = buckets
        .into_iter()
        .filter_map(|(key, (name, first_seen, members))| {
            if members.len() < 2 {
                return None;
            }
            Some(LibraryGroup {
                key,
                name,
                first_seen,
                books: members,
            })
        })
        .collect();
    groups.sort_by_key(|group| group.first_seen);
    groups
}

/// Native kinetic / inertial scroll controller for UI scrollable containers (e.g. library grid, settings).
#[derive(Clone, Debug, Default)]
pub struct UiScrollController {
    pub handle: ScrollHandle,
    pub kinetic: KineticScroll,
}

impl UiScrollController {
    pub fn new() -> Self {
        Self {
            handle: ScrollHandle::new(),
            kinetic: KineticScroll::new(),
        }
    }

    pub fn reset(&mut self) {
        self.kinetic.reset();
    }

    pub fn cancel_inertia(&mut self) {
        self.kinetic.cancel_inertia();
    }

    pub fn has_active_inertia(&self) -> bool {
        self.kinetic.has_active_inertia()
    }

    pub fn is_active(&self) -> bool {
        self.kinetic.is_active()
    }

    /// Process a scroll wheel event on this scrollable container.
    pub fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, reduce_motion: bool) {
        let discrete = matches!(event.delta, ScrollDelta::Lines(_));
        if discrete
            || event.touch_phase == TouchPhase::Started
            || event.touch_phase == TouchPhase::Cancelled
        {
            self.kinetic.reset();
            return;
        }

        // Touch-catch: any new continuous scroll gesture cancels active inertia glide and resets momentum
        if self.kinetic.has_active_inertia() {
            self.kinetic.reset();
        }

        let now = Instant::now();
        let delta = event.delta.pixel_delta(px(16.0));
        let dy = f32::from(delta.y);
        let dx = f32::from(delta.x);

        if event.touch_phase == TouchPhase::Moved {
            self.kinetic.push_motion(now, dx, dy);
        } else if event.touch_phase == TouchPhase::Ended {
            self.kinetic.end_touch(!reduce_motion);
        }
    }

    /// Step kinetic momentum forward by `dt` seconds. Returns true if animation is still running.
    pub fn step(&mut self, dt: f32, reduce_motion: bool) -> bool {
        if reduce_motion {
            self.kinetic.reset();
            return false;
        }

        let now = Instant::now();
        self.kinetic.check_stillness_timeout(now, !reduce_motion);

        if let Some((_dx, dy)) = self.kinetic.step(dt) {
            let current = self.handle.offset();
            let max = self.handle.max_offset();

            if max.y > px(0.) {
                // In GPUI div: scrolling down makes offset.y negative, bounded in [-max.y, 0.0]
                let target_y = current.y + px(dy);
                let clamped_y = target_y.clamp(-max.y, px(0.));

                self.handle.set_offset(Point {
                    x: current.x,
                    y: clamped_y,
                });

                let hit_boundary =
                    (target_y >= px(0.) && dy > 0.0) || (target_y <= -max.y && dy < 0.0);
                if hit_boundary {
                    self.kinetic.cancel_inertia();
                }
            }
        }

        self.kinetic.is_active()
    }
}

struct Yomu {
    focus_handle: FocusHandle,
    dirs: Arc<DataDirs>,
    library: Library,
    colors: UiColors,
    custom_themes: HashMap<String, Rc<ThemeConfig>>,
    custom_theme_path: PathBuf,
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
    library_scroll: UiScrollController,
    group_scroll: UiScrollController,
    shortcuts_scroll: UiScrollController,
    scroll_caught_at: Option<Instant>,
    library_pinch_accum: f32,
    cursor_pos: Option<(f32, f32)>,
    save_seq: u64,
    last_saved_seq: Arc<Mutex<u64>>,
    save_pending: bool,
    save_pending_since: Option<Instant>,
}

impl Yomu {
    fn new(cx: &mut Context<Self>) -> Self {
        let dirs = DataDirs::new().expect("cannot create the yomu data directory");
        Self::with_dirs(dirs, cx)
    }

    fn with_dirs(dirs: DataDirs, cx: &mut Context<Self>) -> Self {
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
        let selected_theme = library.settings().theme_name.as_str();
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
            search_input: None,
            open_group: None,
            applied_opaque: None,
            library_scroll: UiScrollController::new(),
            group_scroll: UiScrollController::new(),
            shortcuts_scroll: UiScrollController::new(),
            scroll_caught_at: None,
            library_pinch_accum: 0.0,
            cursor_pos: None,
            save_seq: 0,
            last_saved_seq: Arc::new(Mutex::new(0)),
            save_pending: false,
            save_pending_since: None,
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
            this.update(cx, |this, cx| {
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
        self.library.settings().animate && !self.reduce_motion
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
                this.update(cx, |this, cx| {
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
            this.update(cx, |this, cx| {
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
            this.update(cx, |this, cx| {
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

    fn overlay_recently_closed(&self) -> bool {
        self.overlay_closed_at
            .is_some_and(|closed| closed.elapsed() < CLICK_GUARD)
    }

    #[cfg(test)]
    fn open_book_menu(&mut self, id: &str, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(book) = self.library.get(id) else {
            return;
        };
        let entries = menu::book_menu(book, book.path.exists());
        self.overlay = Some(Overlay::Menu(BookMenu {
            book_id: id.to_string(),
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
        let message = match (series, count) {
            (Some(name), 1) => format!("Added to series “{name}”"),
            (Some(name), _) => format!("Assigned {count} books to “{name}”"),
            (None, 1) => "Removed from series".to_string(),
            (None, _) => format!("Removed {count} books from their series"),
        };
        self.show_toast(message, cx);
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Bulk actions work on the selection, not on the book that was right-clicked.
        match action {
            BookAction::SetSeries => {
                let ids = if self.selection.contains(id) && self.selection.len() > 1 {
                    self.selected_ids()
                } else {
                    vec![id.to_string()]
                };
                self.open_series_dialog(ids, window, cx);
                return;
            }
            BookAction::RemoveSelected => {
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
            ConfirmAction::RemoveSelected => self.remove_selected(cx),
            ConfirmAction::ResetSettings => self.reset_settings(cx),
            ConfirmAction::ClearThumbnailCache => self.clear_thumbnail_cache(cx),
            ConfirmAction::ClearAllData => self.clear_all_data(cx),
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

    fn set_border_size(&mut self, size: BorderSize, cx: &mut Context<Self>) {
        if self.library.settings().border_size == size {
            return;
        }
        self.library.settings_mut().border_size = size;
        self.save_library();
        cx.notify();
    }

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
        self.group_scroll.reset();
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
        self.apply_theme(&defaults.theme_name, cx);
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
        if let Some(prev) = self.theme_before_preview.take() {
            if self.library.settings().theme_name != prev {
                self.preview_theme(&prev, cx);
            }
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
                    if let Some(action) = picked {
                        self.run_book_action(action, &id, window, cx);
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
                self.group_scroll.reset();
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
                        this.update(cx, |this, cx| {
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
                        this.update(cx, |this, cx| {
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
                        this.update(cx, |this, cx| {
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
                self.group_scroll.reset();
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

    /// Right-click on a book: a book inside a multi-book selection gets the bulk menu; anywhere
    /// else the selection is dropped (like a file manager) and the book gets its own menu.
    fn open_context_menu(
        &mut self,
        id: &str,
        position: Point<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_ids = self.selected_ids();
        let entries = if selected_ids.contains(&id.to_string()) && selected_ids.len() > 1 {
            menu::selection_menu(selected_ids.len())
        } else {
            self.selection.clear();
            let Some(book) = self.library.get(id) else {
                return;
            };
            menu::book_menu(book, book.path.exists())
        };
        let menu = BookMenu {
            book_id: id.to_string(),
            entries,
            position,
            selected: None,
        };
        self.overlay = Some(Overlay::Menu(menu));
        cx.notify();
    }

    fn open_library_menu(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Some(Overlay::LibraryMenu(_))) {
            self.close_overlay(cx);
            return;
        }
        self.overlay = Some(Overlay::LibraryMenu(LibraryMenu::default()));
        cx.notify();
    }

    /// Remove every selected book from the library, asking first only if progress or marks
    /// would be lost. The comic files are never touched.
    fn request_remove_selected(&mut self, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
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
                    action: ConfirmAction::RemoveSelected,
                }));
                cx.notify();
            }
            None => self.remove_selected(cx),
        }
    }

    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        self.dismiss();
        let ids = self.selected_ids();
        for id in &ids {
            self.library.remove(id, &self.dirs);
        }
        self.selection.clear();
        self.prune_open_group();
        self.save_library();
        let message = if ids.len() == 1 {
            "Removed 1 book from the library".to_string()
        } else {
            format!("Removed {} books from the library", ids.len())
        };
        self.show_toast(message, cx);
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
        if matches!(token, "C-f" | "/" | "slash") {
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

    fn enter_group(&mut self, key: String, cx: &mut Context<Self>) {
        self.selection.clear();
        self.open_group = Some(key);
        self.group_scroll.handle.set_offset(Point::default());
        self.group_scroll.reset();
        cx.notify();
    }

    fn exit_group(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
        self.open_group = None;
        self.group_scroll.reset();
        cx.notify();
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
        if self.library.settings().remember_zoom {
            if let Some(reader) = self.reader.as_ref() {
                self.library.settings_mut().zoom = reader.zoom;
                self.save_library_debounced(cx);
            }
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

    fn set_remember_zoom(&mut self, value: Option<bool>, cx: &mut Context<Self>) {
        let next = value.unwrap_or(!self.library.settings().remember_zoom);
        self.library.settings_mut().remember_zoom = next;
        if next {
            if let Some(reader) = self.reader.as_ref() {
                self.library.settings_mut().zoom = reader.zoom;
            }
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
        self.save_library();
        let message = match (enabled, self.reduce_motion) {
            (true, true) => "animations on (the desktop asks for reduced motion, so they stay off)",
            (true, false) => "animations on",
            (false, _) => "animations off",
        };
        self.show_hud(message, cx);
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
        let Some(reader) = self.reader.as_mut() else {
            return;
        };

        let now = Instant::now();
        if touch_phase == TouchPhase::Started || touch_phase == TouchPhase::Cancelled || discrete {
            reader.kinetic.reset();
        } else if reader.kinetic.has_active_inertia() {
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

        if touch_phase == TouchPhase::Ended {
            if reader
                .kinetic
                .end_touch(reader.animate && !self.reduce_motion)
            {
                self.wake();
                cx.notify();
                return;
            }
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
                ThumbnailSize::ExtraSmall => Some(ThumbnailSize::Small),
                ThumbnailSize::Small => Some(ThumbnailSize::Medium),
                ThumbnailSize::Medium => Some(ThumbnailSize::Large),
                ThumbnailSize::Large => Some(ThumbnailSize::ExtraLarge),
                ThumbnailSize::ExtraLarge => Some(ThumbnailSize::Huge),
                ThumbnailSize::Huge => None,
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
                ThumbnailSize::Huge => Some(ThumbnailSize::ExtraLarge),
                ThumbnailSize::ExtraLarge => Some(ThumbnailSize::Large),
                ThumbnailSize::Large => Some(ThumbnailSize::Medium),
                ThumbnailSize::Medium => Some(ThumbnailSize::Small),
                ThumbnailSize::Small => Some(ThumbnailSize::ExtraSmall),
                ThumbnailSize::ExtraSmall => None,
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
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let Some(drag) = reader.drag.take() else {
            return;
        };
        if drag.moved {
            if reader
                .kinetic
                .end_touch(reader.animate && !self.reduce_motion)
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

        let Some(reader) = self.reader.as_mut() else {
            if token == ":" && !self.search_active {
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
        let actions = reader.vim.feed(&token);
        self.command_selection = 0;
        cx.notify();
        for action in actions {
            self.apply(action, vp, window, cx);
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
                    self.settings_tab = SettingsTab::Keyboard;
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
            this.update(cx, |this, cx| {
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

    fn render_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(match self.overlay.as_ref()? {
            Overlay::Menu(menu) => self.render_menu(menu, cx),
            Overlay::LibraryMenu(menu) => self.render_library_menu(menu, cx),
            Overlay::Confirm(dialog) => self.render_confirm(dialog, cx),
            Overlay::Properties(dialog) => self.render_properties(dialog, cx),
            Overlay::Series(dialog) => self.render_series_dialog(dialog, cx),
            Overlay::Settings => self.render_settings(cx),
        })
    }

    /// Full-window layer behind a dialog: swallows clicks meant for the view underneath and
    /// closes the dialog when clicked. `dim` darkens the view behind it.
    fn scrim(&self, dim: bool, cx: &mut Context<Self>) -> gpui_kit::Stateful<gpui_kit::Div> {
        // Explicit insets matter: an absolutely positioned child without them is placed after the
        // in-flow content (below the window) instead of covering it.
        div()
            .id("overlay-scrim")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(with_alpha(
                overlay_shade(self.colors),
                if dim { 0.50 } else { 0.0 },
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_overlay(cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_overlay(cx)),
            )
    }

    fn render_menu(&self, menu: &BookMenu, cx: &mut Context<Self>) -> AnyElement {
        let border_px = self.border_px();
        let rounding_lg = self.radius_lg_px();
        let mut list = div()
            .min_w(px(200.))
            .max_w(px(320.))
            .w_auto()
            .flex()
            .flex_col()
            .p_1()
            .rounded(px(rounding_lg))
            .bg(self.menu_surface())
            .when(border_px > 0.0, |this| {
                this.border(px(border_px)).border_color(self.colors.accent)
            })
            .shadow_lg()
            .text_sm()
            .text_color(self.colors.foreground)
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            })
            .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            });

        for (ix, entry) in menu.entries.iter().enumerate() {
            list = match entry {
                Entry::Separator => list,
                Entry::Item(item) => list.child(self.menu_item(menu, ix, item, cx)),
            };
        }

        #[cfg(test)]
        let list = list.debug_selector(|| "book-menu".into());

        // `anchored` keeps the menu inside the window (it slides back in near the edges) and
        // `deferred` paints it above everything else.
        deferred(
            div()
                .id("book-menu-scrim")
                .occlude()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_overlay(cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_overlay(cx)),
                )
                .child(
                    anchored()
                        .position(menu.position)
                        .anchor(Anchor::TopLeft)
                        .snap_to_window_with_margin(px(8.))
                        .child(list),
                ),
        )
        .with_priority(1)
        .into_any_element()
    }

    fn render_library_menu(&self, menu: &LibraryMenu, cx: &mut Context<Self>) -> AnyElement {
        let entries = [
            (
                IconName::FilePlus,
                "Add files",
                "Import CBZ, CBR and ZIP comics",
            ),
            (
                IconName::FolderPlus,
                "Add folder",
                "Import a folder of images",
            ),
            (
                IconName::Settings,
                "Settings",
                "Themes and reader preferences",
            ),
            (
                IconName::Folder,
                if self.library.settings().group_series {
                    "Show individual volumes"
                } else {
                    "Group into folders"
                },
                if self.library.settings().group_series {
                    "Display books ungrouped"
                } else {
                    "Group volumes by folder"
                },
            ),
            (IconName::LogOut, "Quit", "Close yomu"),
        ];
        let border_px = self.border_px();
        let rounding_lg = self.radius_lg_px();
        let rounding_md = self.radius_px();
        let mut list = div()
            .min_w(px(240.))
            .max_w(px(360.))
            .w_auto()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded(px(rounding_lg))
            .bg(self.menu_surface())
            .when(border_px > 0.0, |this| {
                this.border(px(border_px)).border_color(self.colors.accent)
            })
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            })
            .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            });
        for (index, (icon, label, description)) in entries.into_iter().enumerate() {
            let selected = menu.selected == index;
            list = list.child(
                div()
                    .id(SharedString::from(format!("library-menu-{index}")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded(px(rounding_md))
                    .cursor_pointer()
                    .when(selected, |row| row.bg(self.colors.accent))
                    .when(!selected, |row| {
                        row.hover(|style| style.bg(self.colors.surface_hover))
                    })
                    .child(Icon::new(icon).size_4().text_color(if selected {
                        self.colors.accent_foreground
                    } else {
                        self.colors.foreground
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(if selected {
                                        self.colors.accent_foreground
                                    } else {
                                        self.colors.foreground
                                    })
                                    .child(label),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if selected {
                                        self.colors.accent_foreground
                                    } else {
                                        self.colors.muted
                                    })
                                    .truncate()
                                    .child(description),
                            ),
                    )
                    .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                        if let Some(Overlay::LibraryMenu(menu)) = this.overlay.as_mut() {
                            menu.selected = index;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.run_library_menu_action(index, window, cx);
                    })),
            );
        }

        deferred(
            div()
                .id("library-menu-scrim")
                .occlude()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_overlay(cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_overlay(cx)),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(52.))
                        .right(px(18.))
                        .child(self.animate_popup("library-menu-popup", list)),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }

    fn menu_item(
        &self,
        menu: &BookMenu,
        ix: usize,
        item: &Item,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let icon = match item.action {
            BookAction::Open => IconName::BookOpen,
            BookAction::StartOver | BookAction::ResetProgress => IconName::RotateCcw,
            BookAction::MarkRead => IconName::CircleCheck,
            BookAction::MarkUnread => IconName::CircleX,
            BookAction::RevealFile => IconName::FolderOpen,
            BookAction::Properties => IconName::Info,
            BookAction::SetSeries => IconName::Library,
            BookAction::Remove | BookAction::RemoveSelected => IconName::Trash,
            BookAction::ClearSelection => IconName::X,
        };
        let is_selected = menu.selected == Some(ix);
        let item_color = if !item.enabled {
            self.colors.muted
        } else if is_selected {
            self.colors.accent_foreground
        } else if item.danger {
            self.colors.danger
        } else {
            self.colors.foreground
        };
        let rounding_md = self.radius_px();
        let base = div()
            .id(SharedString::from(format!("menu-item-{ix}")))
            .h(px(28.))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(rounding_md))
            .child(
                div()
                    .w(px(20.))
                    .h(px(20.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(icon).size_4().text_color(item_color)),
            )
            .child(item.label.clone());

        if !item.enabled {
            return base
                .text_color(self.colors.muted)
                .opacity(0.6)
                .into_any_element();
        }

        let action = item.action;
        let book_id = menu.book_id.clone();
        base.text_color(if is_selected {
            self.colors.accent_foreground
        } else if item.danger {
            self.colors.danger
        } else {
            self.colors.foreground
        })
        .cursor_pointer()
        .when(is_selected, |row| row.bg(self.colors.accent))
        .when(!is_selected, |row| {
            row.hover(|style| style.bg(self.colors.surface_hover))
        })
        .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| this.menu_hover(ix, cx)))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.run_book_action(action, &book_id, window, cx);
        }))
        .into_any_element()
    }

    fn panel(&self, width: f32) -> gpui_kit::Stateful<gpui_kit::Div> {
        let border_px = self.border_px();
        let rounding_xl = self.radius_xl_px();
        div()
            .id(SharedString::from(format!("panel-{}", width as u32)))
            .occlude()
            .w(px(width))
            .max_w_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .rounded(px(rounding_xl))
            .bg(self.widget_surface(self.colors.surface))
            .when(border_px > 0.0, |this| {
                this.border(px(border_px)).border_color(self.colors.accent)
            })
            .shadow_lg()
            .text_sm()
            .text_color(self.colors.foreground)
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            })
            .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            })
    }

    fn animate_popup<E>(&self, id: &'static str, surface: E) -> gpui_kit::AnimationElement<E>
    where
        E: IntoElement + Styled + 'static,
    {
        surface.with_animation(
            id,
            Animation::new(Duration::from_millis(if self.reduce_motion {
                1
            } else {
                180
            }))
            .with_easing(ease_out_quint()),
            |surface, progress| {
                surface
                    .opacity(progress)
                    .relative()
                    .top(px((1.0 - progress) * 9.0))
            },
        )
    }

    fn render_confirm(&self, dialog: &ConfirmDialog, cx: &mut Context<Self>) -> AnyElement {
        self.scrim(true, cx)
            .child(
                self.animate_popup("confirm-dialog", self.panel(380.))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(dialog.title.clone()),
                    )
                    .child(
                        div()
                            .text_color(self.colors.muted)
                            .child(dialog.message.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap_2()
                            .child(panel_button(
                                "confirm-cancel",
                                "Cancel",
                                ButtonKind::Normal,
                                self.colors,
                                true,
                                cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx)),
                            ))
                            .child(panel_button(
                                "confirm-accept",
                                dialog.confirm_label,
                                if matches!(dialog.action, ConfirmAction::ClearAllData) {
                                    ButtonKind::Danger
                                } else {
                                    ButtonKind::Primary
                                },
                                self.colors,
                                true,
                                cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.confirm_accept(window, cx);
                                }),
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_properties(&self, dialog: &PropertiesDialog, cx: &mut Context<Self>) -> AnyElement {
        let rows = dialog.rows.iter().map(|(label, value)| {
            div()
                .flex()
                .flex_row()
                .gap_4()
                .child(
                    div()
                        .w(px(84.))
                        .flex_none()
                        .text_color(self.colors.muted)
                        .child(label.clone()),
                )
                .child(div().flex_1().min_w_0().truncate().child(value.clone()))
        });

        self.scrim(true, cx)
            .child(
                self.animate_popup("properties-dialog", self.panel(480.))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(dialog.title.clone()),
                    )
                    .child(div().flex().flex_col().gap_2().children(rows))
                    .child(div().flex().flex_row().justify_end().child(panel_button(
                        "properties-close",
                        "Close",
                        ButtonKind::Normal,
                        self.colors,
                        true,
                        cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx)),
                    ))),
            )
            .into_any_element()
    }

    fn render_series_dialog(&self, dialog: &SeriesDialog, cx: &mut Context<Self>) -> AnyElement {
        self.scrim(true, cx)
            .child(
                self.animate_popup("series-dialog", self.panel(460.))
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if dialog.book_ids.len() == 1 {
                                "Set series"
                            } else {
                                "Set series for selected books"
                            }),
                    )
                    .child(div().text_color(self.colors.muted).child(
                        "A folder name is suggested when the books share a parent folder. Edit it, or clear the field to leave them ungrouped.",
                    ))
                    .child(
                        div()
                            .w_full()
                            .rounded(px(self.radius_px()))
                            .border_1()
                            .border_color(self.colors.accent)
                            .child(
                                Input::new(&dialog.input)
                                    .id("series-name-input")
                                    .cleanable(true)
                                    .bordered(false)
                                    .focus_bordered(false)
                                    .w_full(),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .items_center()
                            .child(div().text_xs().text_color(self.colors.muted).child(
                                "Type · Enter to save · Esc to cancel",
                            ))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_2()
                                    .child(panel_button(
                                        "series-cancel",
                                        "Cancel",
                                        ButtonKind::Normal,
                                        self.colors,
                                        true,
                                        cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.close_overlay(cx)
                                        }),
                                    ))
                                    .child(panel_button(
                                        "series-save",
                                        "Save",
                                        ButtonKind::Primary,
                                        self.colors,
                                        true,
                                        cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.save_series_dialog(cx)
                                        }),
                                    )),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.library.settings();
        let theme_name = settings.theme_name.clone();
        let theme_path = self.custom_theme_path.display().to_string();
        let theme_status = self.theme_status.clone();
        let border_size = settings.border_size;
        let corner_rounding = settings.corner_rounding;
        let continuous = settings.continuous;
        let webcomics = settings.webcomics;
        let fit = settings.fit;
        let remember_zoom = settings.remember_zoom;
        let zoom_val = settings.zoom;
        let animate = settings.animate;
        let grouped = settings.group_series;
        let thumb_size = settings.thumbnail_size;
        let is_default = *settings == Settings::default();
        let cache_bytes = self.dirs.cache_size_bytes();
        let book_count = self.library.books().len();

        let yomu = cx.entity();
        let tab_bar = TabBar::new("settings-tab-bar")
            .underline()
            .selected_index(self.settings_tab.to_index())
            .on_click({
                let yomu = yomu.clone();
                move |&idx, _window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.settings_tab = SettingsTab::from_index(idx);
                        cx.notify();
                    });
                }
            })
            .child(
                Tab::new()
                    .label("Appearance")
                    .prefix(Icon::new(IconName::Palette))
                    .cursor_pointer(),
            )
            .child(
                Tab::new()
                    .label("Library")
                    .prefix(Icon::new(IconName::Library))
                    .cursor_pointer(),
            )
            .child(
                Tab::new()
                    .label("Reader")
                    .prefix(Icon::new(IconName::BookOpen))
                    .cursor_pointer(),
            )
            .child(
                Tab::new()
                    .label("Storage")
                    .prefix(Icon::new(IconName::HardDrive))
                    .cursor_pointer(),
            )
            .child(
                Tab::new()
                    .label("Shortcuts")
                    .prefix(Icon::new(IconName::Keyboard))
                    .cursor_pointer(),
            );

        let border_px = self.border_px();
        let rounding_xl = self.radius_xl_px();
        let rounding_md = self.radius_px();
        let adw_group = move |title: &'static str, rows: Vec<AnyElement>| {
            let section_slug = title
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect::<String>();
            let section_id = SharedString::from(format!("settings-section-{section_slug}"));
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .px_1()
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(self.colors.muted)
                        .child(title),
                )
                .child(
                    div()
                        .id(section_id)
                        .flex()
                        .flex_col()
                        .rounded(px(rounding_xl))
                        .overflow_hidden()
                        .when(border_px > 0.0, |this| {
                            this.border(px(border_px)).border_color(self.colors.border)
                        })
                        .hover(move |style| {
                            if border_px > 0.0 {
                                style.border_color(self.colors.accent)
                            } else {
                                style
                            }
                        })
                        .bg(self.widget_surface(self.colors.surface))
                        .children(rows),
                )
        };

        let adw_row = move |icon: IconName,
                            title: &'static str,
                            subtitle: Option<String>,
                            control: AnyElement,
                            is_last: bool| {
            let mut row = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px_4()
                .py_3()
                .gap_4();
            if !is_last {
                row = row.border_b_1().border_color(self.colors.border);
            }
            row.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .p_2()
                            .rounded(px(rounding_md))
                            .bg(self.colors.surface_hover)
                            .text_color(self.colors.accent)
                            .child(Icon::new(icon).size(px(16.))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(self.colors.foreground)
                                    .child(title),
                            )
                            .children(subtitle.map(|sub| {
                                div().text_xs().text_color(self.colors.muted).child(sub)
                            })),
                    ),
            )
            .child(control)
            .into_any_element()
        };

        fn dropdown_item(
            opt_label: &'static str,
            is_current: bool,
            on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        ) -> PopupMenuItem {
            PopupMenuItem::element(move |_, _| div().cursor_pointer().flex_1().child(opt_label))
                .checked(is_current)
                .on_click(on_click)
        }

        let yomu = cx.entity();
        let body = match self.settings_tab {
            SettingsTab::Appearance => {
                let theme_row = adw_row(
                    IconName::Palette,
                    "Color Theme",
                    Some("Select the theme used for all windows and controls".into()),
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(panel_button(
                            "settings-theme-previous",
                            "‹",
                            ButtonKind::Normal,
                            self.colors,
                            true,
                            cx.listener(|this, _: &ClickEvent, _, cx| this.cycle_theme(-1, cx)),
                        ))
                        .child(
                            div()
                                .min_w(px(120.))
                                .text_center()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(self.colors.foreground)
                                .child(theme_name),
                        )
                        .child(panel_button(
                            "settings-theme-next",
                            "›",
                            ButtonKind::Normal,
                            self.colors,
                            true,
                            cx.listener(|this, _: &ClickEvent, _, cx| this.cycle_theme(1, cx)),
                        ))
                        .into_any_element(),
                    false,
                );

                const BORDER_OPTIONS: &[(BorderSize, &str)] = &[
                    (BorderSize::None, "None (0px)"),
                    (BorderSize::Thin, "Thin (1px)"),
                    (BorderSize::Medium, "Medium (2px)"),
                    (BorderSize::Thick, "Thick (3px)"),
                    (BorderSize::ExtraThick, "Extra Thick (4px)"),
                ];
                let border_dropdown = Button::new("settings-border-size")
                    .label(border_size.label())
                    .dropdown_caret(true)
                    .rounded(ButtonRounded::Medium)
                    .small()
                    .secondary()
                    .outline()
                    .bg(self.colors.surface)
                    .border_color(self.colors.border)
                    .cursor_pointer()
                    .dropdown_menu_with_anchor(Anchor::TopRight, {
                        let yomu = yomu.clone();
                        move |mut menu, _window, _cx| {
                            for &(val, opt_label) in BORDER_OPTIONS {
                                let yomu = yomu.clone();
                                let is_current = val == border_size;
                                menu = menu.item(dropdown_item(
                                    opt_label,
                                    is_current,
                                    move |_, _window, cx| {
                                        yomu.update(cx, |this, cx| {
                                            this.set_border_size(val, cx);
                                        });
                                    },
                                ));
                            }
                            menu
                        }
                    });
                let border_row = adw_row(
                    IconName::Layers,
                    "Border Thickness",
                    Some("Border width for cards, dialogs, and controls".into()),
                    border_dropdown.into_any_element(),
                    false,
                );

                const ROUNDING_OPTIONS: &[(CornerRounding, &str)] = &[
                    (CornerRounding::Sharp, "Sharp (0px)"),
                    (CornerRounding::Subtle, "Subtle (4px)"),
                    (CornerRounding::Medium, "Medium (8px)"),
                    (CornerRounding::Soft, "Soft (14px)"),
                    (CornerRounding::Round, "Round (20px)"),
                    (CornerRounding::ExtraSoft, "Extra Soft (28px)"),
                ];
                let rounding_dropdown = Button::new("settings-corner-rounding")
                    .label(corner_rounding.label())
                    .dropdown_caret(true)
                    .rounded(ButtonRounded::Medium)
                    .small()
                    .secondary()
                    .outline()
                    .bg(self.colors.surface)
                    .border_color(self.colors.border)
                    .cursor_pointer()
                    .dropdown_menu_with_anchor(Anchor::TopRight, {
                        let yomu = yomu.clone();
                        move |mut menu, _window, _cx| {
                            for &(val, opt_label) in ROUNDING_OPTIONS {
                                let yomu = yomu.clone();
                                let is_current = val == corner_rounding;
                                menu = menu.item(dropdown_item(
                                    opt_label,
                                    is_current,
                                    move |_, _window, cx| {
                                        yomu.update(cx, |this, cx| {
                                            this.set_corner_rounding(val, cx);
                                        });
                                    },
                                ));
                            }
                            menu
                        }
                    });
                let rounding_row = adw_row(
                    IconName::Maximize2,
                    "Corner Rounding",
                    Some("Corner radius style across the user interface".into()),
                    rounding_dropdown.into_any_element(),
                    false,
                );

                let reload_row = adw_row(
                    IconName::FolderOpen,
                    "Custom Themes Directory",
                    Some(theme_path),
                    panel_button(
                        "settings-theme-reload",
                        "Reload Themes",
                        ButtonKind::Normal,
                        self.colors,
                        true,
                        cx.listener(|this, _: &ClickEvent, _, cx| this.reload_custom_themes(cx)),
                    ),
                    true,
                );
                let mut app_group = adw_group(
                    "APPEARANCE",
                    vec![theme_row, border_row, rounding_row, reload_row],
                );
                if let Some(status) = theme_status {
                    app_group = app_group.child(
                        div()
                            .px_1()
                            .text_xs()
                            .text_color(self.colors.muted)
                            .child(status),
                    );
                }
                app_group.into_any_element()
            }
            SettingsTab::Library => {
                const THUMB_OPTIONS: &[(ThumbnailSize, &str)] = &[
                    (ThumbnailSize::ExtraSmall, "Extra Small (150px)"),
                    (ThumbnailSize::Small, "Small (180px)"),
                    (ThumbnailSize::Medium, "Medium (210px)"),
                    (ThumbnailSize::Large, "Large (240px)"),
                    (ThumbnailSize::ExtraLarge, "Extra Large (280px)"),
                    (ThumbnailSize::Huge, "Huge (340px)"),
                ];
                let thumb_dropdown = Button::new("settings-thumbnail-size")
                    .label(thumb_size.label())
                    .dropdown_caret(true)
                    .rounded(ButtonRounded::Medium)
                    .small()
                    .secondary()
                    .outline()
                    .bg(self.colors.surface)
                    .border_color(self.colors.border)
                    .cursor_pointer()
                    .dropdown_menu_with_anchor(Anchor::TopRight, {
                        let yomu = yomu.clone();
                        move |mut menu, _window, _cx| {
                            for &(val, opt_label) in THUMB_OPTIONS {
                                let yomu = yomu.clone();
                                let is_current = val == thumb_size;
                                menu = menu.item(dropdown_item(
                                    opt_label,
                                    is_current,
                                    move |_, _window, cx| {
                                        yomu.update(cx, |this, cx| {
                                            this.set_thumbnail_size(val, cx);
                                        });
                                    },
                                ));
                            }
                            menu
                        }
                    });
                let thumb_row = adw_row(
                    IconName::Layers,
                    "Thumbnail Card Size",
                    Some("Size of cover art cards displayed in the library grid".into()),
                    thumb_dropdown.into_any_element(),
                    false,
                );
                let group_switch = Switch::new("settings-group-series")
                    .checked(grouped)
                    .small()
                    .cursor_pointer()
                    .on_click({
                        let yomu = yomu.clone();
                        move |&checked, _window, cx| {
                            yomu.update(cx, |this, cx| {
                                this.library.settings_mut().group_series = checked;
                                this.selection.clear();
                                this.open_group = None;
                                this.group_scroll.reset();
                                this.save_library();
                                cx.notify();
                            });
                        }
                    });
                let group_row = adw_row(
                    IconName::FolderArchive,
                    "Folder Grouping",
                    Some("Group volumes from the same directory into folder cards".into()),
                    group_switch.into_any_element(),
                    true,
                );
                adw_group("LIBRARY DISPLAY & LAYOUT", vec![thumb_row, group_row]).into_any_element()
            }
            SettingsTab::Reader => {
                const FIT_OPTIONS: &[(FitMode, &str)] = &[
                    (FitMode::Page, "Fit Page"),
                    (FitMode::Width, "Fit Width"),
                    (FitMode::Height, "Fit Height"),
                    (FitMode::Original, "Original Size"),
                ];
                let is_custom_zoom = remember_zoom && (zoom_val - 1.0).abs() > 0.01;
                let fit_subtitle = if is_custom_zoom {
                    format!(
                        "Baseline mode: {} (custom zoom active: {}%)",
                        fit.label(),
                        (zoom_val * 100.0).round() as i32
                    )
                } else if remember_zoom {
                    format!("Baseline layout mode: {}", fit.label())
                } else {
                    "Layout mode used when opening any book (resets to 100%)".to_string()
                };

                let fit_dropdown = Button::new("settings-fit")
                    .label(fit.label())
                    .dropdown_caret(true)
                    .rounded(ButtonRounded::Medium)
                    .small()
                    .secondary()
                    .outline()
                    .bg(self.colors.surface)
                    .border_color(self.colors.border)
                    .cursor_pointer()
                    .dropdown_menu_with_anchor(Anchor::TopRight, {
                        let yomu = yomu.clone();
                        move |mut menu, _window, _cx| {
                            for &(mode, opt_label) in FIT_OPTIONS {
                                let yomu = yomu.clone();
                                let is_current = mode == fit && !is_custom_zoom;
                                menu = menu.item(dropdown_item(
                                    opt_label,
                                    is_current,
                                    move |_, _window, cx| {
                                        yomu.update(cx, |this, cx| {
                                            this.set_fit(mode, cx);
                                        });
                                    },
                                ));
                            }
                            menu
                        }
                    });

                let mut fit_trailing = div().flex().items_center().gap_2();
                if is_custom_zoom {
                    let yomu = yomu.clone();
                    fit_trailing = fit_trailing.child(
                        Button::new("settings-reset-zoom")
                            .label("Reset 100%")
                            .rounded(ButtonRounded::Medium)
                            .small()
                            .ghost()
                            .cursor_pointer()
                            .on_click(move |_, _window, cx| {
                                yomu.update(cx, |this, cx| {
                                    this.reset_zoom(cx);
                                });
                            }),
                    );
                }
                fit_trailing = fit_trailing.child(fit_dropdown);

                let fit_row = adw_row(
                    IconName::Maximize2,
                    "Default Page Fit",
                    Some(fit_subtitle),
                    fit_trailing.into_any_element(),
                    false,
                );

                let zoom_switch = Switch::new("settings-remember-zoom")
                    .checked(remember_zoom)
                    .small()
                    .cursor_pointer()
                    .on_click({
                        let yomu = yomu.clone();
                        move |&checked, _window, cx| {
                            yomu.update(cx, |this, cx| {
                                this.set_remember_zoom(Some(checked), cx);
                            });
                        }
                    });
                let zoom_row = adw_row(
                    IconName::ZoomIn,
                    "Remember Zoom Level",
                    Some(
                        "Retain custom zoom and pinch magnification across all books instead of resetting on open".into(),
                    ),
                    zoom_switch.into_any_element(),
                    false,
                );
                let anim_switch = Switch::new("settings-animations")
                    .checked(animate)
                    .small()
                    .cursor_pointer()
                    .on_click({
                        let yomu = yomu.clone();
                        move |&checked, _window, cx| {
                            yomu.update(cx, |this, cx| {
                                this.set_animations(Some(checked), cx);
                            });
                        }
                    });
                let anim_row = adw_row(
                    IconName::Sparkles,
                    "Page Animations",
                    Some("Slide transitions between page turns and smooth scroll gliding".into()),
                    anim_switch.into_any_element(),
                    false,
                );
                let cont_switch = Switch::new("settings-continuous")
                    .checked(continuous)
                    .small()
                    .cursor_pointer()
                    .on_click({
                        let yomu = yomu.clone();
                        move |&checked, _window, cx| {
                            yomu.update(cx, |this, cx| {
                                this.set_continuous(Some(checked), cx);
                            });
                        }
                    });
                let cont_row = adw_row(
                    IconName::AlignVerticalSpaceBetween,
                    "Continuous Vertical Mode",
                    Some("Stack pages vertically and scroll through the whole book".into()),
                    cont_switch.into_any_element(),
                    false,
                );
                let webc_switch = Switch::new("settings-webcomics")
                    .checked(webcomics)
                    .small()
                    .cursor_pointer()
                    .on_click({
                        let yomu = yomu.clone();
                        move |&checked, _window, cx| {
                            yomu.update(cx, |this, cx| {
                                this.set_webcomics(Some(checked), cx);
                            });
                        }
                    });
                let webc_row = adw_row(
                    IconName::Film,
                    "Webcomics Mode",
                    Some(
                        "Forces a connected, gapless vertical strip for continuous reading".into(),
                    ),
                    webc_switch.into_any_element(),
                    true,
                );
                adw_group(
                    "READING BEHAVIOR",
                    vec![fit_row, zoom_row, anim_row, cont_row, webc_row],
                )
                .into_any_element()
            }
            SettingsTab::Storage => {
                let cache_row = adw_row(
                    IconName::HardDrive,
                    "Thumbnail Cache",
                    Some(format!(
                        "{} used across cached cover sizes",
                        format_size(cache_bytes)
                    )),
                    panel_button(
                        "settings-clear-cache",
                        "Clear Cache",
                        ButtonKind::Normal,
                        self.colors,
                        true,
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.request_clear_thumbnail_cache(cx)
                        }),
                    ),
                    false,
                );
                let data_row = adw_row(
                    IconName::Database,
                    "Application Data",
                    Some(format!(
                        "{} comics registered · permanently wipes library and progress",
                        book_count
                    )),
                    panel_button(
                        "settings-clear-all",
                        "Reset All Data",
                        ButtonKind::Danger,
                        self.colors,
                        true,
                        cx.listener(|this, _: &ClickEvent, _, cx| this.request_clear_all_data(cx)),
                    ),
                    true,
                );
                adw_group("STORAGE & DATA MANAGEMENT", vec![cache_row, data_row]).into_any_element()
            }
            SettingsTab::Keyboard => {
                let mut body = TableBody::new();

                for (section_idx, (section, rows)) in HELP.iter().enumerate() {
                    let section_cell = TableCell::new().col_span(2).child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .w_full()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(self.colors.accent)
                                    .child(section.to_uppercase()),
                            )
                            .child(
                                Tag::secondary()
                                    .small()
                                    .child(format!("{} shortcuts", rows.len())),
                            ),
                    );

                    body = body.child(
                        TableRow::new()
                            .bg(self.colors.surface_hover)
                            .when(section_idx > 0, |row| {
                                row.border_t_1().border_color(self.colors.border)
                            })
                            .child(section_cell),
                    );

                    for (keys, description) in rows.iter() {
                        let desc_cell = TableCell::new().child(
                            div()
                                .text_xs()
                                .text_color(self.colors.foreground)
                                .child(*description),
                        );

                        let key_id =
                            SharedString::from(format!("shortcut-key-{section_idx}-{keys}"));
                        let key_cell = TableCell::new().text_right().child(
                            div()
                                .id(key_id)
                                .px_2()
                                .py_0p5()
                                .rounded(px(rounding_md))
                                .when(border_px > 0.0, |this| {
                                    this.border(px(border_px))
                                        .border_color(with_alpha(self.colors.border, 0.7))
                                })
                                .hover(move |style| {
                                    if border_px > 0.0 {
                                        style.border_color(self.colors.accent)
                                    } else {
                                        style
                                    }
                                })
                                .bg(self.colors.surface)
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(self.colors.accent)
                                .child(*keys),
                        );

                        body = body.child(TableRow::new().child(desc_cell).child(key_cell));
                    }
                }

                let table = Table::new()
                    .small()
                    .child(
                        TableHeader::new().child(
                            TableRow::new()
                                .child(TableHead::new().child("ACTION"))
                                .child(TableHead::new().text_right().child("SHORTCUT")),
                        ),
                    )
                    .child(body);

                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .px_1()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(self.colors.muted)
                            .child("KEYBOARD SHORTCUTS & COMMANDS"),
                    )
                    .child(
                        div()
                            .id("settings-shortcuts-card")
                            .flex()
                            .flex_col()
                            .rounded(px(rounding_xl))
                            .overflow_hidden()
                            .when(border_px > 0.0, |this| {
                                this.border(px(border_px)).border_color(self.colors.border)
                            })
                            .hover(move |style| {
                                if border_px > 0.0 {
                                    style.border_color(self.colors.accent)
                                } else {
                                    style
                                }
                            })
                            .bg(self.widget_surface(self.colors.surface))
                            .child(
                                div()
                                    .id("settings-shortcuts-scroll")
                                    .track_scroll(&self.shortcuts_scroll.handle)
                                    .h(px(320.))
                                    .overflow_y_scroll()
                                    .on_scroll_wheel(cx.listener(
                                        |this, event: &ScrollWheelEvent, _, cx| {
                                            this.shortcuts_scroll
                                                .handle_scroll_wheel(event, this.reduce_motion);
                                            this.wake();
                                            cx.notify();
                                        },
                                    ))
                                    .child(table),
                            ),
                    )
                    .into_any_element()
            }
        };

        self.scrim(true, cx)
            .child(
                self.animate_popup("settings-dialog", self.panel(680.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(self.colors.foreground)
                                    .child("Settings"),
                            ),
                    )
                    .child(tab_bar)
                    .child(div().min_h(px(280.)).child(body))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .items_center()
                            .pt_2()
                            .border_t_1()
                            .border_color(self.colors.border)
                            .child(panel_button(
                                "settings-reset",
                                "Reset to Defaults",
                                ButtonKind::Normal,
                                self.colors,
                                !is_default,
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.request_reset_settings(cx)
                                }),
                            ))
                            .child(panel_button(
                                "settings-done",
                                "Done",
                                ButtonKind::Primary,
                                self.colors,
                                true,
                                cx.listener(|this, _: &ClickEvent, _, cx| this.close_overlay(cx)),
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_library(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let all_books = self.library.books();
        let count = all_books.len();
        let books: Vec<&Book> = all_books
            .iter()
            .filter(|book| {
                search::matches(
                    &book.title,
                    &book.path,
                    book.series.as_deref(),
                    &self.search_query,
                )
            })
            .collect();

        let thumb_size = self.library.settings().thumbnail_size;
        let card_w = thumb_size.card_width();
        let gap = 24.0;
        let pad_x = 24.0;
        let vp_w: f32 = window.viewport_size().width.into();
        let avail_w = (vp_w - pad_x * 2.0).max(card_w);
        let cols = ((avail_w + gap) / (card_w + gap)).floor().max(1.0) as usize;
        let grid_w = (cols as f32 * card_w + (cols.saturating_sub(1) as f32) * gap).min(avail_w);

        let make_grid = |elements: Vec<AnyElement>| {
            div().w_full().flex().flex_row().justify_center().child(
                div()
                    .w(px(grid_w))
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_6()
                    .children(elements),
            )
        };
        let status: Option<SharedString> = if self.importing {
            Some(
                self.importing_title
                    .as_ref()
                    .map(|title| format!("{} imported · {title}", self.imported_count))
                    .unwrap_or_else(|| "Scanning folder…".to_string())
                    .into(),
            )
        } else {
            self.toast.clone()
        };

        let view_order = self.current_view_order();
        let view_count = view_order.len();
        let selected = self.selected_ids().len();
        let rounding_md = self.radius_px();
        let is_search_focused = self.search_active;

        let search_field: AnyElement = if let Some(input) = &self.search_input {
            div()
                .id("library-search-wrapper")
                .w(px(360.))
                .rounded(px(rounding_md))
                .border_1()
                .border_color(if is_search_focused {
                    self.colors.accent
                } else {
                    self.colors.border
                })
                .hover(|style| {
                    if !is_search_focused {
                        style.border_color(self.colors.accent)
                    } else {
                        style
                    }
                })
                .child(
                    Input::new(input)
                        .id("library-search")
                        .cleanable(true)
                        .bordered(false)
                        .focus_bordered(false)
                        .prefix(Icon::new(IconName::Search).size(px(16.)))
                        .w_full(),
                )
                .into_any_element()
        } else {
            div().id("library-search").w(px(360.)).into_any_element()
        };

        let selection_controls = if selected > 0 {
            let controls = div().flex().flex_row().items_center().gap_2();
            let controls = if selected < view_count {
                controls.child(panel_button(
                    "select-all",
                    "Select all",
                    ButtonKind::Normal,
                    self.colors,
                    true,
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        let order = this.current_view_order();
                        this.selection.select_all(&order);
                        cx.notify();
                    }),
                ))
            } else {
                controls
            };
            Some(
                controls
                    .child(panel_button(
                        "select-remove",
                        "Remove",
                        ButtonKind::Danger,
                        self.colors,
                        true,
                        cx.listener(|this, _: &ClickEvent, _, cx| this.request_remove_selected(cx)),
                    ))
                    .child(panel_button(
                        "select-cancel",
                        "Cancel",
                        ButtonKind::Normal,
                        self.colors,
                        true,
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.selection.clear();
                            cx.notify();
                        }),
                    )),
            )
        } else {
            None
        };

        let back_button = (self.open_group.is_some()).then(|| {
            icon_button(
                "group-back",
                "Back to library",
                IconName::ArrowLeft,
                self.colors,
                false,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.exit_group(cx);
                }),
            )
        });

        let left_controls = match (back_button, selection_controls) {
            (Some(back), Some(sel)) => Some(
                div()
                    .absolute()
                    .left_6()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(back)
                    .child(sel),
            ),
            (Some(back), None) => Some(
                div()
                    .absolute()
                    .left_6()
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(back),
            ),
            (None, Some(sel)) => Some(
                div()
                    .absolute()
                    .left_6()
                    .flex()
                    .flex_row()
                    .items_center()
                    .child(sel),
            ),
            (None, None) => None,
        };

        let menu_open = matches!(self.overlay, Some(Overlay::LibraryMenu(_)));
        let header = div()
            .relative()
            .w_full()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(72.))
            .px_6()
            .children(left_controls)
            .when(selected == 0 && self.open_group.is_none(), |h| {
                #[cfg(test)]
                {
                    h.child(div().debug_selector(|| "add-files".into()).size_0())
                }
                #[cfg(not(test))]
                {
                    h
                }
            })
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(search_field),
            )
            .child(div().flex_1().flex().justify_end().child(icon_button(
                "library-menu-toggle",
                "Open menu",
                IconName::Menu,
                self.colors,
                menu_open,
                cx.listener(|this, _: &ClickEvent, window, cx| {
                    if matches!(this.overlay, Some(Overlay::LibraryMenu(_))) {
                        this.close_overlay(cx);
                    } else {
                        this.open_library_menu(window, cx);
                    }
                }),
            )));

        let body = if count == 0 {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .text_xl()
                        .text_color(self.colors.foreground)
                        .child("Your library is empty"),
                )
                .child(div().text_sm().text_color(self.colors.muted).child(
                    "Drop comic archives (CBZ, CBR) or folders onto this window, or use Add files (Ctrl+O).",
                ))
                .into_any_element()
        } else if books.is_empty() {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .text_xl()
                        .text_color(self.colors.foreground)
                        .child("No matching books"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(self.colors.muted)
                        .child("Try another title or source path."),
                )
                .into_any_element()
        } else if self.library.settings().group_series {
            let groups = directory_groups(all_books);
            let visible_groups: Vec<_> = if self.search_query.trim().is_empty() {
                groups
                    .iter()
                    .filter_map(|group| {
                        (!group.books.is_empty()).then_some((group, group.books.clone()))
                    })
                    .collect()
            } else {
                let visible_ids: HashSet<&str> = books.iter().map(|b| b.id.as_str()).collect();
                groups
                    .iter()
                    .filter_map(|group| {
                        let members: Vec<_> = group
                            .books
                            .iter()
                            .copied()
                            .filter(|book| visible_ids.contains(book.id.as_str()))
                            .collect();
                        (!members.is_empty()).then_some((group, members))
                    })
                    .collect()
            };
            if let Some(open_key) = self.open_group.as_deref()
                && let Some(group) = groups.iter().find(|group| group.key == open_key)
            {
                let members: Vec<_> = if self.search_query.trim().is_empty() {
                    group.books.clone()
                } else {
                    let visible_ids: HashSet<&str> = books.iter().map(|b| b.id.as_str()).collect();
                    group
                        .books
                        .iter()
                        .copied()
                        .filter(|book| visible_ids.contains(book.id.as_str()))
                        .collect()
                };
                let mut content = div().flex().flex_col().gap_4().child(
                    div().flex().items_center().gap_3().child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(self.colors.foreground)
                            .child(format!("{} · {}", group.name, members.len())),
                    ),
                );
                let grid_or_empty = if members.is_empty() {
                    div()
                        .py_12()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(
                            div()
                                .text_lg()
                                .text_color(self.colors.foreground)
                                .child("No matching books in this group"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(self.colors.muted)
                                .child("Try another title or source path."),
                        )
                        .into_any_element()
                } else {
                    make_grid(
                        members
                            .iter()
                            .map(|book| self.book_card(book, cx))
                            .collect(),
                    )
                    .into_any_element()
                };
                content = content.child(grid_or_empty);
                div()
                    .id("group-grid")
                    .track_scroll(&self.group_scroll.handle)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                        this.group_scroll
                            .handle_scroll_wheel(event, this.reduce_motion);
                        this.wake();
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, _| {
                            this.catch_ui_inertia();
                        }),
                    )
                    .px_6()
                    .pb_6()
                    .child(content)
                    .into_any_element()
            } else {
                let grouped_ids: HashSet<_> = visible_groups
                    .iter()
                    .flat_map(|(_, members)| members.iter().map(|book| book.id.as_str()))
                    .collect();
                let loose_books: Vec<_> = books
                    .iter()
                    .copied()
                    .filter(|book| !grouped_ids.contains(book.id.as_str()))
                    .collect();
                let mut cards: Vec<AnyElement> = visible_groups
                    .iter()
                    .map(|(group, members)| self.group_card(group, members, cx))
                    .collect();
                cards.extend(loose_books.iter().map(|book| self.book_card(book, cx)));
                div()
                    .id("grid")
                    .track_scroll(&self.library_scroll.handle)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                        this.library_scroll
                            .handle_scroll_wheel(event, this.reduce_motion);
                        this.wake();
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, _| {
                            this.catch_ui_inertia();
                        }),
                    )
                    .px_6()
                    .pb_6()
                    .child(make_grid(cards))
                    .into_any_element()
            }
        } else {
            div()
                .id("grid")
                .track_scroll(&self.library_scroll.handle)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                    this.library_scroll
                        .handle_scroll_wheel(event, this.reduce_motion);
                    this.wake();
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, _| {
                        this.catch_ui_inertia();
                    }),
                )
                .px_6()
                .pb_6()
                .child(make_grid(
                    books.iter().map(|book| self.book_card(book, cx)).collect(),
                ))
                .into_any_element()
        };

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| {
                this.on_library_pinch(event.delta, event.phase, cx);
            }))
            .child(header)
            .child(body)
            .children(status.map(|text| {
                div()
                    .absolute()
                    .bottom_5()
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(pill(div(), text, self.colors))
            }))
            .into_any_element()
    }

    fn group_card(
        &self,
        group: &LibraryGroup<'_>,
        books: &[&Book],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let first = books[0];
        let group_key = group.key.clone();
        let thumb_size = self.library.settings().thumbnail_size;
        let card_w = thumb_size.card_width();
        let card_h = thumb_size.card_height();
        let border_px = self.border_px();
        let rounding_lg = self.radius_lg_px();
        let inner_radius = if border_px > 0.0 {
            (rounding_lg - border_px).max(0.0)
        } else {
            rounding_lg
        };

        div()
            .id(SharedString::from(format!("folder-{}", group.key)))
            .w(px(card_w))
            .flex_none()
            .flex()
            .flex_col()
            .rounded(px(rounding_lg))
            .when(border_px > 0.0, |this| {
                this.border(px(border_px)).border_color(self.colors.border)
            })
            .bg(self.widget_surface(self.colors.surface))
            .overflow_hidden()
            .cursor_pointer()
            .when(self.overlay.is_none(), |card| {
                card.hover(|style| {
                    let style = if border_px > 0.0 {
                        style.border_color(self.colors.accent)
                    } else {
                        style
                    };
                    style.shadow_md()
                })
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.enter_group(group_key.clone(), cx);
            }))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(card_h))
                    .flex_none()
                    .overflow_hidden()
                    .rounded_t(px(inner_radius))
                    .bg(self.widget_surface(self.colors.surface))
                    .child(
                        img(self.dirs.cover_for_size(&first.id, thumb_size))
                            .w_full()
                            .h_full()
                            .rounded_t(px(inner_radius))
                            .object_fit(ObjectFit::Cover),
                    )
                    .child(
                        div().absolute().top_2().right_2().child(
                            Tag::secondary()
                                .small()
                                .child(format!("{} volumes", books.len())),
                        ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .gap_2()
                    .p_3()
                    .child(
                        div()
                            .h(px(40.))
                            .line_clamp(2)
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(self.colors.foreground)
                            .child(group.name.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().h(px(4.)))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(self.colors.muted)
                                    .child(format!("{} volumes", books.len())),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn book_card(&self, book: &Book, cx: &mut Context<Self>) -> AnyElement {
        let open_id = book.id.clone();
        let menu_id = book.id.clone();
        let progress = book.progress();
        let status = if book.is_finished() {
            "Finished".to_string()
        } else if book.opened {
            format!(
                "Page {} of {} · {}%",
                book.current_page + 1,
                book.page_count,
                (progress * 100.0).round() as u32
            )
        } else {
            format!("{} pages", book.page_count)
        };

        let thumb_size = self.library.settings().thumbnail_size;
        let card_w = thumb_size.card_width();
        let card_h = thumb_size.card_height();
        let selected = self.selection.contains(&book.id);
        let border_px = self.border_px();
        let rounding_lg = self.radius_lg_px();
        let card_border = if selected {
            border_px.max(2.0)
        } else {
            border_px
        };
        let inner_radius = if card_border > 0.0 {
            (rounding_lg - card_border).max(0.0)
        } else {
            rounding_lg
        };

        let cover = div()
            .relative()
            .w_full()
            .h(px(card_h))
            .flex_none()
            .rounded_t(px(inner_radius))
            .overflow_hidden()
            .bg(self.widget_surface(self.colors.surface))
            .child(
                img(self.dirs.cover_for_size(&book.id, thumb_size))
                    .w_full()
                    .h_full()
                    .rounded_t(px(inner_radius))
                    .object_fit(ObjectFit::Cover),
            )
            .children(selected.then(|| {
                // A tint over the cover so a selected book reads as selected at a glance.
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .rounded_t(px(inner_radius))
                    .bg(with_alpha(self.colors.accent, 0.22))
            }));

        let progress_bar = Progress::new(SharedString::from(format!("progress-{}", book.id)))
            .value(progress * 100.0)
            .color(self.colors.accent)
            .xsmall();

        div()
            .id(SharedString::from(format!("book-{}", book.id)))
            .w(px(card_w))
            .flex_none()
            .flex()
            .flex_col()
            .rounded(px(rounding_lg))
            .when(card_border > 0.0, |this| {
                this.border(px(card_border)).border_color(if selected {
                    self.colors.accent
                } else {
                    self.colors.border
                })
            })
            .bg(self.widget_surface(self.colors.surface))
            .overflow_hidden()
            .cursor_pointer()
            .when(self.overlay.is_none(), |card| {
                card.hover(|style| {
                    let style = if card_border > 0.0 && !selected {
                        style.border_color(self.colors.accent)
                    } else {
                        style
                    };
                    style.shadow_md()
                })
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, _| {
                    this.catch_ui_inertia();
                }),
            )
            // `on_click` only ever fires for the left button, so right-clicking never opens a book.
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if this.overlay_recently_closed() {
                    return;
                }
                if this
                    .scroll_caught_at
                    .is_some_and(|t| t.elapsed() < SCROLL_CATCH_GUARD)
                {
                    return;
                }
                // Ctrl/Shift-click, or any click while books are selected, edits the selection;
                // only a plain click on an unselected library opens the book.
                let modifiers = event.modifiers();
                let control = modifiers.control || modifiers.platform;
                if this.select_click(&open_id, control, modifiers.shift, cx) {
                    return;
                }
                this.open_book(&open_id, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.open_context_menu(&menu_id, event.position, window, cx);
                    // The root window also listens for right-clicks to dismiss an
                    // already-open popup. Do not let this opening click bubble up
                    // and immediately close the popup we just created.
                    cx.stop_propagation();
                }),
            )
            .child(cover)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .gap_2()
                    .p_3()
                    .child(
                        div()
                            .h(px(40.))
                            .line_clamp(2)
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(self.colors.foreground)
                            .child(SharedString::from(book.title.clone())),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(progress_bar)
                            .child(div().text_xs().text_color(self.colors.muted).child(status)),
                    ),
            )
            .into_any_element()
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
            )
        });
        let hud = self.hud.clone().map(|text| {
            pill(
                div().absolute().top_4().left_0().right_0(),
                text,
                self.colors,
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
                )
                .into_any_element()
            })
        };

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
            .children(
                reader
                    .show_help
                    .then(|| help_overlay(self.colors, self.border_px(), self.radius_xl_px())),
            )
            .into_any_element()
    }

    fn sync_ui_scroll(&mut self, dt: f32) -> bool {
        let mut moving = false;
        if self.library_scroll.step(dt, self.reduce_motion) {
            moving = true;
        }
        if self.group_scroll.step(dt, self.reduce_motion) {
            moving = true;
        }
        if self.shortcuts_scroll.step(dt, self.reduce_motion) {
            moving = true;
        }
        moving
    }

    fn catch_ui_inertia(&mut self) -> bool {
        let mut caught = false;
        if self.library_scroll.has_active_inertia() {
            self.library_scroll.reset();
            caught = true;
        }
        if self.group_scroll.has_active_inertia() {
            self.group_scroll.reset();
            caught = true;
        }
        if self.shortcuts_scroll.has_active_inertia() {
            self.shortcuts_scroll.reset();
            caught = true;
        }
        if caught {
            self.scroll_caught_at = Some(Instant::now());
        }
        caught
    }

    fn render_command_box(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let input = self.cmdline_text()?;
        let suggestions = command_suggestions(input, &self.custom_themes);
        let border_px = self.border_px();
        let rounding_lg = self.radius_lg_px();
        let rounding_md = self.radius_px();
        let inner_radius = if border_px > 0.0 {
            (rounding_lg - border_px).max(0.0)
        } else {
            rounding_lg
        };

        let mut prompt = div()
            .id("command-box-panel")
            .w(px(560.))
            .max_w(px(700.))
            .flex()
            .flex_col()
            .rounded(px(rounding_lg))
            .overflow_hidden()
            .when(border_px > 0.0, |this| {
                this.border(px(border_px)).border_color(self.colors.accent)
            })
            .bg(self.colors.surface)
            .shadow_xl()
            .text_sm()
            .text_color(self.colors.foreground)
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            })
            .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            });

        // Top child: Command input row
        let mut input_row = div()
            .id("command-box-input")
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .rounded_t(px(inner_radius))
            .when(suggestions.is_empty(), |this| {
                this.rounded_b(px(inner_radius))
            })
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(self.colors.accent)
                    .text_base()
                    .child(":"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(self.colors.foreground)
                    .child(format!("{input}▏")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(self.colors.muted)
                    .child("↑↓ select · Tab complete · Enter run · Esc cancel"),
            );

        if !suggestions.is_empty() {
            input_row = input_row.border_b_1().border_color(self.colors.border);
        }
        prompt = prompt.child(input_row);

        // Bottom child: Completion suggestions (if any)
        if !suggestions.is_empty() {
            let mut rows = div()
                .id("command-box-suggestions")
                .flex()
                .flex_col()
                .p_1()
                .rounded_b(px(inner_radius))
                .max_h(px(320.))
                .overflow_y_scroll();

            let item_radius = rounding_md.min(inner_radius);
            for (index, (completion, label, description)) in suggestions.iter().enumerate() {
                let selected = index == self.command_selection;
                let completion = completion.clone();
                rows = rows.child(
                    div()
                        .id(SharedString::from(format!("command-suggestion-{index}")))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .px_3()
                        .py(px(6.))
                        .rounded(px(item_radius))
                        .cursor_pointer()
                        .when(border_px > 0.0 || selected, |this| {
                            this.border(px(border_px.max(1.0)))
                                .border_color(if selected {
                                    with_alpha(self.colors.accent, 0.70)
                                } else {
                                    with_alpha(self.colors.accent, 0.0)
                                })
                        })
                        .when(selected, |row| row.bg(with_alpha(self.colors.accent, 0.20)))
                        .hover(|row| row.bg(with_alpha(self.colors.accent, 0.15)))
                        .child(
                            div()
                                .font_weight(if selected {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(if selected {
                                    self.colors.accent
                                } else {
                                    self.colors.foreground
                                })
                                .child(label.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_right()
                                .text_xs()
                                .text_color(self.colors.muted)
                                .truncate()
                                .child(*description),
                        )
                        .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                            if this.command_selection != index {
                                this.command_selection = index;
                                if let Some(draft) = this.cmdline_text().map(str::to_owned) {
                                    this.sync_theme_preview(&draft, cx);
                                }
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            let is_complete = !completion.ends_with(' ');
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
                        })),
                );
            }
            prompt = prompt.child(rows);
        }

        let scrim = div()
            .id("command-box-scrim")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(with_alpha(overlay_shade(self.colors), 0.40))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| this.cancel_cmdline(cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, _, cx| this.cancel_cmdline(cx)),
            )
            .child(self.animate_popup("command-prompt-popup", prompt));

        Some(deferred(scrim).with_priority(3).into_any_element())
    }
}

/// Suggestions for the reader's vim `:` command box:
/// returns (completed command text, display label, short explanation).
fn command_suggestions(
    input: &str,
    custom_themes: &HashMap<String, Rc<ThemeConfig>>,
) -> Vec<(String, String, &'static str)> {
    const COMMANDS: &[(&str, &str)] = &[
        ("fit", "Choose page fit mode"),
        ("zoom", "Set zoom percentage or reset"),
        ("animate", "Toggle page animations"),
        ("continuous", "Toggle continuous scrolling"),
        ("webcomics", "Toggle gapless webcomics mode"),
        ("remember-zoom", "Toggle retaining zoom across books"),
        ("theme", "Change color theme"),
        ("goto", "Jump to a page number"),
        ("mark", "Set a letter mark"),
        ("settings", "Open Settings"),
        ("info", "Toggle page information"),
        ("help", "Show keyboard help"),
        ("fullscreen", "Toggle fullscreen"),
        ("q", "Return to the library"),
        ("qa", "Quit yomu"),
    ];
    const FITS: &[(&str, &str)] = &[
        ("page", "Fit the whole page"),
        ("width", "Fit the page width"),
        ("height", "Fit the page height"),
        ("original", "Show at original size"),
    ];
    const TOGGLES: &[(&str, &str)] = &[("on", "Enable"), ("off", "Disable")];
    const ZOOM: &[(&str, &str)] = &[
        ("reset", "Reset zoom"),
        ("150", "Set zoom to 150% (edit the number as needed)"),
    ];

    let trimmed = input.trim_start();
    let mut words = trimmed.split_whitespace();
    let command = words.next().unwrap_or_default();
    let has_separator = trimmed.ends_with(char::is_whitespace);
    if let Some((_, _)) = COMMANDS.iter().find(|(name, _)| *name == command)
        && (has_separator || input.contains(' '))
    {
        if command == "theme" {
            let partial = trimmed
                .strip_prefix(command)
                .unwrap_or("")
                .trim_start()
                .to_lowercase();
            let mut themes: Vec<(String, &'static str)> = Vec::new();
            for (name, config) in &*BUILT_IN_THEMES {
                let desc = match config.mode {
                    gpui_kit::component::ThemeMode::Light => "Light theme",
                    gpui_kit::component::ThemeMode::Dark => "Dark theme",
                };
                themes.push((name.clone(), desc));
            }
            for (name, config) in custom_themes {
                if !BUILT_IN_THEMES.contains_key(name) {
                    let desc = match config.mode {
                        gpui_kit::component::ThemeMode::Light => "Custom light theme",
                        gpui_kit::component::ThemeMode::Dark => "Custom dark theme",
                    };
                    themes.push((name.clone(), desc));
                }
            }
            themes.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
            return themes
                .into_iter()
                .filter(|(name, _)| {
                    partial.is_empty()
                        || name.to_lowercase().starts_with(&partial)
                        || name.to_lowercase().contains(&partial)
                })
                .map(|(name, description)| (format!("{command} {name}"), name, description))
                .collect();
        }

        let args = if command == "fit" {
            FITS
        } else if matches!(
            command,
            "animate" | "continuous" | "webcomics" | "remember-zoom"
        ) {
            TOGGLES
        } else if command == "zoom" {
            ZOOM
        } else {
            &[]
        };
        let partial = words.next().unwrap_or_default();
        return args
            .iter()
            .filter(|(value, _)| value.starts_with(partial))
            .map(|(value, description)| {
                (
                    format!("{command} {value}"),
                    (*value).to_string(),
                    *description,
                )
            })
            .collect();
    }

    let prefix = if has_separator { "" } else { command };
    COMMANDS
        .iter()
        .filter(|(name, _)| name.starts_with(prefix))
        .take(8)
        .map(|(name, description)| (format!("{name} "), (*name).to_string(), *description))
        .collect()
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
        let mut animating = self.sync_reader(window, cx, dt);
        if self.sync_ui_scroll(dt) {
            animating = true;
        }
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

        if self.search_input.is_none() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(self.search_query.clone())
                    .placeholder("Search title, path, or series…")
            });
            cx.subscribe(&input, |this, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.search_query = input.read(cx).value().to_string();
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.deactivate_search(cx);
                }
                _ => {}
            })
            .detach();
            self.search_input = Some(input);
        }

        let reading = self.reader.is_some();
        let content = match self.reader.as_ref() {
            Some(reader) => self.render_reader(reader, cx),
            None => self.render_library(window, cx),
        };

        let root_bg = self.colors.background;
        let overlay = self.render_overlay(cx);
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
                    this.catch_ui_inertia();
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

// ----------------------------------------------------------------------------------------
// Small helpers
// ----------------------------------------------------------------------------------------

const HELP: &[(&str, &[(&str, &str)])] = &[
    (
        "Navigation",
        &[
            (
                "j, k, h, l / Arrows",
                "Scroll view by step; turns page at boundary.",
            ),
            ("Space / Shift+Space", "Scroll one viewport down or up."),
            ("Ctrl+D / Ctrl+U", "Scroll half a viewport down or up."),
            ("J, K / PgDn, PgUp", "Navigate to next or previous page."),
            ("gg / G / [N]G", "Jump to first, last, or page N."),
            ("m[a] / '[a] / ''", "Set mark, jump to mark, or jump back."),
        ],
    ),
    (
        "Display & Zoom",
        &[
            (
                "a / s / zh / z1",
                "Fit to page, width, height, or original size.",
            ),
            ("Tab", "Cycle through available fit modes."),
            ("+ / - / 0 / zi / zo", "Zoom in, zoom out, or reset zoom."),
            (
                "Ctrl+Wheel / Pinch / Drag",
                "Zoom at cursor position, or drag to pan.",
            ),
            ("zt / zz / zb", "Align view to top, center, or bottom."),
            (
                "v / w / i / f",
                "Toggle continuous, webcomics, info, or fullscreen.",
            ),
        ],
    ),
    (
        "Commands (:)",
        &[
            (
                ":fit [mode]",
                "Set fit mode (page, width, height, original).",
            ),
            (":zoom [pct]", "Set zoom percentage, or reset to fit."),
            (":q / :qa", "Close current comic, or quit application."),
            (
                ":settings / Ctrl+,",
                "Open settings and preferences dialog.",
            ),
            (
                ":animate [on|off]",
                "Enable or disable smooth page animations.",
            ),
            (
                ":continuous [on|off]",
                "Toggle stacked continuous reading mode.",
            ),
            (
                ":webcomics [on|off]",
                "Toggle gapless webcomics strip mode.",
            ),
            (":theme [name]", "Change color theme or cycle themes."),
        ],
    ),
];

fn help_overlay(colors: UiColors, border_px: f32, rounding_xl: f32) -> AnyElement {
    let mut panel = div()
        .w(px(600.))
        .max_w_full()
        .flex()
        .flex_col()
        .gap_3()
        .p_6()
        .rounded(px(rounding_xl))
        .when(border_px > 0.0, |this| {
            this.border(px(border_px)).border_color(colors.border)
        })
        .bg(colors.surface)
        .shadow_lg()
        .text_sm();

    for (title, rows) in HELP {
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(*title),
                )
                .children(rows.iter().map(|(keys, what)| {
                    div()
                        .flex()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .gap_4()
                        .child(div().text_color(colors.foreground).child(*what))
                        .child(div().flex_none().text_color(colors.accent).child(*keys))
                })),
        );
    }
    panel = panel.child(
        div()
            .text_xs()
            .text_color(colors.muted)
            .child("Press ? or Esc to close"),
    );

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(with_alpha(
            overlay_shade(colors),
            (0.5 + colors.widget_opacity * 0.15).min(0.65),
        ))
        .child(panel)
        .into_any_element()
}

fn icon_button(
    id: &'static str,
    label: &'static str,
    icon: IconName,
    colors: UiColors,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let button = Button::new(id)
        .accessibility_label(label)
        .tooltip(label)
        .rounded(ButtonRounded::Medium)
        .small()
        .ghost()
        .outline()
        .border_color(if active { colors.accent } else { colors.border })
        .text_color(if active {
            colors.accent
        } else {
            colors.foreground
        })
        .bg(colors.surface)
        .cursor_pointer()
        // Render the bundled SVG directly so its theme color is explicit. The
        // Button icon convenience path inherits a color from Kit's component
        // theme, which can be transparent against custom/translucent surfaces.
        .child(Icon::new(icon).size_4().text_color(if active {
            colors.accent
        } else {
            colors.foreground
        }))
        .on_click(on_click);
    #[cfg(test)]
    {
        div()
            .debug_selector(|| id.into())
            .child(button)
            .into_any_element()
    }
    #[cfg(not(test))]
    {
        button.into_any_element()
    }
}

fn app_icon() -> Option<Arc<image::RgbaImage>> {
    let bytes = include_bytes!("../assets/logo.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let resized = image::imageops::resize(&img, 128, 128, image::imageops::FilterType::Lanczos3);
    Some(Arc::new(resized))
}

fn ensure_desktop_icon() {
    #[cfg(target_os = "linux")]
    if let Some(data_dir) = dirs::data_dir() {
        let mut updated = false;
        let logo_bytes = include_bytes!("../assets/logo.png");
        let pixmap_icon = data_dir.join("pixmaps/yomu.png");
        let pixmap_needs_update = std::fs::read(&pixmap_icon)
            .map(|existing| existing != logo_bytes)
            .unwrap_or(true);
        if pixmap_needs_update {
            if let Some(parent) = pixmap_icon.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::write(&pixmap_icon, logo_bytes).is_ok() {
                updated = true;
            }
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
            let icon_needs_update = std::fs::read(&path)
                .map(|existing| existing != *bytes)
                .unwrap_or(true);
            if icon_needs_update {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if std::fs::write(&path, bytes).is_ok() {
                    updated = true;
                }
            }
        }

        let desktop_file = data_dir.join("applications/yomu.desktop");
        let desktop_content = include_str!("../packaging/yomu.desktop");
        if !desktop_file.exists() {
            if let Some(parent) = desktop_file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&desktop_file, desktop_content);
            updated = true;
        } else if let Ok(existing) = std::fs::read_to_string(&desktop_file)
            && !existing.contains("Icon=")
        {
            let _ = std::fs::write(&desktop_file, desktop_content);
            updated = true;
        }

        if updated {
            let hicolor_dir = data_dir.join("icons/hicolor");
            let _ = std::process::Command::new("gtk-update-icon-cache")
                .arg("-f")
                .arg("-t")
                .arg(hicolor_dir)
                .output();
        }
    }
}

fn main() {
    ensure_desktop_icon();
    application()
        // GPUI Kit icons are SVG asset paths. Without registering this source,
        // custom-rendered toolbar and context-menu icons resolve to empty images.
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx: &mut App| {
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
                        ..Default::default()
                    }),
                    // Compositors match window rules (Niri, Hyprland, Sway…) on the app id.
                    app_id: Some("yomu".into()),
                    window_min_size: Some(size(px(480.), px(360.))),
                    icon: app_icon(),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(Yomu::new);
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
