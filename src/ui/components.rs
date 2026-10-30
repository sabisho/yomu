//! Reusable UI elements, dialog buttons, and layout helpers.

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ObjectFit, Pixels, RenderImage, Rgba, SharedString, Styled,
    StyledImage, Window, div, img, px,
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

    let card_w = thumb_size.card_width();
    let cover_h = thumb_size.card_height();

    let min_space = if content_w < 560.0 { 12.0 } else { 14.0 };

    // Maximum number of cards that can fit with at least min_space margin on edges and between cards
    let cols = if content_w <= card_w + min_space * 2.0 {
        1
    } else {
        (((content_w - min_space) / (card_w + min_space)).floor() as usize).max(1)
    };

    let (pad_x, gap) = if cols <= 1 {
        let pad = ((content_w - card_w) / 2.0).max(min_space);
        (pad, min_space)
    } else {
        let rem = (content_w - (cols as f32 * card_w)).max(0.0);
        let space = rem / (cols + 1) as f32;
        let gap = space.floor().max(min_space);
        let pad_x =
            ((content_w - (cols as f32 * card_w + (cols - 1) as f32 * gap)) / 2.0).max(min_space);
        (pad_x, gap)
    };

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
#[allow(dead_code)]
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

/// Subtle count badge used in the sidebar and navigation items.
pub fn stat_badge(
    count: usize,
    active: bool,
    _colors: UiColors,
    _radius: Pixels,
) -> impl IntoElement {
    if active {
        Tag::primary().small().child(count.to_string())
    } else {
        Tag::secondary().small().child(count.to_string())
    }
}

/// Badge for manga genre tags.
pub fn genre_tag(
    label: impl Into<SharedString>,
    _colors: UiColors,
    _radius: Pixels,
) -> impl IntoElement {
    Tag::secondary().small().child(label.into())
}
