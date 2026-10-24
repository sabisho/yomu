//! Main toolbar matching macOS Yomu design reference.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable, Size};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Entity, FontWeight, InteractiveElement, Pixels, Styled, Window,
    div, px,
};

use super::components::{open_shortcut_str, search_shortcut_str, with_alpha};
use super::theme::UiColors;

pub struct MainToolbarProps<'a> {
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub is_in_group: bool,
    pub sidebar_collapsed: bool,
    pub is_scrolled: bool,
    pub search_input: Option<&'a Entity<InputState>>,
    pub is_grid_mode: bool,
    pub is_group_series: bool,
    pub colors: UiColors,
    pub radius: Pixels,
}

pub fn render_main_toolbar(
    props: MainToolbarProps,
    on_toggle_sidebar: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_back: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_forward: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_toggle_grid: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_group_series: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_import: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_toggle_grid = std::rc::Rc::new(on_toggle_grid);
    let on_toggle_grid_false = on_toggle_grid.clone();
    let on_toggle_group_series = std::rc::Rc::new(on_toggle_group_series);
    let colors = props.colors;

    div()
        .h(px(52.))
        .w_full()
        .px_4()
        .flex()
        .items_center()
        .justify_between()
        .when(props.is_scrolled, |s| {
            s.border_b_1().border_color(colors.border)
        })
        .when(!props.is_scrolled, |s| s.border_b_0())
        // Left: Sidebar toggle + Back / Forward navigation
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(
                    div()
                        .id("toolbar-sidebar-toggle")
                        .size(px(28.))
                        .rounded(props.radius)
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|s| s.bg(with_alpha(colors.foreground, 0.08)))
                        .tooltip(|window, cx| Tooltip::new("Toggle sidebar").build(window, cx))
                        .on_click(on_toggle_sidebar)
                        .child(Icon::new(IconName::PanelLeft).size(px(16.)).text_color(
                            if props.sidebar_collapsed {
                                colors.accent
                            } else {
                                colors.foreground
                            },
                        )),
                )
                .child(
                    div()
                        .id("toolbar-nav-back")
                        .size(px(28.))
                        .rounded(props.radius)
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(props.is_in_group, |this| {
                            this.debug_selector(|| "group-back".into())
                        })
                        .when(props.can_go_back, |style| {
                            style
                                .cursor_pointer()
                                .hover(|s| s.bg(with_alpha(colors.foreground, 0.08)))
                                .on_click(on_back)
                        })
                        .tooltip(|window, cx| Tooltip::new("Back").build(window, cx))
                        .child(Icon::new(IconName::ChevronLeft).size(px(16.)).text_color(
                            if props.can_go_back {
                                colors.foreground
                            } else {
                                colors.muted
                            },
                        )),
                )
                .child(
                    div()
                        .id("toolbar-nav-forward")
                        .size(px(28.))
                        .rounded(props.radius)
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(props.can_go_forward, |style| {
                            style
                                .cursor_pointer()
                                .hover(|s| s.bg(with_alpha(colors.foreground, 0.08)))
                                .on_click(on_forward)
                        })
                        .tooltip(|window, cx| Tooltip::new("Forward").build(window, cx))
                        .child(Icon::new(IconName::ChevronRight).size(px(16.)).text_color(
                            if props.can_go_forward {
                                colors.foreground
                            } else {
                                colors.muted
                            },
                        )),
                ),
        )
        // Center: Search input (adaptive, flex-1 with min/max bounds)
        .child(
            div()
                .flex_1()
                .max_w(px(400.))
                .min_w(px(140.))
                .mx_3()
                .flex()
                .items_center()
                .relative()
                .child(if let Some(input) = props.search_input {
                    div()
                        .w_full()
                        .rounded(props.radius)
                        .border_1()
                        .border_color(colors.border)
                        .bg(with_alpha(colors.surface, 0.6))
                        .child(
                            Input::new(input)
                                .id("main-search-input")
                                .cleanable(true)
                                .bordered(false)
                                .focus_bordered(false)
                                .prefix(Icon::new(IconName::Search).size(px(14.)))
                                .suffix(
                                    div()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded(px(4.))
                                        .bg(with_alpha(colors.foreground, 0.08))
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(colors.muted)
                                        .child(search_shortcut_str()),
                                )
                                .w_full(),
                        )
                        .into_any_element()
                } else {
                    div().into_any_element()
                }),
        )
        // Right: View toggles & Import button
        .child(
            div()
                .flex()
                .items_center()
                .gap_2p5()
                // 1. Folder Grouping switch: Folders vs Individual Volumes (icon pair, no text)
                .child(
                    div()
                        .id("toolbar-grouping-segmented")
                        .h(px(28.))
                        .flex()
                        .items_center()
                        .bg(with_alpha(colors.surface, 0.6))
                        .border_1()
                        .border_color(colors.border)
                        .rounded(props.radius)
                        .p(px(2.))
                        .gap(px(2.))
                        .child(
                            div()
                                .id("toolbar-group-folders-toggle")
                                .h_full()
                                .w(px(28.))
                                .rounded(px((f32::from(props.radius) - 2.0).max(2.0)))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .bg(if props.is_group_series {
                                    colors.accent.into()
                                } else {
                                    with_alpha(colors.foreground, 0.0)
                                })
                                .when(!props.is_group_series, |this| {
                                    this.hover(|s| s.bg(with_alpha(colors.foreground, 0.08)))
                                })
                                .tooltip(|window, cx| {
                                    Tooltip::new("Group by series folders").build(window, cx)
                                })
                                .on_click({
                                    let on_toggle = on_toggle_group_series.clone();
                                    move |_, window, cx| on_toggle(true, window, cx)
                                })
                                .child(Icon::new(IconName::Folder).size(px(14.)).text_color(
                                    if props.is_group_series {
                                        colors.accent_foreground
                                    } else {
                                        colors.muted
                                    },
                                )),
                        )
                        .child(
                            div()
                                .id("toolbar-group-volumes-toggle")
                                .h_full()
                                .w(px(28.))
                                .rounded(px((f32::from(props.radius) - 2.0).max(2.0)))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .bg(if !props.is_group_series {
                                    colors.accent.into()
                                } else {
                                    with_alpha(colors.foreground, 0.0)
                                })
                                .when(props.is_group_series, |this| {
                                    this.hover(|s| s.bg(with_alpha(colors.foreground, 0.08)))
                                })
                                .tooltip(|window, cx| {
                                    Tooltip::new("Show individual volumes").build(window, cx)
                                })
                                .on_click({
                                    let on_toggle = on_toggle_group_series.clone();
                                    move |_, window, cx| on_toggle(false, window, cx)
                                })
                                .child(Icon::new(IconName::BookOpen).size(px(14.)).text_color(
                                    if !props.is_group_series {
                                        colors.accent_foreground
                                    } else {
                                        colors.muted
                                    },
                                )),
                        ),
                )
                // 2. View Mode switch: Grid vs List (icon pair, no text)
                .child(
                    div()
                        .id("toolbar-viewmode-segmented")
                        .h(px(28.))
                        .flex()
                        .items_center()
                        .bg(with_alpha(colors.surface, 0.6))
                        .border_1()
                        .border_color(colors.border)
                        .rounded(props.radius)
                        .p(px(2.))
                        .gap(px(2.))
                        .child(
                            div()
                                .id("view-mode-grid-toggle")
                                .h_full()
                                .w(px(28.))
                                .rounded(px((f32::from(props.radius) - 2.0).max(2.0)))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .bg(if props.is_grid_mode {
                                    colors.accent.into()
                                } else {
                                    with_alpha(colors.foreground, 0.0)
                                })
                                .when(!props.is_grid_mode, |this| {
                                    this.hover(|s| s.bg(with_alpha(colors.foreground, 0.08)))
                                })
                                .tooltip(|window, cx| Tooltip::new("Grid view").build(window, cx))
                                .on_click({
                                    let on_toggle_grid = on_toggle_grid.clone();
                                    move |_, window, cx| on_toggle_grid(true, window, cx)
                                })
                                .child(Icon::new(IconName::LayoutGrid).size(px(14.)).text_color(
                                    if props.is_grid_mode {
                                        colors.accent_foreground
                                    } else {
                                        colors.muted
                                    },
                                )),
                        )
                        .child(
                            div()
                                .id("view-mode-list-toggle")
                                .h_full()
                                .w(px(28.))
                                .rounded(px((f32::from(props.radius) - 2.0).max(2.0)))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .bg(if !props.is_grid_mode {
                                    colors.accent.into()
                                } else {
                                    with_alpha(colors.foreground, 0.0)
                                })
                                .when(props.is_grid_mode, |this| {
                                    this.hover(|s| s.bg(with_alpha(colors.foreground, 0.08)))
                                })
                                .tooltip(|window, cx| Tooltip::new("List view").build(window, cx))
                                .on_click({
                                    let on_toggle_grid_false = on_toggle_grid_false.clone();
                                    move |_, window, cx| on_toggle_grid_false(false, window, cx)
                                })
                                .child(Icon::new(IconName::List).size(px(14.)).text_color(
                                    if !props.is_grid_mode {
                                        colors.accent_foreground
                                    } else {
                                        colors.muted
                                    },
                                )),
                        ),
                )
                // 3. Import Button
                .child(
                    div().debug_selector(|| "add-files".into()).child(
                        Button::new("add-files")
                            .primary()
                            .icon(IconName::Plus)
                            .label("Import")
                            .with_size(Size::Size(px(28.)))
                            .rounded(ButtonRounded::Size(props.radius))
                            .bg(colors.accent)
                            .text_color(colors.accent_foreground)
                            .tooltip(format!("Import manga ({})", open_shortcut_str()))
                            .on_click(on_import),
                    ),
                ),
        )
        .into_any_element()
}
