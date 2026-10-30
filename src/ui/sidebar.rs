//! Navigation sidebar for Yomu's two-pane application shell.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, InteractiveElement, Pixels, ScrollHandle,
    Styled, Window, div, px,
};

use super::components::{macos_sidebar_bg, settings_shortcut_str, stat_badge, with_alpha};
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
    pub scroll_handle: &'a ScrollHandle,
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
    let sidebar_bg = macos_sidebar_bg(colors);

    let nav_item = |section: SidebarSection,
                    icon: IconName,
                    label: &'static str,
                    count: Option<usize>,
                    active: bool|
     -> AnyElement {
        let (bg, text_col, icon_col) = if active {
            (
                colors.accent.into(),
                colors.accent_foreground,
                colors.accent_foreground,
            )
        } else {
            (
                with_alpha(colors.foreground, 0.0),
                colors.foreground,
                colors.muted,
            )
        };

        let on_select = on_select.clone();
        let mut row = div()
            .id(ElementId::from(label))
            .h(px(32.))
            .px_3()
            .mx_2()
            .my_0p5()
            .rounded(props.radius)
            .flex()
            .items_center()
            .justify_between()
            .bg(bg)
            .text_color(text_col)
            .cursor_pointer()
            .when(!active, |this| {
                this.hover(|style| style.bg(with_alpha(colors.foreground, 0.06)))
            })
            .on_click(move |_, window, cx| on_select(section, window, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .child(Icon::new(icon).size(px(16.)).text_color(icon_col))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(if active {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .child(label),
                    ),
            );

        if let Some(c) = count {
            row = row.child(stat_badge(c, active, colors, props.radius));
        }

        row.into_any_element()
    };

    div()
        .w(px(240.))
        .h_full()
        .flex()
        .flex_col()
        .justify_between()
        .bg(sidebar_bg)
        .border_r_1()
        .border_color(colors.border)
        // Top Header: App Brand "Yomu" (offset on macOS for native traffic lights)
        .child(
            div()
                .h(px(52.))
                .w_full()
                .when(cfg!(target_os = "macos"), |s| s.pl(px(80.)))
                .when(!cfg!(target_os = "macos"), |s| s.px_4())
                .flex()
                .items_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(IconName::BookOpen)
                                .size(px(17.))
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
            div()
                .id("sidebar-nav-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .track_scroll(props.scroll_handle)
                .scroll_bounce(true)
                .scroll_wheel(false)
                .flex()
                .flex_col()
                // Main Navigation
                .child(nav_item(
                    SidebarSection::ContinueReading,
                    IconName::Play,
                    "Continue Reading",
                    Some(props.continue_reading_count),
                    props.current_section == SidebarSection::ContinueReading,
                ))
                .child(nav_item(
                    SidebarSection::AllManga,
                    IconName::LayoutGrid,
                    "All Manga",
                    Some(props.all_manga_count),
                    props.current_section == SidebarSection::AllManga,
                ))
                .child(nav_item(
                    SidebarSection::Collections,
                    IconName::Folder,
                    "Collections",
                    Some(props.collections_count),
                    props.current_section == SidebarSection::Collections,
                ))
                .child(nav_item(
                    SidebarSection::History,
                    IconName::Clock,
                    "History",
                    None,
                    props.current_section == SidebarSection::History,
                ))
                .child(nav_item(
                    SidebarSection::Downloads,
                    IconName::Download,
                    "Downloads",
                    Some(props.downloads_count),
                    props.current_section == SidebarSection::Downloads,
                )),
        )
        // Settings bar at the bottom
        .child(
            div().p_3().flex().items_center().justify_end().child(
                div()
                    .id("sidebar-settings-btn")
                    .size(px(28.))
                    .rounded(props.radius)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|style| style.bg(with_alpha(colors.foreground, 0.08)))
                    .tooltip(|window, cx| {
                        Tooltip::new(format!("Settings ({})", settings_shortcut_str()))
                            .build(window, cx)
                    })
                    .on_click(on_settings)
                    .child(
                        Icon::new(IconName::Settings)
                            .size(px(16.))
                            .text_color(colors.muted),
                    ),
            ),
        )
        .into_any_element()
}
