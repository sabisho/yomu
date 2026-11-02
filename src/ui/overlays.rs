//! Overlay definitions and rendering: menus, dialogs, settings tabs, and modal states.

use gpui_kit::component::input::InputState;
use gpui_kit::component::label::Label;
use gpui_kit::component::tag::Tag;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Entity, FontWeight, Hsla, IntoElement, ParentElement, SharedString, Styled, div, px,
};
use yomu::menu::BookAction;

use crate::ui::components::{overlay_shade, with_alpha};
use crate::ui::theme::UiColors;

#[allow(dead_code)]
#[derive(Clone)]
pub enum ConfirmAction {
    Book(BookAction, String),
    RemoveBooks(Vec<String>),
    RemoveSelected,
    ResetSettings,
    ClearThumbnailCache,
    ClearAllData,
    ForceQuit,
}

#[derive(Clone)]
pub struct ConfirmDialog {
    pub title: SharedString,
    pub message: SharedString,
    pub confirm_label: &'static str,
    pub action: ConfirmAction,
}

#[allow(dead_code)]
pub struct PropertiesDialog {
    pub title: SharedString,
    pub rows: Vec<(String, String)>,
}

#[allow(dead_code)]
pub struct SeriesDialog {
    pub book_ids: Vec<String>,
    pub input: Entity<InputState>,
}

/// At most one dialog is open at a time; it sits on top of whatever view is showing.
#[allow(dead_code)]
pub enum Overlay {
    Confirm(ConfirmDialog),
    Properties(PropertiesDialog),
    Series(SeriesDialog),
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum SettingsTab {
    #[default]
    General,
    Appearance,
    Reader,
    Shortcuts,
    Advanced,
    About,
}

impl SettingsTab {
    #[allow(dead_code)]
    pub fn from_index(idx: usize) -> Self {
        match idx {
            0 => Self::General,
            1 => Self::Appearance,
            2 => Self::Reader,
            3 => Self::Shortcuts,
            4 => Self::Advanced,
            5 => Self::About,
            _ => Self::General,
        }
    }

    #[allow(dead_code)]
    pub fn to_index(self) -> usize {
        match self {
            Self::General => 0,
            Self::Appearance => 1,
            Self::Reader => 2,
            Self::Shortcuts => 3,
            Self::Advanced => 4,
            Self::About => 5,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Reader => "Reader",
            Self::Shortcuts => "Shortcuts",
            Self::Advanced => "Advanced",
            Self::About => "About",
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct OverlayStyle {
    pub colors: UiColors,
    pub border_px: f32,
    pub radius_px: f32,
    pub rounding_lg: f32,
    pub rounding_xl: f32,
    pub surface_bg: Hsla,
}

pub const HELP: &[(&str, &[(&str, &str)])] = &[
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
                "d / s / v / w",
                "Toggle double page, spread override, continuous, webcomics.",
            ),
            ("i / f", "Toggle info overlay or fullscreen."),
        ],
    ),
];

pub fn help_overlay(colors: UiColors, border_px: f32, rounding_xl: f32) -> AnyElement {
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
                    Label::new(*title)
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.muted),
                )
                .children(rows.iter().map(|(keys, what)| {
                    div()
                        .flex()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .gap_4()
                        .child(Label::new(*what).text_sm().text_color(colors.foreground))
                        .child(Tag::secondary().child(*keys))
                })),
        );
    }
    panel = panel.child(
        Label::new("Press ? or Esc to close")
            .text_xs()
            .text_color(colors.muted),
    );

    let duration = std::time::Duration::from_millis(120);
    let animated_panel = gpui_kit::base::animation::EffectTransition::new(duration)
        .ease(gpui_kit::base::animation::ease_out_cubic)
        .fade(0.0, 1.0)
        .slide_y(px(-6.), px(0.))
        .apply(panel, "help-overlay-panel");

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
        .child(animated_panel)
        .into_any_element()
}
