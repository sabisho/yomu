//! Navigation sidebar for Yomu's two-pane application shell.

use gpui_kit::assets::IconName;
use gpui_kit::component::h_flex;
use gpui_kit::component::sidebar::{
    Sidebar, SidebarCollapsible, SidebarFooter, SidebarMenu, SidebarMenuItem,
};
use gpui_kit::component::{Icon, Side};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, FontWeight, Pixels, ScrollHandle, Styled, Window, div, px,
};

use super::components::{TOOLBAR_HEIGHT, settings_shortcut_str, stat_badge, toolbar_icon_button};
use super::theme::UiColors;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainView {
    ContinueReading,
    AllManga,
    Collections,
    History,
    Downloads,
    Details(String),
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarSection {
    #[default]
    ContinueReading,
    AllManga,
    Collections,
    History,
    Downloads,
}

pub struct SidebarProps<'a> {
    #[allow(dead_code)]
    pub scroll_handle: &'a ScrollHandle,
    pub collapsed: bool,
    pub current_section: SidebarSection,
    pub continue_reading_count: usize,
    pub all_manga_count: usize,
    pub collections_count: usize,
    pub downloads_count: usize,
    pub colors: UiColors,
    pub radius: Pixels,
}

pub fn render_sidebar(
    props: SidebarProps,
    on_select: impl Fn(SidebarSection, &mut Window, &mut App) + 'static,
    on_settings: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_select = std::rc::Rc::new(on_select);
    let colors = props.colors;

    Sidebar::new("app-sidebar")
        .side(Side::Left)
        .w(px(240.))
        .collapsible(SidebarCollapsible::Offcanvas)
        .collapsed(props.collapsed)
        .header(
            h_flex()
                .h(TOOLBAR_HEIGHT)
                .w_full()
                .when(cfg!(target_os = "macos"), |s| s.pl(px(80.)))
                .when(!cfg!(target_os = "macos"), |s| s.px_3())
                .items_center()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Icon::new(IconName::BookOpen)
                                .size(px(18.))
                                .text_color(colors.accent),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(colors.foreground)
                                .child("Yomu"),
                        ),
                ),
        )
        .child(
            SidebarMenu::new()
                .child(
                    SidebarMenuItem::new("Continue Reading")
                        .icon(IconName::Play)
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
                            this.suffix(move |_, _| stat_badge(count, is_active, colors, radius))
                        }),
                )
                .child(
                    SidebarMenuItem::new("All Manga")
                        .icon(IconName::LayoutGrid)
                        .active(props.current_section == SidebarSection::AllManga)
                        .on_click({
                            let on_select = on_select.clone();
                            move |_, window, cx| on_select(SidebarSection::AllManga, window, cx)
                        })
                        .when(props.all_manga_count > 0, |this| {
                            let count = props.all_manga_count;
                            let is_active = props.current_section == SidebarSection::AllManga;
                            let radius = props.radius;
                            this.suffix(move |_, _| stat_badge(count, is_active, colors, radius))
                        }),
                )
                .child(
                    SidebarMenuItem::new("Collections")
                        .icon(IconName::Folder)
                        .active(props.current_section == SidebarSection::Collections)
                        .on_click({
                            let on_select = on_select.clone();
                            move |_, window, cx| on_select(SidebarSection::Collections, window, cx)
                        })
                        .when(props.collections_count > 0, |this| {
                            let count = props.collections_count;
                            let is_active = props.current_section == SidebarSection::Collections;
                            let radius = props.radius;
                            this.suffix(move |_, _| stat_badge(count, is_active, colors, radius))
                        }),
                )
                .child(
                    SidebarMenuItem::new("History")
                        .icon(IconName::Clock)
                        .active(props.current_section == SidebarSection::History)
                        .on_click({
                            let on_select = on_select.clone();
                            move |_, window, cx| on_select(SidebarSection::History, window, cx)
                        }),
                )
                .child(
                    SidebarMenuItem::new("Downloads")
                        .icon(IconName::Download)
                        .active(props.current_section == SidebarSection::Downloads)
                        .on_click({
                            let on_select = on_select.clone();
                            move |_, window, cx| on_select(SidebarSection::Downloads, window, cx)
                        })
                        .when(props.downloads_count > 0, |this| {
                            let count = props.downloads_count;
                            let is_active = props.current_section == SidebarSection::Downloads;
                            let radius = props.radius;
                            this.suffix(move |_, _| stat_badge(count, is_active, colors, radius))
                        }),
                ),
        )
        .footer(
            SidebarFooter::new()
                .justify_end()
                .child(toolbar_icon_button(
                    "sidebar-settings-btn",
                    IconName::Settings,
                    format!("Settings ({})", settings_shortcut_str()),
                    props.radius,
                    on_settings,
                )),
        )
        .into_any_element()
}
