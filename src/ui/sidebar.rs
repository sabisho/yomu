//! Navigation sidebar for Yomu's two-pane application shell.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::sidebar::{
    Sidebar, SidebarCollapsible, SidebarGroup, SidebarMenu, SidebarMenuItem,
};
use gpui_kit::component::{Collapsible, Icon, Side, h_flex, v_flex};
use gpui_kit::gpui::Anchor;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, FontWeight, Pixels, ScrollHandle, Styled, Window, div, px,
};

use super::components::{
    CONTROL_HEIGHT_ACTION, SPACE_SM, TOOLBAR_HEIGHT, button_radius, open_shortcut_str,
    settings_shortcut_str, stat_badge,
};
use super::theme::UiColors;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainView {
    ContinueReading,
    AllManga,
    Details(String),
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarSection {
    #[default]
    ContinueReading,
    AllManga,
}

pub struct SidebarProps<'a> {
    #[allow(dead_code)]
    pub scroll_handle: &'a ScrollHandle,
    pub collapsed: bool,
    pub current_section: SidebarSection,
    pub continue_reading_count: usize,
    pub all_manga_count: usize,
    pub colors: UiColors,
    pub radius: Pixels,
}

pub fn render_sidebar(
    props: SidebarProps,
    on_select: impl Fn(SidebarSection, &mut Window, &mut App) + 'static,
    on_settings: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_import_files: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_import_folder: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_select = std::rc::Rc::new(on_select);
    let on_settings = std::rc::Rc::new(on_settings);
    let on_import_files = std::rc::Rc::new(on_import_files);
    let on_import_folder = std::rc::Rc::new(on_import_folder);
    let colors = props.colors;

    Sidebar::new("app-sidebar")
        .side(Side::Left)
        .w(px(240.))
        .collapsible(SidebarCollapsible::Icon)
        .collapsed(props.collapsed)
        .header(
            h_flex()
                .h(TOOLBAR_HEIGHT)
                .w_full()
                .when(cfg!(target_os = "macos"), |s| s.pl(px(80.)))
                .when(!cfg!(target_os = "macos"), |s| s.px_3())
                .items_center()
                .when(!props.collapsed, |this| {
                    this.child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Icon::new(IconName::BookOpen)
                                    .size(px(20.))
                                    .text_color(colors.accent),
                            )
                            .child(
                                div()
                                    .flex_col()
                                    .gap(px(1.))
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(colors.accent)
                                            .child("YOMU"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(colors.muted)
                                            .child("Manga & Comic Reader"),
                                    ),
                            ),
                    )
                })
                .when(props.collapsed, |this| {
                    this.justify_center().child(
                        Icon::new(IconName::BookOpen)
                            .size(px(20.))
                            .text_color(colors.accent),
                    )
                }),
        )
        .child(
            SidebarGroup::new("LIBRARY")
                .collapsed(props.collapsed)
                .child(
                    SidebarMenu::new()
                        .child(
                            SidebarMenuItem::new("Continue Reading")
                                .icon(Icon::new(IconName::BookOpen).size(px(16.)))
                                .active(props.current_section == SidebarSection::ContinueReading)
                                .on_click({
                                    let on_select = on_select.clone();
                                    move |_, window, cx| {
                                        on_select(SidebarSection::ContinueReading, window, cx)
                                    }
                                })
                                .when(props.continue_reading_count > 0, |this| {
                                    let count = props.continue_reading_count;
                                    let is_active =
                                        props.current_section == SidebarSection::ContinueReading;
                                    let radius = props.radius;
                                    this.suffix(move |_, _| {
                                        stat_badge(count, is_active, colors, radius)
                                    })
                                }),
                        )
                        .child(
                            SidebarMenuItem::new("All Manga")
                                .icon(Icon::new(IconName::LayoutGrid).size(px(16.)))
                                .active(props.current_section == SidebarSection::AllManga)
                                .on_click({
                                    let on_select = on_select.clone();
                                    move |_, window, cx| {
                                        on_select(SidebarSection::AllManga, window, cx)
                                    }
                                })
                                .when(props.all_manga_count > 0, |this| {
                                    let count = props.all_manga_count;
                                    let is_active =
                                        props.current_section == SidebarSection::AllManga;
                                    let radius = props.radius;
                                    this.suffix(move |_, _| {
                                        stat_badge(count, is_active, colors, radius)
                                    })
                                }),
                        ),
                ),
        )
        .footer({
            let on_settings = on_settings.clone();
            let on_import_files = on_import_files.clone();
            let on_import_folder = on_import_folder.clone();
            let button_r = button_radius(props.radius, CONTROL_HEIGHT_ACTION);

            let import_btn = div()
                .when(!props.collapsed, |this| this.w_full())
                .debug_selector(|| "add-files".into())
                .child({
                    let mut btn = Button::new("sidebar-import-btn")
                        .ghost()
                        .rounded(ButtonRounded::Size(button_r))
                        .tooltip(format!("Import manga ({})", open_shortcut_str()))
                        .tooltip_placement(gpui_kit::component::Placement::Right)
                        .accessibility_label("Import manga");

                    if props.collapsed {
                        btn = btn
                            .size(CONTROL_HEIGHT_ACTION)
                            .icon(Icon::new(IconName::Plus).size(px(16.)));
                    } else {
                        btn = btn.w_full().h(CONTROL_HEIGHT_ACTION).px_2().child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .gap_2()
                                .child(Icon::new(IconName::Plus).size(px(16.)))
                                .child(div().text_sm().child("Import")),
                        );
                    }

                    btn.dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, _| {
                        let on_import_files = on_import_files.clone();
                        let on_import_folder = on_import_folder.clone();
                        menu = menu
                            .min_w(px(180.))
                            .item(
                                PopupMenuItem::new("Import Files…")
                                    .icon(IconName::FilePlus)
                                    .on_click(move |e, window, cx| on_import_files(e, window, cx)),
                            )
                            .item(
                                PopupMenuItem::new("Import Folder…")
                                    .icon(IconName::FolderPlus)
                                    .on_click(move |e, window, cx| on_import_folder(e, window, cx)),
                            );
                        menu
                    })
                });

            let settings_btn = div()
                .when(!props.collapsed, |this| this.w_full())
                .debug_selector(|| "sidebar-settings-btn".into())
                .child({
                    let mut btn = Button::new("sidebar-settings-btn")
                        .ghost()
                        .rounded(ButtonRounded::Size(button_r))
                        .tooltip(format!("Settings ({})", settings_shortcut_str()))
                        .tooltip_placement(gpui_kit::component::Placement::Right)
                        .accessibility_label("Settings");

                    if props.collapsed {
                        btn = btn
                            .size(CONTROL_HEIGHT_ACTION)
                            .icon(Icon::new(IconName::Settings).size(px(16.)));
                    } else {
                        btn = btn.w_full().h(CONTROL_HEIGHT_ACTION).px_2().child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .gap_2()
                                .child(Icon::new(IconName::Settings).size(px(16.)))
                                .child(div().text_sm().child("Settings")),
                        );
                    }

                    btn.on_click(move |e, window, cx| on_settings(e, window, cx))
                });

            div()
                .w_full()
                .when(props.collapsed, |this| this.px_2())
                .when(!props.collapsed, |this| this.px_3())
                .pb_3()
                .pt_2()
                .border_t_1()
                .border_color(colors.border)
                .child(
                    v_flex()
                        .w_full()
                        .gap(SPACE_SM)
                        .when(!props.collapsed, |this| this.items_start())
                        .when(props.collapsed, |this| this.items_center())
                        .child(import_btn)
                        .child(settings_btn),
                )
        })
        .into_any_element()
}
