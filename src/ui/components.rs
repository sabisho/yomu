//! Reusable UI elements, dialog buttons, and layout helpers.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Disableable, Selectable, Sizable, Size};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ElementId, Pixels, Rgba, SharedString, Styled, Window, div,
    px,
};

use super::theme::UiColors;
use crate::library::ThumbnailSize;

// ============================================================================
// 1. Control Sizing Constants
// ============================================================================

/// Standard height for toolbar actions, search fields, and top-level icon buttons (28px).
pub const CONTROL_HEIGHT_TOOLBAR: Pixels = px(28.);

/// Compact height for segmented control options and inline toggle buttons (24px).
pub const CONTROL_HEIGHT_SEGMENT: Pixels = px(24.);

/// Standard height for action buttons, dialog buttons, and sidebar nav rows (32px).
#[allow(dead_code)]
pub const CONTROL_HEIGHT_ACTION: Pixels = px(32.);

/// Prominent height for primary call-to-action buttons (hero continue reading, large imports) (36px).
#[allow(dead_code)]
pub const CONTROL_HEIGHT_PRIMARY: Pixels = px(36.);

/// Fixed height for application top toolbar and floating reader header (52px).
pub const TOOLBAR_HEIGHT: Pixels = px(52.);

// ============================================================================
// 2. Icon Sizing Scale
// ============================================================================

/// Compact icon size for inline indicators, search prefixes, and segmented buttons (14px).
pub const ICON_SIZE_COMPACT: Pixels = px(14.);

/// Standard icon size for toolbar buttons, navigation rows, and dropdown items (16px).
#[allow(dead_code)]
pub const ICON_SIZE_STANDARD: Pixels = px(16.);

/// Action icon size for branding, headers, and dialogs (18px).
#[allow(dead_code)]
pub const ICON_SIZE_ACTION: Pixels = px(18.);

/// Large icon size for lateral controls, floating chevrons, and hero actions (20px).
#[allow(dead_code)]
pub const ICON_SIZE_LARGE: Pixels = px(20.);

/// Display icon size for empty-state illustrations and placeholders (48px).
#[allow(dead_code)]
pub const ICON_SIZE_EMPTY: Pixels = px(48.);

// ============================================================================
// 3. Desktop Semantic Spacing Scale
// ============================================================================

/// 2px: Micro gaps, optical adjustments, segmented container insets.
pub const SPACE_2XS: Pixels = px(2.);

/// 4px: Tight gaps, icon-to-label spacing, title-to-subtitle spacing.
#[allow(dead_code)]
pub const SPACE_XS: Pixels = px(4.);

/// 8px: Standard gap between related buttons and controls.
#[allow(dead_code)]
pub const SPACE_SM: Pixels = px(8.);

/// 12px: Medium gap between content items, compact container padding.
#[allow(dead_code)]
pub const SPACE_MD: Pixels = px(12.);

/// 16px: Standard container padding, panel insets, section gaps.
#[allow(dead_code)]
pub const SPACE_LG: Pixels = px(16.);

/// 24px: Major section separation, view-level breathing room.
#[allow(dead_code)]
pub const SPACE_XL: Pixels = px(24.);

/// 32px: Large surface breathing room, empty-state padding.
#[allow(dead_code)]
pub const SPACE_2XL: Pixels = px(32.);

// ============================================================================
// 4. Geometry & Radius Helpers
// ============================================================================

/// Fixed corner radius tier for micro elements (3px).
#[allow(dead_code)]
pub const RADIUS_XS: Pixels = px(3.);
/// Fixed corner radius tier for compact elements (4px).
#[allow(dead_code)]
pub const RADIUS_SM: Pixels = px(4.);
/// Fixed corner radius tier for standard controls (6px).
#[allow(dead_code)]
pub const RADIUS_MD: Pixels = px(6.);
/// Fixed corner radius tier for large containers and cards (8px).
#[allow(dead_code)]
pub const RADIUS_LG: Pixels = px(8.);
/// Fixed corner radius tier for modal panels and dialogs (10px).
#[allow(dead_code)]
pub const RADIUS_XL: Pixels = px(10.);

/// Calculates a proportional button radius based on the configured application radius,
/// clamping to at most 25% of the button height (e.g. 7px for a 28px button, 6px for 24px)
/// to preserve rounded-square icon button geometry and prevent degenerating into a circular pill.
#[inline]
pub fn button_radius(app_radius: Pixels, control_height: Pixels) -> Pixels {
    let max_r = f32::from(control_height) * 0.25;
    px(f32::from(app_radius).clamp(0.0, max_r))
}

/// Calculates an inner nested corner radius ensuring concentric geometry:
/// `inner ≈ max(2.0, outer - gap)`
#[inline]
pub fn inner_radius(outer_radius: Pixels, gap: Pixels) -> Pixels {
    px((f32::from(outer_radius) - f32::from(gap)).max(2.0))
}

// ============================================================================
// 5. Button Builders
// ============================================================================

/// Creates a standard toolbar ghost button with consistent sizing (28x28px),
/// bounded corner radius, and accessibility tooltip.
pub fn toolbar_icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tooltip: impl Into<SharedString>,
    app_radius: Pixels,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    let radius = button_radius(app_radius, CONTROL_HEIGHT_TOOLBAR);
    Button::new(id)
        .ghost()
        .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
        .rounded(ButtonRounded::Size(radius))
        .tooltip(tooltip)
        .icon(icon)
        .on_click(on_click)
}

/// Creates a compact segmented toggle button (24x24px) for view mode or grouping switches.
pub fn segmented_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tooltip: impl Into<SharedString>,
    selected: bool,
    inner_radius: Pixels,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .ghost()
        .with_size(Size::Size(CONTROL_HEIGHT_SEGMENT))
        .rounded(ButtonRounded::Size(inner_radius))
        .tooltip(tooltip)
        .selected(selected)
        .icon(icon)
        .on_click(on_click)
}

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
    if colors.is_dark() {
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
    if colors.is_light() {
        colors.foreground
    } else {
        colors.background
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::hsla;

    #[test]
    fn test_sizing_scale_hierarchy() {
        assert!(CONTROL_HEIGHT_SEGMENT < CONTROL_HEIGHT_TOOLBAR);
        assert!(CONTROL_HEIGHT_TOOLBAR < CONTROL_HEIGHT_ACTION);
        assert!(CONTROL_HEIGHT_ACTION < CONTROL_HEIGHT_PRIMARY);

        assert!(SPACE_2XS < SPACE_XS);
        assert!(SPACE_XS < SPACE_SM);
        assert!(SPACE_SM < SPACE_MD);
        assert!(SPACE_MD < SPACE_LG);
        assert!(SPACE_LG < SPACE_XL);
        assert!(SPACE_XL < SPACE_2XL);

        assert!(ICON_SIZE_COMPACT < ICON_SIZE_STANDARD);
        assert!(ICON_SIZE_STANDARD < ICON_SIZE_ACTION);
        assert!(ICON_SIZE_ACTION < ICON_SIZE_LARGE);
        assert!(ICON_SIZE_LARGE < ICON_SIZE_EMPTY);

        assert!(RADIUS_XS < RADIUS_SM);
        assert!(RADIUS_SM < RADIUS_MD);
        assert!(RADIUS_MD < RADIUS_LG);
        assert!(RADIUS_LG < RADIUS_XL);
    }

    #[test]
    fn test_button_radius_clamping() {
        // For a 28px button, max radius is 28 * 0.25 = 7.0px
        assert_eq!(button_radius(px(0.), CONTROL_HEIGHT_TOOLBAR), px(0.));
        assert_eq!(button_radius(px(4.), CONTROL_HEIGHT_TOOLBAR), px(4.));
        assert_eq!(button_radius(px(7.), CONTROL_HEIGHT_TOOLBAR), px(7.));
        assert_eq!(button_radius(px(12.), CONTROL_HEIGHT_TOOLBAR), px(7.));
        assert_eq!(button_radius(px(999.), CONTROL_HEIGHT_TOOLBAR), px(7.));

        // For a 24px button, max radius is 24 * 0.25 = 6.0px
        assert_eq!(button_radius(px(0.), CONTROL_HEIGHT_SEGMENT), px(0.));
        assert_eq!(button_radius(px(4.), CONTROL_HEIGHT_SEGMENT), px(4.));
        assert_eq!(button_radius(px(6.), CONTROL_HEIGHT_SEGMENT), px(6.));
        assert_eq!(button_radius(px(12.), CONTROL_HEIGHT_SEGMENT), px(6.));
    }

    #[test]
    fn test_inner_radius_concentric() {
        // inner = max(2.0, outer - gap)
        assert_eq!(inner_radius(px(8.), px(2.)), px(6.));
        assert_eq!(inner_radius(px(4.), px(2.)), px(2.));
        assert_eq!(inner_radius(px(2.), px(2.)), px(2.));
        assert_eq!(inner_radius(px(1.), px(2.)), px(2.));
    }

    #[test]
    fn test_ui_colors_light_dark_modes() {
        let mut dark_colors = UiColors::from_tokens(gpui_kit::component::ColorTokens::dark());
        dark_colors.background = hsla(0.0, 0.0, 0.15, 1.0);
        assert!(dark_colors.is_dark());
        assert!(!dark_colors.is_light());

        let mut light_colors = UiColors::from_tokens(gpui_kit::component::ColorTokens::light());
        light_colors.background = hsla(0.0, 0.0, 0.95, 1.0);
        assert!(light_colors.is_light());
        assert!(!light_colors.is_dark());
    }

    #[test]
    fn test_button_builders_construct() {
        let btn = toolbar_icon_button(
            "test-toolbar-btn",
            IconName::PanelLeft,
            "Toggle",
            px(8.),
            |_, _, _| {},
        );
        let _elem: AnyElement = btn.into_any_element();

        let seg = segmented_button(
            "test-seg-btn",
            IconName::Folder,
            "Folder",
            true,
            px(4.),
            |_, _, _| {},
        );
        let _elem2: AnyElement = seg.into_any_element();
    }
}
