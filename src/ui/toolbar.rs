//! Main toolbar matching macOS Yomu design reference.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonRounded, ButtonVariants};
use gpui_kit::component::h_flex;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Icon, Selectable, Side, Sizable, Size};
use gpui_kit::gpui::Anchor;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Entity, FontWeight, InteractiveElement, Keystroke, Pixels,
    SharedString, Styled, Window, div, px,
};

use super::components::{
    CONTROL_HEIGHT_TOOLBAR, ICON_SIZE_COMPACT, SPACE_SM, TOOLBAR_HEIGHT, button_radius,
    control_icon, toolbar_icon_button, with_alpha,
};
use super::theme::UiColors;
use yomu::library::{SortBy, SortDirection};

pub struct MainToolbarProps<'a> {
    pub can_go_back: bool,
    pub view_title: Option<SharedString>,
    pub item_count: Option<SharedString>,
    pub sidebar_collapsed: bool,
    pub is_scrolled: bool,
    pub show_search: bool,
    pub search_input: Option<&'a Entity<InputState>>,
    pub show_view_switches: bool,
    pub is_grid_mode: bool,
    pub show_grouping: bool,
    pub is_group_series: bool,
    pub show_sort: bool,
    pub sort_by: SortBy,
    pub sort_direction: SortDirection,
    pub is_client_decorated: bool,
    pub selection_count: usize,
    pub is_all_selected: bool,
    pub on_select_all: Option<std::rc::Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
    pub on_unselect_all: Option<std::rc::Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
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
    on_change_sort: impl Fn(SortBy, SortDirection, &mut Window, &mut App) + 'static,
    on_close_window: Option<std::rc::Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
) -> AnyElement {
    let on_toggle_grid = std::rc::Rc::new(on_toggle_grid);
    let on_toggle_group_series = std::rc::Rc::new(on_toggle_group_series);
    let on_change_sort = std::rc::Rc::new(on_change_sort);
    let colors = props.colors;
    let button_radius = button_radius(props.radius, CONTROL_HEIGHT_TOOLBAR);

    div()
        .h(TOOLBAR_HEIGHT)
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
        // The three equal-width regions keep search centered in the main pane.
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(SPACE_SM)
                .child(toolbar_icon_button(
                    "toolbar-sidebar-toggle",
                    IconName::PanelLeft,
                    "Toggle sidebar",
                    props.radius,
                    on_toggle_sidebar,
                ))
                .when(props.can_go_back, |this| {
                    this.child(div().debug_selector(|| "group-back".into()).child(
                        toolbar_icon_button(
                            "group-back",
                            IconName::ChevronLeft,
                            "Back to library",
                            props.radius,
                            on_back,
                        ),
                    ))
                })
                .when_some(props.view_title, |this, title| {
                    this.child(
                        h_flex()
                            .items_baseline()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(colors.foreground)
                                    .child(title),
                            )
                            .when_some(props.item_count, |this, count| {
                                this.child(div().text_xs().text_color(colors.muted).child(count))
                            }),
                    )
                }),
        )
        // Center: Search input, responsive up to the preferred reading width.
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .justify_center()
                .when(props.show_search && props.selection_count == 0, |this| {
                    if let Some(input) = props.search_input {
                        this.child(
                            div()
                                .w_full()
                                .max_w(px(400.))
                                .min_w_0()
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
                                        .prefix(Icon::new(IconName::Search).size(ICON_SIZE_COMPACT))
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
                .flex_1()
                .min_w_0()
                .items_center()
                .justify_end()
                .gap(SPACE_SM)
                .when(props.selection_count > 0, |this| {
                    let on_select_all = props.on_select_all.clone();
                    let on_unselect_all = props.on_unselect_all.clone();
                    let on_remove_selected = props.on_remove_selected.clone();
                    let on_cancel_selection = props.on_cancel_selection.clone();
                    let is_all_selected = props.is_all_selected;
                    this.child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colors.foreground)
                            .child(format!("{} selected", props.selection_count)),
                    )
                    .child(if is_all_selected {
                        div().debug_selector(|| "unselect-all".into()).child(
                            Button::new("unselect-all")
                                .ghost()
                                .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
                                .rounded(ButtonRounded::Size(button_radius))
                                .label("Unselect all")
                                .tooltip("Clear selection")
                                .on_click(move |e, window, cx| {
                                    if let Some(f) = on_unselect_all.as_ref() {
                                        f(e, window, cx);
                                    }
                                }),
                        )
                    } else {
                        div().debug_selector(|| "select-all".into()).child(
                            Button::new("select-all")
                                .ghost()
                                .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
                                .rounded(ButtonRounded::Size(button_radius))
                                .label("Select all")
                                .tooltip("Select all in view")
                                .on_click(move |e, window, cx| {
                                    if let Some(f) = on_select_all.as_ref() {
                                        f(e, window, cx);
                                    }
                                }),
                        )
                    })
                    .child(
                        div().debug_selector(|| "select-remove".into()).child(
                            Button::new("select-remove")
                                .ghost()
                                .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
                                .rounded(ButtonRounded::Size(button_radius))
                                .label("Remove")
                                .tooltip("Remove selected books from library")
                                .accessibility_label("Remove selected books from library")
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
                                .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
                                .rounded(ButtonRounded::Size(button_radius))
                                .label("Cancel")
                                .tooltip("Cancel selection (Esc)")
                                .accessibility_label("Cancel selection (Esc)")
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
                        // 1. Folder Grouping segmented control: switches between Groups view and Volumes view
                        .when(props.show_grouping, |this| {
                            let on_toggle_group_series = on_toggle_group_series.clone();

                            this.child(
                                div()
                                    .debug_selector(|| "grouping-mode-toggle".into())
                                    .child(
                                        ButtonGroup::new("grouping-mode-segmented")
                                            .compact()
                                            .child(
                                                Button::new("mode-groups")
                                                    .ghost()
                                                    .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
                                                    .icon(control_icon(IconName::Folder))
                                                    .selected(props.is_group_series)
                                                    .tooltip("Groups view")
                                                    .accessibility_label("Groups view")
                                                    .on_click({
                                                        let on_toggle =
                                                            on_toggle_group_series.clone();
                                                        move |_, window, cx| {
                                                            on_toggle(true, window, cx)
                                                        }
                                                    }),
                                            )
                                            .child(
                                                Button::new("mode-volumes")
                                                    .ghost()
                                                    .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
                                                    .icon(control_icon(IconName::BookOpen))
                                                    .selected(!props.is_group_series)
                                                    .tooltip("Volumes view")
                                                    .accessibility_label("Volumes view")
                                                    .on_click({
                                                        let on_toggle =
                                                            on_toggle_group_series.clone();
                                                        move |_, window, cx| {
                                                            on_toggle(false, window, cx)
                                                        }
                                                    }),
                                            ),
                                    ),
                            )
                        })
                        // 2. Contextual view toggle: switches between Grid and List view
                        .when(props.show_view_switches, |this| {
                            let (icon, tooltip, next_is_grid) = if props.is_grid_mode {
                                (IconName::List, "Switch to List View", false)
                            } else {
                                (IconName::LayoutGrid, "Switch to Grid View", true)
                            };
                            let on_toggle_grid = on_toggle_grid.clone();

                            this.child(
                                div().debug_selector(|| "view-mode-toggle".into()).child(
                                    Button::new("view-mode-toggle")
                                        .ghost()
                                        .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
                                        .rounded(ButtonRounded::Size(button_radius))
                                        .icon(icon)
                                        .tooltip(tooltip)
                                        .accessibility_label(tooltip)
                                        .on_click(move |_, window, cx| {
                                            on_toggle_grid(next_is_grid, window, cx);
                                        }),
                                ),
                            )
                        })
                        // 2. Sort Dropdown (neutral appearance when idle)
                        .when(props.show_sort, |this| {
                            let on_change_sort = on_change_sort.clone();
                            let current_by = props.sort_by;
                            let current_dir = props.sort_direction;
                            this.child(
                                div().debug_selector(|| "toolbar-sort".into()).child(
                                    Button::new("toolbar-sort")
                                        .icon(control_icon(IconName::ArrowUpDown))
                                        .ghost()
                                        .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
                                        .rounded(ButtonRounded::Size(button_radius))
                                        .tooltip(format!(
                                            "Sort: {} ({})",
                                            current_by.label(),
                                            current_dir.label(current_by)
                                        ))
                                        .accessibility_label(format!(
                                            "Sort: {} ({})",
                                            current_by.label(),
                                            current_dir.label(current_by)
                                        ))
                                        .dropdown_menu_with_anchor(
                                            Anchor::TopRight,
                                            move |mut menu, _, _| {
                                                let on_change_sort = on_change_sort.clone();
                                                menu = menu.check_side(Side::Left).min_w(px(200.));
                                                for by in SortBy::ALL {
                                                    let on_change_sort = on_change_sort.clone();
                                                    let is_active = current_by == by;
                                                    let target_dir = if is_active {
                                                        current_dir.toggle()
                                                    } else {
                                                        by.default_direction()
                                                    };
                                                    menu = menu.item(
                                                        PopupMenuItem::new(by.label())
                                                            .checked(is_active)
                                                            .on_click(move |_, window, cx| {
                                                                on_change_sort(
                                                                    by, target_dir, window, cx,
                                                                );
                                                            }),
                                                    );
                                                }
                                                if current_by != SortBy::Default {
                                                    menu = menu.item(PopupMenuItem::separator());
                                                    for dir in [
                                                        SortDirection::Ascending,
                                                        SortDirection::Descending,
                                                    ] {
                                                        let on_change_sort = on_change_sort.clone();
                                                        let is_checked = current_dir == dir;
                                                        let label = dir.label(current_by);
                                                        menu = menu.item(
                                                            PopupMenuItem::new(label)
                                                                .checked(is_checked)
                                                                .on_click(move |_, window, cx| {
                                                                    on_change_sort(
                                                                        current_by, dir, window, cx,
                                                                    );
                                                                }),
                                                        );
                                                    }
                                                }
                                                menu
                                            },
                                        ),
                                ),
                            )
                        })
                })
                .when(props.is_client_decorated, |this| {
                    this.child(toolbar_icon_button(
                        "toolbar-csd-close",
                        IconName::Close,
                        "Close",
                        props.radius,
                        move |e, window, cx| {
                            if let Some(on_close) = on_close_window.as_ref() {
                                on_close(e, window, cx);
                            }
                        },
                    ))
                }),
        )
        .into_any_element()
}
