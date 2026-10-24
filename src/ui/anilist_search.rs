//! AniList Search view matching the reference design.

use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{Icon, IconName, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, Entity, FontWeight, InteractiveElement, MouseButton,
    MouseDownEvent, Pixels, ScrollHandle, ScrollWheelEvent, SharedString, Styled, Window, div, img,
    px,
};
use yomu::anilist::AniListMedia;

use super::components::{genre_tag, search_shortcut_str, with_alpha};
use super::theme::UiColors;

pub struct AniListSearchProps<'a> {
    pub scroll_handle: &'a ScrollHandle,
    pub search_input: &'a Entity<InputState>,
    pub selected_filter: usize, // 0 = All, 1 = Title, 2 = Author, 3 = Genre, 4 = Tag
    pub results: &'a [AniListMedia],
    pub is_searching: bool,
    pub colors: UiColors,
    pub radius: Pixels,
}

pub fn render_anilist_search(
    props: AniListSearchProps,
    on_scroll_wheel: impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static,
    on_mouse_down: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    on_select_filter: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_add_or_match: impl Fn(u64, &mut Window, &mut App) + 'static,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_select_filter = std::rc::Rc::new(on_select_filter);
    let on_add_or_match = std::rc::Rc::new(on_add_or_match);
    let colors = props.colors;
    let filters = ["All", "Title", "Author", "Genre", "Tag"];

    div()
        .id("anilist-search-container")
        .track_scroll(props.scroll_handle)
        .size_full()
        .overflow_y_scroll()
        .on_scroll_wheel(on_scroll_wheel)
        .on_mouse_down(MouseButton::Left, on_mouse_down)
        .p_6()
        .flex()
        .flex_col()
        .gap_4()
        // Top Search Bar Row
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .rounded(props.radius)
                        .border_1()
                        .border_color(colors.accent)
                        .bg(with_alpha(colors.surface, 0.7))
                        .child(
                            Input::new(props.search_input)
                                .id("anilist-search-input")
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
                        ),
                )
                .child(
                    Button::new("anilist-cancel-btn")
                        .ghost()
                        .label("Cancel")
                        .rounded(ButtonRounded::Medium)
                        .small()
                        .on_click(on_close),
                ),
        )
        // Filter Chips Row
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .children(filters.iter().enumerate().map(|(idx, &name)| {
                    let on_select_filter = on_select_filter.clone();
                    let active = props.selected_filter == idx;
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "filter-chip-{idx}"
                        ))))
                        .px_3()
                        .py_1()
                        .rounded(props.radius)
                        .bg(if active {
                            with_alpha(colors.accent, 1.0)
                        } else {
                            with_alpha(colors.foreground, 0.07)
                        })
                        .text_xs()
                        .font_weight(if active {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if active {
                            colors.accent_foreground
                        } else {
                            colors.foreground
                        })
                        .cursor_pointer()
                        .on_click(move |_, window, cx| on_select_filter(idx, window, cx))
                        .child(name)
                })),
        )
        // Results Area
        .child(if props.is_searching {
            div()
                .w_full()
                .p_12()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    Icon::new(IconName::RefreshCw)
                        .size(px(24.))
                        .text_color(colors.accent),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.muted)
                        .child("Searching AniList GraphQL..."),
                )
                .into_any_element()
        } else if props.results.is_empty() {
            div()
                .w_full()
                .p_12()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Search)
                        .size(px(36.))
                        .text_color(colors.muted),
                )
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.foreground)
                        .child("Search Manga on AniList"),
                )
                .child(div().text_xs().text_color(colors.muted).child(
                    "Type a title above to discover manga, metadata, and link to your library.",
                ))
                .into_any_element()
        } else {
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap_3()
                .children(props.results.iter().map(|item| {
                    let on_add_or_match = on_add_or_match.clone();
                    let media_id = item.id;
                    let display_title = item.title.preferred().to_string();
                    let subtitle = item.title.subtitle().map(String::from);
                    let year_str = item
                        .start_year
                        .map(|y| y.to_string())
                        .unwrap_or_else(|| "2024".to_string());
                    let status_str = item.status.as_deref().unwrap_or("Ongoing");
                    let meta_line = format!("Manga · {year_str} · {status_str}");
                    let cover_url = item.cover_url().map(String::from);

                    div()
                        .w_full()
                        .p_3()
                        .rounded(props.radius)
                        .bg(with_alpha(colors.surface, 0.65))
                        .border_1()
                        .border_color(colors.border)
                        .hover(|style| style.bg(with_alpha(colors.foreground, 0.05)))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3p5()
                                .child(
                                    div()
                                        .w(px(56.))
                                        .h(px(80.))
                                        .rounded(px(4.))
                                        .overflow_hidden()
                                        .bg(with_alpha(colors.foreground, 0.06))
                                        .child(if let Some(url) = cover_url {
                                            img(url)
                                                .size_full()
                                                .object_fit(gpui_kit::ObjectFit::Cover)
                                                .into_any_element()
                                        } else {
                                            Icon::new(IconName::BookOpen)
                                                .size(px(24.))
                                                .text_color(colors.muted)
                                                .into_any_element()
                                        }),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(colors.foreground)
                                                .child(display_title),
                                        )
                                        .children(subtitle.map(|sub| {
                                            div().text_xs().text_color(colors.muted).child(sub)
                                        }))
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(colors.muted)
                                                .child(meta_line),
                                        )
                                        .child(div().flex().flex_wrap().gap_1().children(
                                            item.genres.iter().take(3).map(|g| {
                                                genre_tag(g.as_str(), colors, props.radius)
                                            }),
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .id(ElementId::from(SharedString::from(format!(
                                    "add-anilist-media-{media_id}"
                                ))))
                                .size(px(32.))
                                .rounded(props.radius)
                                .bg(with_alpha(colors.accent, 0.15))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(|s| s.bg(with_alpha(colors.accent, 0.35)))
                                .on_click(move |_, window, cx| {
                                    on_add_or_match(media_id, window, cx)
                                })
                                .child(
                                    Icon::new(IconName::Plus)
                                        .size(px(16.))
                                        .text_color(colors.accent),
                                ),
                        )
                }))
                .into_any_element()
        })
        .into_any_element()
}
