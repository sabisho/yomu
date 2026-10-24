//! Reusable UI elements, dialog buttons, and layout helpers.

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, InteractiveElement, ObjectFit, Pixels, RenderImage, Rgba,
    SharedString, Styled, StyledImage, Window, div, img, px,
};

use super::theme::UiColors;
use crate::library::ThumbnailSize;

#[derive(Clone, Copy, Debug)]
pub struct ResponsiveGrid {
    #[allow(dead_code)]
    pub cols: usize,
    pub card_w: f32,
    pub cover_h: f32,
    pub gap: f32,
    pub pad_x: f32,
}

pub fn compute_responsive_grid(
    viewport_width: f32,
    sidebar_visible: bool,
    thumb_size: ThumbnailSize,
) -> ResponsiveGrid {
    let sidebar_w = if sidebar_visible { 240.0 } else { 0.0 };
    let content_w = (viewport_width - sidebar_w).max(200.0);
    let is_narrow = content_w < 560.0;
    let pad_x = if is_narrow { 16.0 } else { 24.0 };
    let gap = if is_narrow { 16.0 } else { 24.0 };
    let available_w = (content_w - pad_x * 2.0).max(120.0);

    let base_w = thumb_size.card_width();
    let max_cols = ((available_w + gap) / (base_w * 0.74 + gap))
        .floor()
        .max(1.0) as usize;
    let min_cols = ((available_w + gap) / (base_w * 1.36 + gap))
        .floor()
        .max(1.0) as usize;
    let target_cols = ((available_w + gap) / (base_w + gap)).round().max(1.0) as usize;
    let cols = target_cols.clamp(min_cols, max_cols);

    let card_w = if cols == 1 {
        available_w.min(base_w * 1.25)
    } else {
        ((available_w - (cols - 1) as f32 * gap) / cols as f32).max(120.0)
    };
    let cover_h = (card_w * 1.5).round();

    ResponsiveGrid {
        cols,
        card_w,
        cover_h,
        gap,
        pad_x,
    }
}

pub fn with_alpha(color: gpui_kit::Hsla, alpha: f32) -> Rgba {
    let mut color: Rgba = color.into();
    color.a = alpha;
    color
}

/// Returns the search shortcut for the current OS.
/// On macOS: "⌘K", on Linux / Windows: "Ctrl+K"
pub fn search_shortcut_str() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘K"
    } else {
        "Ctrl+K"
    }
}

/// Returns the settings shortcut for the current OS.
/// On macOS: "⌘,", on Linux / Windows: "Ctrl+,"
pub fn settings_shortcut_str() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘,"
    } else {
        "Ctrl+,"
    }
}

/// Returns the open / import files shortcut for the current OS.
/// On macOS: "⌘O", on Linux / Windows: "Ctrl+O"
pub fn open_shortcut_str() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘O"
    } else {
        "Ctrl+O"
    }
}

/// Returns the zoom shortcut description for the current OS.
/// On macOS: "+ / - or Cmd+Wheel", on Linux / Windows: "+ / - or Ctrl+Wheel"
pub fn zoom_shortcut_str() -> &'static str {
    if cfg!(target_os = "macos") {
        "+ / - or Cmd+Wheel"
    } else {
        "+ / - or Ctrl+Wheel"
    }
}

/// Computes macOS-style sidebar background with refined subtle contrast relative to main content pane.
/// Preserves previous contrast reference for easy revert if requested:
///   Previous Dark:  with_alpha(colors.surface, 0.45)
///   Previous Light: with_alpha(colors.surface, 0.85)
pub fn macos_sidebar_bg(colors: UiColors) -> Rgba {
    if colors.background.l < 0.5 {
        // Refined Dark mode: subtle deeper tone for the macOS navigation pane
        let mut hsla = colors.background;
        hsla.l = (hsla.l * 0.78).clamp(0.04, 0.35);
        hsla.into()
    } else {
        // Refined Light mode: soft macOS subtle warm/cool gray tint
        let mut hsla = colors.background;
        hsla.l = (hsla.l * 0.95).clamp(0.85, 0.97);
        hsla.into()
    }
}

pub fn overlay_shade(colors: UiColors) -> gpui_kit::Hsla {
    if colors.background.l > 0.5 {
        colors.foreground
    } else {
        colors.background
    }
}

/// One page image at an exact position and size (see `render_reader`).
pub fn page_layer(
    image: &Arc<RenderImage>,
    left: f32,
    top: f32,
    display: (f32, f32),
) -> AnyElement {
    img(image.clone())
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(display.0))
        .h(px(display.1))
        .object_fit(ObjectFit::Fill)
        .into_any_element()
}

pub fn centered(text: impl Into<SharedString>, colors: UiColors) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_color(colors.muted)
        .child(text.into())
        .into_any_element()
}

pub fn pill(row: Div, text: impl Into<SharedString>, colors: UiColors, radius: Pixels) -> Div {
    row.flex().justify_center().child(
        div()
            .px_3()
            .py_1()
            .rounded(radius)
            .bg(colors.surface)
            .border_1()
            .border_color(colors.border)
            .shadow_md()
            .text_sm()
            .text_color(colors.foreground)
            .child(text.into()),
    )
}

#[derive(Clone, Copy)]
pub enum ButtonKind {
    Normal,
    Primary,
    Danger,
}

/// A button for dialogs: same shape as the header buttons, on the dialog's raised surface.
pub fn panel_button(
    id: &'static str,
    label: impl Into<SharedString>,
    kind: ButtonKind,
    colors: UiColors,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let button = Button::new(id)
        .label(label)
        .rounded(ButtonRounded::Medium)
        .small()
        .disabled(!enabled)
        .when(enabled, |btn| btn.cursor_pointer())
        .when(!enabled, |btn| btn.cursor_not_allowed())
        .on_click(on_click);
    let button = match kind {
        ButtonKind::Normal => button
            .secondary()
            .outline()
            .bg(colors.surface)
            .border_color(colors.border),
        ButtonKind::Primary => button.primary().bg(colors.accent),
        ButtonKind::Danger => button.danger().outline(),
    };
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

/// Three macOS traffic light dots (close, minimize, zoom) rendered at the top-left of the window.
pub fn mac_window_controls(
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_min: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_zoom: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .id("traffic-close")
                .size(px(12.))
                .rounded_full()
                .bg(gpui_kit::rgb(0xFF5F56))
                .border_1()
                .border_color(gpui_kit::rgb(0xE0443E))
                .cursor_pointer()
                .tooltip(|window, cx| Tooltip::new("Close window").build(window, cx))
                .on_click(on_close),
        )
        .child(
            div()
                .id("traffic-min")
                .size(px(12.))
                .rounded_full()
                .bg(gpui_kit::rgb(0xFFBD2E))
                .border_1()
                .border_color(gpui_kit::rgb(0xDEA123))
                .cursor_pointer()
                .tooltip(|window, cx| Tooltip::new("Minimize window").build(window, cx))
                .on_click(on_min),
        )
        .child(
            div()
                .id("traffic-zoom")
                .size(px(12.))
                .rounded_full()
                .bg(gpui_kit::rgb(0x27C93F))
                .border_1()
                .border_color(gpui_kit::rgb(0x1AAB29))
                .cursor_pointer()
                .tooltip(|window, cx| Tooltip::new("Toggle fullscreen (F11)").build(window, cx))
                .on_click(on_zoom),
        )
}

/// Subtle count badge used in the sidebar and navigation items.
pub fn stat_badge(count: usize, active: bool, colors: UiColors, radius: Pixels) -> Div {
    let bg_color = if active {
        with_alpha(colors.accent_foreground, 0.22)
    } else {
        with_alpha(colors.foreground, 0.08)
    };
    let text_color = if active {
        colors.accent_foreground
    } else {
        colors.muted
    };
    div()
        .px_2()
        .py_0p5()
        .rounded(radius)
        .bg(bg_color)
        .text_xs()
        .font_weight(gpui_kit::FontWeight::MEDIUM)
        .text_color(text_color)
        .child(count.to_string())
}

/// Badge for manga genre tags.
pub fn genre_tag(label: impl Into<SharedString>, colors: UiColors, radius: Pixels) -> Div {
    div()
        .px_2p5()
        .py_1()
        .rounded(radius)
        .bg(with_alpha(colors.foreground, 0.08))
        .text_xs()
        .font_weight(gpui_kit::FontWeight::NORMAL)
        .text_color(colors.foreground)
        .child(label.into())
}

/// Star rating pill.
pub fn star_rating(score: i32, popularity: Option<i32>, colors: UiColors) -> Div {
    let score_str = format!("★ {:.2}", score as f32 / 10.0);
    let mut row = div().flex().items_center().gap_2().child(
        div()
            .text_sm()
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .text_color(gpui_kit::rgb(0xF5A623))
            .child(score_str),
    );
    if let Some(pop) = popularity {
        row = row.child(
            div()
                .text_xs()
                .text_color(colors.muted)
                .child(format!("#{pop} Popularity")),
        );
    }
    row
}

/// Section header for sidebars and content groupings.
pub fn section_header(title: impl Into<SharedString>, colors: UiColors) -> Div {
    div()
        .px_3()
        .pt_4()
        .pb_1p5()
        .text_xs()
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(title.into())
}
