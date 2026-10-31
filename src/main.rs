//! yomu: a minimal, GPU-accelerated, Wayland-native manga reader.

#![forbid(unsafe_code)]

use std::{
    cell::RefCell,
    collections::HashSet,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Theme as KitTheme};
#[allow(unused_imports)]
use gpui_kit::popup::{PopupAnchor, PopupConstraintAdjustment, PopupGravity, PopupOptions};
#[allow(unused_imports)]
use gpui_kit::{
    Anchor, Animation, AnimationExt, AnyElement, AnyWindowHandle, App, Bounds, ClickEvent, Context,
    Decorations, ElementId, Entity, ExternalPaths, FocusHandle, Focusable, FontWeight, KeyBinding,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit,
    PathPromptOptions, PinchEvent, Pixels, Point, RenderImage, Rgba, ScrollDelta, ScrollEvent,
    ScrollHandle, SharedString, Size, Styled, Subscription, TitlebarOptions, TouchPhase, Window,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowKind, WindowOptions,
    actions, anchored, application, canvas, deferred, div, ease_out_quint, img, point, prelude::*,
    px, relative, size,
};
#[cfg(test)]
use yomu::layout;
use yomu::{
    layout::{Arrive, FitMode},
    library::{
        self, Book, BorderSize, CornerRounding, Settings, ThemeModePreference, ThumbnailSize,
    },
    menu::{self, BookAction},
    motion,
    util::format_size,
};

pub use yomu::util::now_secs;

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

mod app;
mod ui;

pub use app::{
    CLICK_GUARD, DRAG_THRESHOLD, EDGE_EPS, HUD_SECS, ImportEvent, LINE_STEP, NavDestination,
    ReaderScrollHandle, SCROLL_CATCH_GUARD, Side, WHEEL_LINE, Yomu,
};

use ui::library_view::{
    BookCardProps, LibraryGroup, directory_groups, render_book_card, render_book_list_row,
    render_folder_card, render_folder_list_row,
};
use ui::overlays::{
    BookMenu, ConfirmAction, ConfirmDialog, LibraryMenu, Overlay, PropertiesDialog, SeriesDialog,
};
use ui::reader::{Reader, WheelParams, viewport_of};
#[allow(unused_imports)]
use ui::theme::custom_theme_fingerprint;
use ui::theme::{
    BUILT_IN_THEMES, DEFAULT_THEME_NAME, UiColors, built_in_theme, ensure_custom_theme_file,
    load_custom_themes,
};

impl Yomu {
    fn toggle_vim_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        self.library.settings_mut().vim_mode = enabled;
        self.save_library(cx);
        cx.notify();
    }

    pub fn toggle_group_series(&mut self, group: bool, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        self.library.settings_mut().group_series = group;
        if !group {
            self.open_group = None;
            self.group_scroll.set_offset(Point::default());
        }
        self.save_library(cx);
        cx.notify();
    }

    fn widget_surface(&self, color: gpui_kit::Hsla) -> gpui_kit::Hsla {
        color
    }

    fn menu_surface(&self) -> gpui_kit::Hsla {
        self.colors.surface
    }

    /// An input just started an animation: start the frame clock now so the first frame already
    /// accounts for the input-to-frame latency.
    pub(crate) fn wake(&mut self) {
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

    pub(crate) fn show_toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
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
    pub(crate) fn show_hud(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
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
    pub(crate) fn close_overlay(&mut self, cx: &mut Context<Self>) {
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
        if self.is_quitting {
            return;
        }
        let Some(Overlay::Series(dialog)) = self.overlay.take() else {
            return;
        };
        self.overlay_closed_at = Some(Instant::now());
        let count = dialog.book_ids.len();
        let value = dialog.input.read(cx).value().to_string();
        let series = (!value.trim().is_empty()).then_some(value);

        self.library.set_series(&dialog.book_ids, series.clone());
        self.prune_open_group();
        self.save_library(cx);
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
    pub(crate) fn run_book_action(
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
                    self.save_library(cx);
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
                    self.save_library(cx);
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
        if self.is_quitting {
            return;
        }
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
                self.save_library(cx);
                self.open_book(id, window, cx);
            }
            BookAction::MarkRead => {
                if let Some(book) = self.library.get_mut(id) {
                    book.mark_read();
                }
                self.save_library(cx);
                self.show_toast(format!("Marked “{title}” as read"), cx);
            }
            BookAction::MarkUnread | BookAction::ResetProgress => {
                if let Some(book) = self.library.get_mut(id) {
                    book.reset_progress();
                }
                self.save_library(cx);
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

    pub(crate) fn confirm_accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Overlay::Confirm(dialog)) = self.overlay.take() else {
            return;
        };
        if self.is_quitting && !matches!(dialog.action, ConfirmAction::ForceQuit) {
            return;
        }
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
                self.save_library(cx);
                self.show_toast("Collection deleted", cx);
            }
            ConfirmAction::ForceQuit => {
                self.overlay = None;
                cx.quit();
            }
        }
    }

    // ------------------------------------------------------------------------------------
    // Settings
    // ------------------------------------------------------------------------------------

    pub(crate) fn open_settings(&mut self, cx: &mut Context<Self>) {
        match self.overlay {
            None => self.overlay = Some(Overlay::Settings),
            Some(Overlay::Settings) => self.close_overlay(cx),
            Some(_) => {}
        }
        cx.notify();
    }

    pub(crate) fn open_settings_window(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(cx);
    }

    fn set_thumbnail_size(&mut self, size: ThumbnailSize, cx: &mut Context<Self>) {
        if self.is_quitting || self.library.settings().thumbnail_size == size {
            return;
        }
        self.library.settings_mut().thumbnail_size = size;
        self.save_library(cx);
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
        if self.is_quitting || self.library.settings().border_size == size {
            return;
        }
        self.library.settings_mut().border_size = size;
        self.save_library(cx);
        cx.notify();
    }

    #[allow(dead_code)]
    fn set_corner_rounding(&mut self, rounding: CornerRounding, cx: &mut Context<Self>) {
        if self.is_quitting || self.library.settings().corner_rounding == rounding {
            return;
        }
        self.library.settings_mut().corner_rounding = rounding;
        Self::apply_corner_rounding(rounding, cx);
        self.save_library(cx);
        cx.notify();
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
        if self.is_quitting {
            return;
        }
        let size = self.library.settings().thumbnail_size;
        let library = self.library.clone();
        let dirs = Arc::clone(&self.dirs);
        cx.background_executor()
            .spawn(async move {
                let _ = dirs.clear_covers();
                library.ensure_covers_for_size(&dirs, size);
            })
            .detach();
        self.overlay = Some(Overlay::Settings);
        self.show_toast("Thumbnail cache cleared", cx);
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
        if self.is_quitting {
            return;
        }
        self.reader = None;
        self.selection.clear();
        self.open_group = None;
        self.group_scroll.set_offset(Point::default());
        self.library.clear_all_data();
        let dirs = Arc::clone(&self.dirs);
        cx.background_executor()
            .spawn(async move {
                let _ = dirs.clear_covers();
            })
            .detach();
        self.save_library(cx);
        self.overlay = Some(Overlay::Settings);
        self.show_toast("Application data reset to defaults", cx);
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
        if self.is_quitting {
            return;
        }
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
        self.save_library(cx);
        self.overlay = Some(Overlay::Settings);
        cx.notify();
    }

    pub(crate) fn apply_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
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
        self.save_library(cx);
        cx.notify();
    }

    pub(crate) fn preview_theme(&mut self, name: &str, cx: &mut Context<Self>) {
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

    pub(crate) fn cycle_theme(&mut self, delta: isize, cx: &mut Context<Self>) {
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
        if self.is_quitting {
            return;
        }
        self.library.settings_mut().theme_mode = mode;
        let scheme = self.library.settings().color_scheme.clone();
        let resolved =
            ui::theme::resolve_theme_name(&scheme, mode, self.system_dark, &self.custom_themes);
        self.apply_theme(&resolved, cx);
    }

    #[allow(dead_code)]
    fn cycle_color_scheme(&mut self, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
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
        if self.is_quitting {
            return;
        }
        self.library.settings_mut().color_scheme = scheme.to_string();
        let mode = self.library.settings().theme_mode;
        let resolved =
            ui::theme::resolve_theme_name(scheme, mode, self.system_dark, &self.custom_themes);
        self.apply_theme(&resolved, cx);
    }

    #[allow(dead_code)]
    pub(crate) fn reload_custom_themes(&mut self, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
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
        if self.is_quitting {
            return;
        }
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

    pub(crate) fn run_library_menu_action(
        &mut self,
        selected: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting {
            return;
        }
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
                self.save_library(cx);
                cx.notify();
            }
            4 => {
                self.request_quit(cx);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------------------------
    // Import
    // ------------------------------------------------------------------------------------

    fn import_dialog(&mut self, directories: bool, cx: &mut Context<Self>) {
        if self.is_quitting || self.importing || self.overlay.is_some() {
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
                this.update(cx, |this, cx| {
                    if !this.is_quitting {
                        this.import_paths(paths, cx);
                    }
                })
                .ok();
            }
            Ok(Err(err)) => {
                let message = format!("{err:#}");
                this.update(cx, |this, cx| {
                    if !this.is_quitting {
                        this.show_toast(message, cx);
                    }
                })
                .ok();
            }
            _ => {}
        })
        .detach();
    }

    fn import_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.is_quitting || self.importing || paths.is_empty() {
            return;
        }
        self.importing = true;
        self.imported_count = 0;
        self.importing_title = None;
        let cancel_token = Arc::new(AtomicBool::new(false));
        self.import_cancel = Some(Arc::clone(&cancel_token));
        cx.notify();

        let dirs = self.dirs.clone();
        let existing = self.library.ids();
        let (sender, receiver) = mpsc::channel();
        let cancel_worker = Arc::clone(&cancel_token);
        cx.background_executor()
            .spawn(async move {
                let report = library::import_paths_with_progress_cancellable(
                    &paths,
                    &dirs,
                    &existing,
                    Some(&cancel_worker),
                    |book| {
                        let _ = sender.send(ImportEvent::Book(book.clone()));
                    },
                );
                let _ = sender.send(ImportEvent::Finished(report));
            })
            .detach();

        let cancel_ui = Arc::clone(&cancel_token);
        cx.spawn(async move |this, cx| {
            loop {
                let should_stop = this
                    .read_with(cx, |this, _| {
                        this.is_quitting || cancel_ui.load(Ordering::Relaxed)
                    })
                    .unwrap_or(true);
                if should_stop {
                    let _ = this.update(cx, |this, _| {
                        let was_active = this
                            .import_cancel
                            .as_ref()
                            .is_some_and(|active| Arc::ptr_eq(active, &cancel_ui));
                        if was_active {
                            this.import_cancel = None;
                            this.importing = false;
                            this.importing_title = None;
                        }
                    });
                    break;
                }
                match receiver.try_recv() {
                    Ok(ImportEvent::Book(book)) => {
                        let _ = this
                            .update(cx, |this, cx| {
                                if this.is_quitting || cancel_ui.load(Ordering::Relaxed) {
                                    return;
                                }
                                let title = book.title.clone();
                                if this.library.add(book) {
                                    this.save_library(cx);
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
                                let was_active = this
                                    .import_cancel
                                    .as_ref()
                                    .is_some_and(|active| Arc::ptr_eq(active, &cancel_ui));
                                if was_active {
                                    this.import_cancel = None;
                                    this.importing = false;
                                    this.importing_title = None;
                                }
                                if !this.is_quitting && !cancel_ui.load(Ordering::Relaxed) {
                                    this.show_toast(report.summary(), cx);
                                }
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
                                let was_active = this
                                    .import_cancel
                                    .as_ref()
                                    .is_some_and(|active| Arc::ptr_eq(active, &cancel_ui));
                                if was_active {
                                    this.import_cancel = None;
                                    this.importing = false;
                                    this.importing_title = None;
                                }
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
        if self.is_quitting {
            return;
        }
        let Some(title) = self.library.get(id).map(|b| b.title.clone()) else {
            return;
        };
        self.library.remove(id, &self.dirs);
        self.prune_selection();
        self.prune_open_group();
        self.save_library(cx);
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
    pub(crate) fn current_view_order(&self) -> Vec<String> {
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
        if self.is_quitting {
            return;
        }
        self.dismiss();
        for id in ids {
            self.library.remove(id, &self.dirs);
            self.selection.remove(id);
        }
        self.prune_open_group();
        self.save_library(cx);
        let message = if ids.len() == 1 {
            "Removed 1 book from the library".to_string()
        } else {
            format!("Removed {} books from the library", ids.len())
        };
        self.show_toast(message, cx);
    }

    /// Remove every selected book from the library, asking first only if progress or marks
    /// would be lost. The comic files are never touched.
    pub(crate) fn request_remove_selected(&mut self, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
        self.request_remove_books(&ids, cx);
    }

    fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
        self.remove_books(&ids, cx);
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

    pub(crate) fn exit_group(&mut self, cx: &mut Context<Self>) {
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

    #[allow(dead_code)]
    fn on_library_pinch(&mut self, delta: f32, phase: TouchPhase, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
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
                self.save_library(cx);
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
                self.save_library(cx);
                self.show_toast(format!("Thumbnails: {}", new_size.label()), cx);
                cx.notify();
            }
        }
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
                    this.save_library(cx);
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
                        this.save_library(cx);
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
                        this.save_library(cx);
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
            {
                let yomu = yomu.clone();
                Some(Rc::new(move |_, _, cx| {
                    yomu.update(cx, |this, cx| this.request_quit(cx));
                }))
            },
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
                            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
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
                            }))
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
            .size_full()
            .overflow_y_scrollbar()
            .track_scroll(scroll_handle)
            .scroll_bounce(true)
            .scroll_wheel(false)
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

    fn render_reader(&self, reader: &Reader, cx: &mut Context<Self>) -> AnyElement {
        let vp = reader.viewport;

        let reader_scroll_handle = if reader.continuous {
            let doc_height = reader.continuous_total_height(&vp);
            if doc_height > vp.height {
                let max_offset = (doc_height - vp.height).max(0.0);
                Some(ReaderScrollHandle {
                    bounds: Bounds {
                        origin: point(px(0.), px(0.)),
                        size: size(px(vp.width), px(vp.height)),
                    },
                    offset_y: reader.off.1,
                    content_height: doc_height,
                    sync: self.reader_scroll_sync.clone(),
                    max_offset,
                })
            } else {
                None
            }
        } else {
            None
        };

        let yomu = cx.entity();
        let book_title = self
            .library
            .get(&reader.book_id)
            .map(|b| b.title.clone())
            .unwrap_or_default();

        let props = ui::reader::ReaderProps {
            reader,
            colors: self.colors,
            radius: px(self.radius_px()),
            border_px: self.border_px(),
            radius_xl_px: self.radius_xl_px(),
            book_title: &book_title,
            hud: self.hud.clone(),
            scroll_handle: reader_scroll_handle.as_ref(),
        };

        let yomu_back = yomu.clone();
        let on_back = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            yomu_back.update(cx, |this, cx| this.close_book(window, cx));
        };

        let yomu_fit = yomu.clone();
        let on_set_fit = move |mode: FitMode, _: &mut Window, cx: &mut App| {
            yomu_fit.update(cx, |this, cx| this.set_fit(mode, cx));
        };

        let yomu_prev = yomu.clone();
        let on_turn_prev = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            yomu_prev.update(cx, |this, cx| {
                this.turn_by(false, 1, Arrive::Start, cx);
            });
        };

        let yomu_next = yomu.clone();
        let on_turn_next = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            yomu_next.update(cx, |this, cx| {
                this.turn_by(true, 1, Arrive::Start, cx);
            });
        };

        let yomu_down = yomu.clone();
        let on_mouse_down = move |event: &MouseDownEvent, _: &mut Window, cx: &mut App| {
            yomu_down.update(cx, |this, _| {
                this.drag_start((f32::from(event.position.x), f32::from(event.position.y)));
            });
        };

        let yomu_move = yomu.clone();
        let on_mouse_move = move |event: &MouseMoveEvent, window: &mut Window, cx: &mut App| {
            let at = (f32::from(event.position.x), f32::from(event.position.y));
            let pressed = event.pressed_button == Some(MouseButton::Left);
            let vp = viewport_of(window);
            yomu_move.update(cx, |this, cx| {
                this.cursor_pos = Some(at);
                this.drag_move(at, pressed, vp, cx);
            });
        };

        let yomu_up = yomu.clone();
        let on_mouse_up = move |event: &MouseUpEvent, window: &mut Window, cx: &mut App| {
            let x = f32::from(event.position.x);
            let vp = viewport_of(window);
            yomu_up.update(cx, |this, cx| {
                this.drag_end(x, vp, cx);
            });
        };

        let yomu_wheel = yomu.clone();
        let on_scroll = move |event: &ScrollEvent, window: &mut Window, cx: &mut App| {
            let delta = event.delta.pixel_delta(px(WHEEL_LINE));
            let discrete = matches!(event.delta, ScrollDelta::Lines(_));
            let control = event.modifiers.control || event.modifiers.platform;
            let params = WheelParams::new(
                (f32::from(delta.x), f32::from(delta.y)),
                control,
                (f32::from(event.position.x), f32::from(event.position.y)),
                discrete,
                event.touch_phase,
                viewport_of(window),
            );
            yomu_wheel.update(cx, |this, cx| {
                this.on_wheel(params, cx);
            });
        };

        let yomu_pinch = yomu;
        let on_pinch = move |event: &PinchEvent, window: &mut Window, cx: &mut App| {
            let at = (f32::from(event.position.x), f32::from(event.position.y));
            let delta = event.delta;
            let phase = event.phase;
            let vp = viewport_of(window);
            yomu_pinch.update(cx, |this, cx| {
                this.on_pinch(at, delta, phase, vp, cx);
            });
        };

        ui::reader::render_reader(
            props,
            on_back,
            on_set_fit,
            on_turn_prev,
            on_turn_next,
            on_mouse_down,
            on_mouse_move,
            on_mouse_up,
            on_scroll,
            on_pinch,
        )
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
        if let Some(target_y) = self.reader_scroll_sync.take()
            && let Some(reader) = self.reader.as_mut()
        {
            reader.scroll_target.1 = target_y;
            reader.off.1 = target_y;
        }

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

        let shutdown_barrier = if self.is_quitting {
            Some(
                div()
                    .id("shutdown-barrier")
                    .occlude()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .cursor(gpui_kit::CursorStyle::Arrow)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .on_scroll(|_, _, cx| cx.stop_propagation()),
            )
        } else {
            None
        };

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
                    if !this.is_quitting && this.search_active {
                        this.deactivate_search(cx);
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &ToggleFullscreen, window, _| {
                if !this.is_quitting {
                    window.toggle_fullscreen();
                }
            }))
            .on_action(cx.listener(|this, _: &ImportFiles, _, cx| {
                if !this.is_quitting {
                    this.import_dialog(false, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ImportFolder, _, cx| {
                if !this.is_quitting {
                    this.import_dialog(true, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                if !this.is_quitting {
                    this.open_settings_window(window, cx);
                }
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                if !this.is_quitting && this.overlay.is_none() {
                    this.import_paths(paths.paths().to_vec(), cx);
                }
            }))
            .child(content)
            .children(overlay)
            .children(cmd_box)
            .children(shutdown_barrier)
    }
}

pub use app::desktop::{
    app_icon, desktop_entry_content, install_desktop_integration, install_desktop_integration_to,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args
        .iter()
        .any(|arg| arg == "--install-desktop" || arg == "--install-icons")
    {
        let force = args.iter().any(|arg| arg == "--force");
        if let Err(err) = app::desktop::install_desktop_integration(force) {
            eprintln!("yomu: desktop installation failed: {err:#}");
            std::process::exit(1);
        }
        return;
    }

    let vim_cli_override = args.iter().any(|arg| arg == "--vim" || arg == "--vim-mode");

    // Eagerly initialize the memory budget on application startup before window creation
    // and before entering the GPUI render loop.
    let _ = yomu::pagecache::system_memory_budget();

    application()
        // GPUI Kit icons are SVG asset paths. Without registering this source,
        // custom-rendered toolbar and context-menu icons resolve to empty images.
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            KitTheme::set_scrollbar_mode(gpui_kit::component::scroll::ScrollbarMode::Scrolling, cx);
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

            let yomu_view: Rc<RefCell<Option<Entity<Yomu>>>> = Rc::new(RefCell::new(None));
            let yomu_for_quit = yomu_view.clone();
            cx.on_action(move |_: &Quit, cx: &mut App| {
                if let Some(view) = yomu_for_quit.borrow().as_ref() {
                    view.update(cx, |this, cx| this.request_quit(cx));
                } else {
                    cx.quit();
                }
            });
            cx.on_window_closed(|cx, _window_id| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let bounds = Bounds::centered(None, size(px(1180.), px(780.)), cx);
            let yomu_view_setter = yomu_view.clone();
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
                    *yomu_view_setter.borrow_mut() = Some(view.clone());
                    view.update(cx, |this, _| this.vim_cli_override = vim_cli_override);
                    let focus = view.read(cx).focus_handle.clone();
                    window.focus(&focus, cx);
                    window.on_window_should_close(cx, {
                        let view = view.clone();
                        move |_window, cx| {
                            let is_quitting = view.read(cx).is_quitting;
                            if !is_quitting {
                                view.update(cx, |this, cx| this.request_quit(cx));
                            }
                            false
                        }
                    });
                    view
                },
            )
            .expect("failed to open the window");

            cx.activate(true);
        });
}

#[cfg(test)]
mod tests;
