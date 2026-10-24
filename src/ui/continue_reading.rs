//! Continue Reading view with Hero banner, Up Next queue, and Recently Added grid.

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, InteractiveElement, MouseButton,
    MouseDownEvent, Pixels, ScrollHandle, ScrollWheelEvent, SharedString, Styled, Window, div, img,
    px,
};
use yomu::library::Book;

use super::components::{genre_tag, with_alpha};
use super::theme::UiColors;

pub struct ContinueReadingItem<'a> {
    pub book: &'a Book,
    pub cover_path: PathBuf,
    pub display_title: &'a str,
    pub chapter_label: String,
    pub progress: f32,
    pub genres: Vec<String>,
}

pub struct ContinueReadingProps<'a> {
    pub scroll_handle: &'a ScrollHandle,
    pub hero: Option<ContinueReadingItem<'a>>,
    pub queue: Vec<ContinueReadingItem<'a>>,
    pub recent: Vec<ContinueReadingItem<'a>>,
    pub colors: UiColors,
    pub radius: Pixels,
    pub border_px: f32,
    pub card_w: f32,
    pub cover_h: f32,
}

pub fn render_continue_reading_view(
    props: ContinueReadingProps,
    on_scroll_wheel: impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static,
    on_mouse_down: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    on_continue_hero: impl Fn(&str, &mut Window, &mut App) + 'static,
    on_open_details: impl Fn(&str, &mut Window, &mut App) + 'static,
    on_see_all: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_book_menu: impl Fn(&str, (f32, f32), &mut Window, &mut App) + 'static,
    on_import: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_open_details = std::rc::Rc::new(on_open_details);
    let on_book_menu = std::rc::Rc::new(on_book_menu);
    let colors = props.colors;

    if props.hero.is_none() && props.recent.is_empty() {
        return div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .child(
                Icon::new(IconName::BookOpen)
                    .size(px(48.))
                    .text_color(colors.muted),
            )
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(colors.foreground)
                    .child("Your Library is Empty"),
            )
            .child(div().text_sm().text_color(colors.muted).child(
                "Import comic archives (.cbz, .cbr, .zip) or manga folders to start reading.",
            ))
            .child(
                Button::new("empty-import-button")
                    .primary()
                    .label("+ Import Manga")
                    .rounded(ButtonRounded::Medium)
                    .bg(colors.accent)
                    .text_color(colors.accent_foreground)
                    .on_click(on_import),
            )
            .into_any_element();
    }

    div()
        .id("continue-reading-container")
        .track_scroll(props.scroll_handle)
        .size_full()
        .overflow_y_scroll()
        .on_scroll_wheel(on_scroll_wheel)
        .on_mouse_down(MouseButton::Left, on_mouse_down)
        .p_6()
        .flex()
        .flex_col()
        .gap_6()
        // Section 1: Continue Reading
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .text_color(colors.foreground)
                        .child("Continue Reading"),
                )
                // Hero card + Up Next Queue
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap_4()
                        // Hero card
                        .child(if let Some(hero) = props.hero {
                            let hero_id = hero.book.id.clone();
                            let hero_id_clone = hero_id.clone();
                            let on_book_menu_hero = on_book_menu.clone();
                            div()
                                .flex_1()
                                .min_w(px(320.))
                                .min_h(px(210.))
                                .rounded(props.radius)
                                .bg(with_alpha(colors.surface, 0.7))
                                .when(props.border_px > 0.0, |s| {
                                    s.border(px(props.border_px)).border_color(colors.border)
                                })
                                .overflow_hidden()
                                .flex()
                                .flex_row()
                                .child(
                                    div()
                                        .w(px(140.))
                                        .flex_none()
                                        .h_full()
                                        .rounded_l(props.radius)
                                        .overflow_hidden()
                                        .bg(with_alpha(colors.foreground, 0.05))
                                        .child(
                                            img(hero.cover_path)
                                                .size_full()
                                                .rounded_l(props.radius)
                                                .object_fit(gpui_kit::ObjectFit::Cover),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .p_4()
                                        .flex()
                                        .flex_col()
                                        .justify_between()
                                        .gap_2()
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .gap_1p5()
                                                .child(
                                                    div()
                                                        .text_base()
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(colors.foreground)
                                                        .truncate()
                                                        .child(hero.display_title.to_string()),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(colors.muted)
                                                        .truncate()
                                                        .child(hero.chapter_label),
                                                )
                                                .child(
                                                    div()
                                                        .pt_2()
                                                        .pb_1()
                                                        .flex()
                                                        .items_center()
                                                        .gap_3()
                                                        .child(
                                                            div()
                                                                .w_full()
                                                                .max_w(px(220.))
                                                                .child(Progress::new("hero-progress").value(hero.progress * 100.0)),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .text_color(colors.muted)
                                                                .child(format!("{:.0}%", hero.progress * 100.0)),
                                                        ),
                                                )
                                                .child(
                                                    div()
                                                        .flex()
                                                        .flex_wrap()
                                                        .gap_1p5()
                                                        .pt_1()
                                                        .children(hero.genres.iter().map(|g| genre_tag(g.as_str(), colors, props.radius))),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_3()
                                                .child(
                                                    Button::new("hero-continue-btn")
                                                        .primary()
                                                        .label("▶ Continue Reading")
                                                        .rounded(ButtonRounded::Medium)
                                                        .bg(colors.accent)
                                                        .text_color(colors.accent_foreground)
                                                        .on_click(move |_, window, cx| {
                                                            on_continue_hero(&hero_id, window, cx)
                                                        }),
                                                )
                                                .child(
                                                    div()
                                                        .id("hero-more-btn")
                                                        .size(px(32.))
                                                        .rounded(props.radius)
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .bg(with_alpha(colors.foreground, 0.06))
                                                        .cursor_pointer()
                                                        .on_click(move |event: &ClickEvent, window, cx| {
                                                            let pos: (f32, f32) = (event.position().x.into(), event.position().y.into());
                                                            on_book_menu_hero(&hero_id_clone, pos, window, cx);
                                                        })
                                                        .child(
                                                            Icon::new(IconName::Ellipsis)
                                                                .size(px(16.))
                                                                .text_color(colors.foreground),
                                                        ),
                                                ),
                                        ),
                                )
                                .into_any_element()
                        } else {
                            div().into_any_element()
                        })
                        // Up Next Queue Column
                        .child(
                            div()
                                .w(px(280.))
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .gap_2()
                                .children(props.queue.iter().map(|item| {
                                    let on_open_details = on_open_details.clone();
                                    let book_id = item.book.id.clone();
                                    div()
                                        .id(ElementId::from(SharedString::from(format!("queue-item-{}", item.book.id))))
                                        .h(px(68.))
                                        .p_2()
                                        .rounded(props.radius)
                                        .bg(with_alpha(colors.surface, 0.6))
                                        .when(props.border_px > 0.0, |s| {
                                            s.border(px(props.border_px)).border_color(colors.border)
                                        })
                                        .cursor_pointer()
                                        .hover(|style| style.bg(with_alpha(colors.foreground, 0.06)))
                                        .on_click(move |_, window, cx| {
                                            on_open_details(&book_id, window, cx)
                                        })
                                        .flex()
                                        .flex_row()
                                        .gap_3()
                                        .child(
                                            div()
                                                .w(px(46.))
                                                .h_full()
                                                .rounded(px(4.))
                                                .overflow_hidden()
                                                .bg(with_alpha(colors.foreground, 0.05))
                                                .child(
                                                    img(item.cover_path.clone())
                                                        .size_full()
                                                        .rounded(px(4.))
                                                        .object_fit(gpui_kit::ObjectFit::Cover),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .flex()
                                                .flex_col()
                                                .justify_center()
                                                .gap_0p5()
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(colors.foreground)
                                                        .truncate()
                                                        .child(item.display_title.to_string()),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(colors.muted)
                                                        .truncate()
                                                        .child(format!(
                                                            "{} · {:.0}%",
                                                            item.chapter_label,
                                                            item.progress * 100.0
                                                        )),
                                                ),
                                        )
                                })),
                        ),
                ),
        )
        // Section 2: Recently Added
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .pt_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(colors.foreground)
                                .child("Recently Added"),
                        )
                        .child(
                            div()
                                .id("recently-added-see-all-btn")
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(colors.accent)
                                .cursor_pointer()
                                .hover(|s| s.underline())
                                .on_click(on_see_all)
                                .child("See All"),
                        ),
                )
                // Grid of Cover Cards
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap_4()
                        .children(props.recent.iter().map(|item| {
                            let on_open_details = on_open_details.clone();
                            let on_book_menu = on_book_menu.clone();
                            let book_id = item.book.id.clone();
                            let book_id_clone = book_id.clone();
                            div()
                                .id(ElementId::from(SharedString::from(format!("recent-card-{}", item.book.id))))
                                .w(px(props.card_w))
                                .flex()
                                .flex_col()
                                .rounded(props.radius)
                                .bg(with_alpha(colors.surface, 0.6))
                                .when(props.border_px > 0.0, |s| {
                                    s.border(px(props.border_px)).border_color(colors.border)
                                })
                                .overflow_hidden()
                                .cursor_pointer()
                                .hover(|style| {
                                    style
                                        .border_color(colors.accent)
                                        .shadow_md()
                                })
                                .on_click(move |_, window, cx| {
                                    on_open_details(&book_id, window, cx)
                                })
                                .child(
                                    div()
                                        .w_full()
                                        .h(px(props.cover_h))
                                        .rounded_t(props.radius)
                                        .bg(with_alpha(colors.foreground, 0.04))
                                        .overflow_hidden()
                                        .child(
                                            img(item.cover_path.clone())
                                                .size_full()
                                                .rounded_t(props.radius)
                                                .object_fit(gpui_kit::ObjectFit::Cover),
                                        ),
                                )
                                .child(
                                    div()
                                        .rounded_b(props.radius)
                                        .p_2()
                                        .flex()
                                        .items_start()
                                        .justify_between()
                                        .gap_1()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .flex()
                                                .flex_col()
                                                .gap_0p5()
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(colors.foreground)
                                                        .truncate()
                                                        .child(item.display_title.to_string()),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(colors.muted)
                                                        .truncate()
                                                        .child(format!(
                                                            "{} · {:.0}%",
                                                            item.chapter_label,
                                                            item.progress * 100.0
                                                        )),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .id(ElementId::from(SharedString::from(format!("recent-card-menu-{}", item.book.id))))
                                                .size(px(20.))
                                                .flex_none()
                                                .rounded(props.radius)
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .cursor_pointer()
                                                .hover(|s| s.bg(with_alpha(colors.foreground, 0.1)))
                                                .on_click(move |event: &ClickEvent, window, cx| {
                                                    let pos: (f32, f32) = (event.position().x.into(), event.position().y.into());
                                                    on_book_menu(&book_id_clone, pos, window, cx);
                                                })
                                                .child(
                                                    Icon::new(IconName::Ellipsis)
                                                        .size(px(12.))
                                                        .text_color(colors.muted),
                                                ),
                                        ),
                                )
                        })),
                ),
        )
        .into_any_element()
}
