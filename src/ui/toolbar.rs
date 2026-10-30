//! Main toolbar matching macOS Yomu design reference.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonRounded, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Selectable, Sizable, Size};
use gpui_kit::gpui::Anchor;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Entity, FontWeight, InteractiveElement, Keystroke, Pixels, Styled,
    Window, div, px,
};

use super::components::{open_shortcut_str, with_alpha};
use super::theme::UiColors;

pub struct MainToolbarProps<'a> {
    pub can_go_back: bool,
    pub sidebar_collapsed: bool,
    pub is_scrolled: bool,
    pub show_search: bool,
    pub search_input: Option<&'a Entity<InputState>>,
    pub show_view_switches: bool,
    pub is_grid_mode: bool,
    pub show_grouping: bool,
    pub is_group_series: bool,
    pub show_import: bool,
    pub is_client_decorated: bool,
    pub selection_count: usize,
    pub on_select_all: Option<std::rc::Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    pub on_remove_selected: Option<std::rc::Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    pub on_cancel_selection: Option<std::rc::Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    pub colors: UiColors,
    pub radius: Pixels,
}

pub fn render_main_toolbar(
    props: MainToolbarProps,
    on_toggle_sidebar: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_back: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_toggle_grid: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_group_series: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_import_files: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_import_folder: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_close_window: Option<std::rc::Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
) -> AnyElement {
    let on_toggle_grid = std::rc::Rc::new(on_toggle_grid);
    let on_toggle_grid_false = on_toggle_grid.clone();
    let on_toggle_group_series = std::rc::Rc::new(on_toggle_group_series);
    let on_import_files = std::rc::Rc::new(on_import_files);
    let on_import_folder = std::rc::Rc::new(on_import_folder);
    let colors = props.colors;
    let button_radius = px(f32::from(props.radius).min(7.0));

    div()
        .h(px(52.))
        .flex_shrink_0()
        .w_full()
        .px_4()
        .when(props.sidebar_collapsed && cfg!(target_os = "macos"), |s| {
            s.pl(px(80.))
        })
        .flex()
        .items_center()
        .justify_between()
        .border_b_1()
        .border_color(if props.is_scrolled {
            colors.border
        } else {
            colors.border.opacity(0.0)
        })
        // Left: Sidebar toggle + Back navigation (visible only when inside a group/series)
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(
                    Button::new("toolbar-sidebar-toggle")
                        .ghost()
                        .with_size(Size::Size(px(28.)))
                        .rounded(ButtonRounded::Size(button_radius))
                        .tooltip("Toggle sidebar")
                        .icon(IconName::PanelLeft)
                        .on_click(on_toggle_sidebar),
                )
                .when(props.can_go_back, |this| {
                    this.child(
                        div().debug_selector(|| "group-back".into()).child(
                            Button::new("group-back")
                                .ghost()
                                .with_size(Size::Size(px(28.)))
                                .rounded(ButtonRounded::Size(button_radius))
                                .tooltip("Back to library")
                                .icon(IconName::ChevronLeft)
                                .on_click(on_back),
                        ),
                    )
                }),
        )
        // Center: Search input (adaptive, flexible, centered in available space)
        .child(
            div()
                .flex_1()
                .min_w(px(140.))
                .mx_3()
                .flex()
                .items_center()
                .justify_center()
                .when(props.show_search && props.selection_count == 0, |this| {
                    if let Some(input) = props.search_input {
                        this.child(
                            div()
                                .w_full()
                                .max_w(px(400.))
                                .min_w(px(140.))
                                .flex_shrink_0()
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
                                        .suffix(Kbd::new(
                                            Keystroke::parse(if cfg!(target_os = "macos") {
                                                "cmd-k"
                                            } else {
                                                "ctrl-k"
                                            })
                                            .unwrap(),
                                        ))
                                        .w_full(),
                                ),
                        )
                    } else {
                        this
                    }
                }),
        )
        // Right: Context-aware actions
        .child(
            div()
                .flex()
                .items_center()
                .gap_2p5()
                .when(props.selection_count > 0, |this| {
                    let on_select_all = props.on_select_all.clone();
                    let on_remove_selected = props.on_remove_selected.clone();
                    let on_cancel_selection = props.on_cancel_selection.clone();
                    this.child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colors.foreground)
                            .child(format!("{} selected", props.selection_count)),
                    )
                    .child(
                        div().debug_selector(|| "select-all".into()).child(
                            Button::new("select-all")
                                .ghost()
                                .small()
                                .label("Select all")
                                .on_click(move |e, window, cx| {
                                    if let Some(f) = on_select_all.as_ref() {
                                        f(e, window, cx);
                                    }
                                }),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "select-remove".into()).child(
                            Button::new("select-remove")
                                .ghost()
                                .small()
                                .label("Remove")
                                .on_click(move |e, window, cx| {
                                    if let Some(f) = on_remove_selected.as_ref() {
                                        f(e, window, cx);
                                    }
                                }),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "select-cancel".into()).child(
                            Button::new("select-cancel")
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(move |e, window, cx| {
                                    if let Some(f) = on_cancel_selection.as_ref() {
                                        f(e, window, cx);
                                    }
                                }),
                        ),
                    )
                })
                .when(props.selection_count == 0, |this| {
                    this
                        // 1. Folder Grouping switch: Folders vs Individual Volumes
                        .when(props.show_grouping, |this| {
                            let inner_radius = px((f32::from(props.radius) - 2.0).max(2.0));
                            this.child(
                                ButtonGroup::new("toolbar-grouping-segmented")
                                    .h(px(28.))
                                    .items_center()
                                    .bg(with_alpha(colors.surface, 0.6))
                                    .border_1()
                                    .border_color(colors.border)
                                    .rounded(props.radius)
                                    .p(px(2.))
                                    .child(
                                        Button::new("toolbar-group-folders-toggle")
                                            .ghost()
                                            .with_size(Size::Size(px(24.)))
                                            .rounded(ButtonRounded::Size(inner_radius))
                                            .tooltip("Group by series folders")
                                            .selected(props.is_group_series)
                                            .icon(IconName::Folder)
                                            .on_click({
                                                let on_toggle = on_toggle_group_series.clone();
                                                move |_, window, cx| on_toggle(true, window, cx)
                                            }),
                                    )
                                    .child(
                                        Button::new("toolbar-group-volumes-toggle")
                                            .ghost()
                                            .with_size(Size::Size(px(24.)))
                                            .rounded(ButtonRounded::Size(inner_radius))
                                            .tooltip("Show individual volumes")
                                            .selected(!props.is_group_series)
                                            .icon(IconName::BookOpen)
                                            .on_click({
                                                let on_toggle = on_toggle_group_series.clone();
                                                move |_, window, cx| on_toggle(false, window, cx)
                                            }),
                                    ),
                            )
                        })
                        // 2. View Mode switch: Grid vs List
                        .when(props.show_view_switches, |this| {
                            let inner_radius = px((f32::from(props.radius) - 2.0).max(2.0));
                            this.child(
                                ButtonGroup::new("toolbar-viewmode-segmented")
                                    .h(px(28.))
                                    .items_center()
                                    .bg(with_alpha(colors.surface, 0.6))
                                    .border_1()
                                    .border_color(colors.border)
                                    .rounded(props.radius)
                                    .p(px(2.))
                                    .child(
                                        Button::new("view-mode-grid-toggle")
                                            .ghost()
                                            .with_size(Size::Size(px(24.)))
                                            .rounded(ButtonRounded::Size(inner_radius))
                                            .tooltip("Grid view")
                                            .selected(props.is_grid_mode)
                                            .icon(IconName::LayoutGrid)
                                            .on_click({
                                                let on_toggle_grid = on_toggle_grid.clone();
                                                move |_, window, cx| {
                                                    on_toggle_grid(true, window, cx)
                                                }
                                            }),
                                    )
                                    .child(
                                        Button::new("view-mode-list-toggle")
                                            .ghost()
                                            .with_size(Size::Size(px(24.)))
                                            .rounded(ButtonRounded::Size(inner_radius))
                                            .tooltip("List view")
                                            .selected(!props.is_grid_mode)
                                            .icon(IconName::List)
                                            .on_click({
                                                let on_toggle_grid_false =
                                                    on_toggle_grid_false.clone();
                                                move |_, window, cx| {
                                                    on_toggle_grid_false(false, window, cx)
                                                }
                                            }),
                                    ),
                            )
                        })
                        // 3. Import Button: unified rounded-square button matching 28px toolbar controls
                        .when(props.show_import, |this| {
                            let on_import_files = on_import_files.clone();
                            let on_import_folder = on_import_folder.clone();
                            // Clamping radius to 7.0px (25% of 28px) preserves the rounded-square
                            // geometry and prevents degenerating into a circular disc under large/pill radii.
                            let button_radius = px(f32::from(props.radius).min(7.0));
                            this.child(
                                div().debug_selector(|| "add-files".into()).child(
                                    Button::new("add-files")
                                        .icon(IconName::Plus)
                                        .ghost()
                                        .with_size(Size::Size(px(28.)))
                                        .rounded(ButtonRounded::Size(button_radius))
                                        .border_1()
                                        .border_color(colors.border)
                                        .bg(with_alpha(colors.surface, 0.6))
                                        .tooltip(format!("Import manga ({})", open_shortcut_str()))
                                        .dropdown_menu_with_anchor(
                                            Anchor::TopRight,
                                            move |mut menu, _, _| {
                                                let on_import_files = on_import_files.clone();
                                                let on_import_folder = on_import_folder.clone();
                                                menu = menu
                                                    .min_w(px(180.))
                                                    .item(
                                                        PopupMenuItem::new("Import Files…")
                                                            .icon(IconName::FilePlus)
                                                            .on_click(move |e, window, cx| {
                                                                on_import_files(e, window, cx)
                                                            }),
                                                    )
                                                    .item(
                                                        PopupMenuItem::new("Import Folder…")
                                                            .icon(IconName::FolderPlus)
                                                            .on_click(move |e, window, cx| {
                                                                on_import_folder(e, window, cx)
                                                            }),
                                                    );
                                                menu
                                            },
                                        ),
                                ),
                            )
                        })
                })
                .when(props.is_client_decorated, |this| {
                    this.child(
                        Button::new("toolbar-csd-close")
                            .ghost()
                            .with_size(Size::Size(px(24.)))
                            .rounded(ButtonRounded::Large)
                            .tooltip("Close")
                            .icon(IconName::Close)
                            .on_click(move |e, window, cx| {
                                if let Some(on_close) = on_close_window.as_ref() {
                                    on_close(e, window, cx);
                                }
                            }),
                    )
                }),
        )
        .into_any_element()
}
