//! Navigation sidebar for Yomu's two-pane application shell.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, InteractiveElement, Pixels, ScrollHandle,
    SharedString, Styled, Window, div, px,
};

use super::components::{
    macos_sidebar_bg, section_header, settings_shortcut_str, stat_badge, with_alpha,
};
use super::theme::UiColors;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainView {
    ContinueReading,
    AllManga,
    Collections,
    History,
    Downloads,
    AniListCategory(AniListCategory),
    Details(String),
    AniListSearch,
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
    AniList(AniListCategory),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AniListCategory {
    MyList,
    Planning,
    Reading,
    Completed,
    Paused,
    Dropped,
}

impl AniListCategory {
    pub const ALL: [AniListCategory; 6] = [
        AniListCategory::MyList,
        AniListCategory::Planning,
        AniListCategory::Reading,
        AniListCategory::Completed,
        AniListCategory::Paused,
        AniListCategory::Dropped,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::MyList => "My List",
            Self::Planning => "Planning",
            Self::Reading => "Reading",
            Self::Completed => "Completed",
            Self::Paused => "Paused",
            Self::Dropped => "Dropped",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Self::MyList => IconName::List,
            Self::Planning => IconName::Bookmark,
            Self::Reading => IconName::BookOpen,
            Self::Completed => IconName::Check,
            Self::Paused => IconName::Pause,
            Self::Dropped => IconName::Clock,
        }
    }
}

pub struct SidebarProps<'a> {
    pub scroll_handle: &'a ScrollHandle,
    pub current_section: SidebarSection,
    pub continue_reading_count: usize,
    pub all_manga_count: usize,
    pub collections_count: usize,
    pub downloads_count: usize,
    pub anilist_counts: [usize; 6],
    pub user_name: SharedString,
    pub anilist_connected: bool,
    pub colors: UiColors,
    pub radius: Pixels,
}

pub fn render_sidebar(
    props: SidebarProps,
    on_select: impl Fn(SidebarSection, &mut Window, &mut App) + 'static,
    on_settings: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_open_anilist: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
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
        .child(gpui_kit::base::ScrollBounce::new(
            "sidebar-scroll-bounce",
            props.scroll_handle,
            div()
                .id("sidebar-nav-scroll")
                .track_scroll(props.scroll_handle)
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .overflow_y_scroll()
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
                ))
                // AniList Section
                .child(section_header("ANILIST", colors))
                .children(AniListCategory::ALL.iter().enumerate().map(|(idx, &cat)| {
                    let active = props.current_section == SidebarSection::AniList(cat);
                    nav_item(
                        SidebarSection::AniList(cat),
                        cat.icon(),
                        cat.label(),
                        Some(props.anilist_counts[idx]),
                        active,
                    )
                })),
        ))
        // User Profile & Settings bar at the bottom
        .child(
            div()
                .p_3()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .id("sidebar-profile-card")
                        .flex()
                        .items_center()
                        .gap_2p5()
                        .cursor_pointer()
                        .rounded(props.radius)
                        .p_1()
                        .hover(|style| style.bg(with_alpha(colors.foreground, 0.06)))
                        .tooltip(|window, cx| {
                            Tooltip::new("AniList integration & profile").build(window, cx)
                        })
                        .on_click(on_open_anilist)
                        .child(
                            div()
                                .size(px(32.))
                                .rounded(props.radius)
                                .bg(with_alpha(colors.accent, 0.2))
                                .border_1()
                                .border_color(colors.accent)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(IconName::User)
                                        .size(px(16.))
                                        .text_color(colors.accent),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(colors.foreground)
                                        .child(props.user_name),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(if props.anilist_connected {
                                            gpui_kit::rgb(0x34C759)
                                        } else {
                                            colors.muted.into()
                                        })
                                        .child(if props.anilist_connected {
                                            "AniList Connected"
                                        } else {
                                            "Connect AniList"
                                        }),
                                ),
                        ),
                )
                .child(
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
