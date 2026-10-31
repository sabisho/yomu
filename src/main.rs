//! yomu: a minimal, GPU-accelerated, Wayland-native manga reader.

#![forbid(unsafe_code)]

use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use gpui_kit::component::Theme as KitTheme;
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
    library::{self, BorderSize, CornerRounding, Settings, ThemeModePreference, ThumbnailSize},
    motion,
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

use ui::library_view::directory_groups;
use ui::overlays::{ConfirmAction, ConfirmDialog, Overlay};
use ui::reader::{Reader, WheelParams, viewport_of};
#[allow(unused_imports)]
use ui::theme::custom_theme_fingerprint;
use ui::theme::{
    BUILT_IN_THEMES, DEFAULT_THEME_NAME, UiColors, built_in_theme, ensure_custom_theme_file,
    load_custom_themes,
};

impl Yomu {
    pub(crate) fn toggle_vim_mode(&mut self, enabled: bool, cx: &mut Context<Self>) {
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

    pub(crate) fn widget_surface(&self, color: gpui_kit::Hsla) -> gpui_kit::Hsla {
        color
    }

    pub(crate) fn menu_surface(&self) -> gpui_kit::Hsla {
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
    pub(crate) fn dismiss(&mut self) {
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

    pub(crate) fn set_thumbnail_size(&mut self, size: ThumbnailSize, cx: &mut Context<Self>) {
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

    pub(crate) fn border_px(&self) -> f32 {
        self.library.settings().border_size.pixels()
    }

    pub(crate) fn radius_px(&self) -> f32 {
        self.library.settings().corner_rounding.radius_pixels()
    }

    pub(crate) fn radius_lg_px(&self) -> f32 {
        self.library.settings().corner_rounding.radius_lg_pixels()
    }

    pub(crate) fn radius_xl_px(&self) -> f32 {
        self.library.settings().corner_rounding.radius_xl_pixels()
    }

    #[allow(dead_code)]
    pub(crate) fn set_border_size(&mut self, size: BorderSize, cx: &mut Context<Self>) {
        if self.is_quitting || self.library.settings().border_size == size {
            return;
        }
        self.library.settings_mut().border_size = size;
        self.save_library(cx);
        cx.notify();
    }

    #[allow(dead_code)]
    pub(crate) fn set_corner_rounding(&mut self, rounding: CornerRounding, cx: &mut Context<Self>) {
        if self.is_quitting || self.library.settings().corner_rounding == rounding {
            return;
        }
        self.library.settings_mut().corner_rounding = rounding;
        Self::apply_corner_rounding(rounding, cx);
        self.save_library(cx);
        cx.notify();
    }

    pub(crate) fn request_clear_thumbnail_cache(&mut self, cx: &mut Context<Self>) {
        self.overlay = Some(Overlay::Confirm(ConfirmDialog {
            title: "Clear thumbnail cache?".into(),
            message: "All cached cover thumbnails will be deleted and regenerated on demand. Comics and reading progress remain intact.".into(),
            confirm_label: "Clear Cache",
            action: ConfirmAction::ClearThumbnailCache,
        }));
        cx.notify();
    }

    pub(crate) fn clear_thumbnail_cache(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn request_clear_all_data(&mut self, cx: &mut Context<Self>) {
        self.overlay = Some(Overlay::Confirm(ConfirmDialog {
            title: "Reset all application data?".into(),
            message: "This will permanently remove all books, reading progress, marks, settings, and cached thumbnails.".into(),
            confirm_label: "Reset Everything",
            action: ConfirmAction::ClearAllData,
        }));
        cx.notify();
    }

    pub(crate) fn clear_all_data(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn request_reset_settings(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn reset_settings(&mut self, cx: &mut Context<Self>) {
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

    pub(crate) fn set_theme_mode(&mut self, mode: ThemeModePreference, cx: &mut Context<Self>) {
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

    pub(crate) fn set_color_scheme(&mut self, scheme: &str, cx: &mut Context<Self>) {
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

    // ------------------------------------------------------------------------------------
    // Import
    // ------------------------------------------------------------------------------------

    pub(crate) fn import_dialog(&mut self, directories: bool, cx: &mut Context<Self>) {
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

    pub(crate) fn prune_open_group(&mut self) {
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

    pub(crate) fn selected_ids(&self) -> Vec<String> {
        self.selection.in_order(&self.current_view_order())
    }

    pub(crate) fn prune_selection(&mut self) {
        let existing = self.library.ids();
        self.selection.retain_existing(&existing);
    }

    /// A click on a book card with the given modifiers. Returns `false` if it is a plain click
    /// that should open the book instead.
    pub(crate) fn select_click(
        &mut self,
        id: &str,
        ctrl: bool,
        shift: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let order = self.current_view_order();
        let handled = self.selection.click(&order, id, ctrl, shift);
        if handled {
            cx.notify();
        }
        handled
    }

    pub(crate) fn select_all(&mut self, cx: &mut Context<Self>) {
        let order = self.current_view_order();
        self.selection.select_all(&order);
        cx.notify();
    }

    #[allow(dead_code)]
    pub(crate) fn enter_group(&mut self, key: String, cx: &mut Context<Self>) {
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
