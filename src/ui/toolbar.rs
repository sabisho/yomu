//! Main toolbar matching macOS Yomu design reference.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
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

use super::components::{
    CONTROL_HEIGHT_TOOLBAR, ICON_SIZE_COMPACT, SPACE_2XS, SPACE_SM, TOOLBAR_HEIGHT, button_radius,
    inner_radius, open_shortcut_str, segmented_button, toolbar_icon_button, with_alpha,
};
use super::theme::UiColors;
use yomu::library::{SortBy, SortDirection};

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
    pub show_sort: bool,
    pub sort_by: SortBy,
    pub sort_direction: SortDirection,
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
    on_change_sort: impl Fn(SortBy, SortDirection, &mut Window, &mut App) + 'static,
    on_import_files: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_import_folder: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_close_window: Option<std::rc::Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
) -> AnyElement {
    let on_toggle_grid = std::rc::Rc::new(on_toggle_grid);
    let on_toggle_grid_false = on_toggle_grid.clone();
    let on_toggle_group_series = std::rc::Rc::new(on_toggle_group_series);
    let on_change_sort = std::rc::Rc::new(on_change_sort);
    let on_import_files = std::rc::Rc::new(on_import_files);
    let on_import_folder = std::rc::Rc::new(on_import_folder);
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
        // Left: Sidebar toggle + Back navigation (visible only when inside a group/series)
        .child(
            div()
                .flex()
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
                .items_center()
                .gap(SPACE_SM)
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
                                .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
                                .rounded(ButtonRounded::Size(button_radius))
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
                                .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
                                .rounded(ButtonRounded::Size(button_radius))
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
                                .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
                                .rounded(ButtonRounded::Size(button_radius))
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
                            let inner_r = inner_radius(button_radius, SPACE_2XS);
                            this.child(
                                div()
                                    .id("toolbar-grouping-segmented")
                                    .flex()
                                    .items_center()
                                    .h(CONTROL_HEIGHT_TOOLBAR)
                                    .bg(with_alpha(colors.surface, 0.6))
                                    .border_1()
                                    .border_color(colors.border)
                                    .rounded(button_radius)
                                    .p(SPACE_2XS)
                                    .gap(SPACE_2XS)
                                    .child(segmented_button(
                                        "toolbar-group-folders-toggle",
                                        IconName::Folder,
                                        "Group by series folders",
                                        props.is_group_series,
                                        inner_r,
                                        {
                                            let on_toggle = on_toggle_group_series.clone();
                                            move |_, window, cx| on_toggle(true, window, cx)
                                        },
                                    ))
                                    .child(segmented_button(
                                        "toolbar-group-volumes-toggle",
                                        IconName::BookOpen,
                                        "Show individual volumes",
                                        !props.is_group_series,
                                        inner_r,
                                        {
                                            let on_toggle = on_toggle_group_series.clone();
                                            move |_, window, cx| on_toggle(false, window, cx)
                                        },
                                    )),
                            )
                        })
                        // 2. View Mode switch: Grid vs List
                        .when(props.show_view_switches, |this| {
                            let inner_r = inner_radius(button_radius, SPACE_2XS);
                            this.child(
                                div()
                                    .id("toolbar-viewmode-segmented")
                                    .flex()
                                    .items_center()
                                    .h(CONTROL_HEIGHT_TOOLBAR)
                                    .bg(with_alpha(colors.surface, 0.6))
                                    .border_1()
                                    .border_color(colors.border)
                                    .rounded(button_radius)
                                    .p(SPACE_2XS)
                                    .gap(SPACE_2XS)
                                    .child(segmented_button(
                                        "view-mode-grid-toggle",
                                        IconName::LayoutGrid,
                                        "Grid view",
                                        props.is_grid_mode,
                                        inner_r,
                                        {
                                            let on_toggle_grid = on_toggle_grid.clone();
                                            move |_, window, cx| on_toggle_grid(true, window, cx)
                                        },
                                    ))
                                    .child(segmented_button(
                                        "view-mode-list-toggle",
                                        IconName::List,
                                        "List view",
                                        !props.is_grid_mode,
                                        inner_r,
                                        {
                                            let on_toggle_grid_false = on_toggle_grid_false.clone();
                                            move |_, window, cx| {
                                                on_toggle_grid_false(false, window, cx)
                                            }
                                        },
                                    )),
                            )
                        })
                        // 3. Sort Dropdown
                        .when(props.show_sort, |this| {
                            let on_change_sort = on_change_sort.clone();
                            let current_by = props.sort_by;
                            let current_dir = props.sort_direction;
                            let is_custom_sorted = current_by != SortBy::Default
                                || current_dir != SortDirection::Ascending;
                            this.child(
                                div().debug_selector(|| "toolbar-sort".into()).child(
                                    Button::new("toolbar-sort")
                                        .icon(IconName::ArrowUpDown)
                                        .ghost()
                                        .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
                                        .rounded(ButtonRounded::Size(button_radius))
                                        .selected(is_custom_sorted)
                                        .tooltip(format!(
                                            "Sort: {} ({})",
                                            current_by.label(),
                                            current_dir.label(current_by)
                                        ))
                                        .dropdown_menu_with_anchor(
                                            Anchor::TopRight,
                                            move |mut menu, _, _| {
                                                let on_change_sort = on_change_sort.clone();
                                                menu = menu.min_w(px(200.));
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
                        // 4. Import Button: unified ghost button matching 28px toolbar controls
                        .when(props.show_import, |this| {
                            let on_import_files = on_import_files.clone();
                            let on_import_folder = on_import_folder.clone();
                            this.child(
                                div().debug_selector(|| "add-files".into()).child(
                                    Button::new("add-files")
                                        .icon(IconName::Plus)
                                        .ghost()
                                        .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
                                        .rounded(ButtonRounded::Size(button_radius))
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
