//! Library card, folder, and list row rendering primitives for Yomu's two-pane application shell.

#![allow(clippy::too_many_arguments)]
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, ClickEvent, ElementId, FontWeight, InteractiveElement, MouseButton, MouseDownEvent,
    ObjectFit, Pixels, SharedString, Styled, Window, div, img, px, relative,
};

use crate::library::Book;
use crate::ui::components::{stat_badge, with_alpha};
use crate::ui::theme::UiColors;

/// A grouped set of books sharing a parent directory or an explicit series name.
pub struct LibraryGroup<'a> {
    pub key: String,
    #[allow(dead_code)]
    pub name: String,
    pub first_seen: usize,
    pub books: Vec<&'a Book>,
}

/// Partitions a slice of books into series or directory-based folders.
pub fn directory_groups<'a>(books: impl IntoIterator<Item = &'a Book>) -> Vec<LibraryGroup<'a>> {
    let mut buckets: std::collections::BTreeMap<String, (String, usize, Vec<&'a Book>)> =
        std::collections::BTreeMap::new();
    for (index, book) in books.into_iter().enumerate() {
        let manual_series = book
            .series
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty());
        let (key, name) = if let Some(series) = manual_series {
            (
                format!("series:{}", series.to_lowercase()),
                series.to_string(),
            )
        } else {
            let parent = book.path.parent().unwrap_or_else(|| Path::new("."));
            let display = parent
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| parent.display().to_string());
            (format!("directory:{}", parent.display()), display)
        };
        buckets
            .entry(key)
            .and_modify(|(_, _, members)| members.push(book))
            .or_insert_with(|| (name, index, vec![book]));
    }

    let mut groups: Vec<_> = buckets
        .into_iter()
        .filter_map(|(key, (name, first_seen, members))| {
            if members.len() < 2 {
                return None;
            }
            Some(LibraryGroup {
                key,
                name,
                first_seen,
                books: members,
            })
        })
        .collect();
    groups.sort_by_key(|group| group.first_seen);
    groups
}

/// Properties needed to render an individual book grid card.
pub struct BookCardProps<'a> {
    pub book: &'a Book,
    pub cover_path: PathBuf,
    pub card_w: f32,
    pub cover_h: f32,
    pub radius: Pixels,
    pub border_px: f32,
    pub is_selected: bool,
    pub colors: UiColors,
}

/// Renders a single book grid card.
pub fn render_book_card(
    props: BookCardProps,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    on_right_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui_kit::App) + 'static,
) -> AnyElement {
    let book = props.book;
    let card_w = props.card_w;
    let cover_h = props.cover_h;
    let radius = props.radius;
    let border_px = props.border_px;
    let is_selected = props.is_selected;
    let colors = props.colors;
    let inner_radius = (radius - px(border_px)).max(px(0.));
    let progress = book.progress();
    let total = book.page_count;
    let current = if book.opened {
        book.current_page + 1
    } else {
        0
    };

    div()
        .id(ElementId::from(SharedString::from(format!(
            "manga-item-{}",
            book.id
        ))))
        .w(px(card_w))
        .flex_none()
        .flex()
        .flex_col()
        .rounded(radius)
        .bg(if is_selected {
            with_alpha(colors.accent, 0.15)
        } else {
            with_alpha(colors.surface, 0.6)
        })
        .when(border_px > 0.0 || is_selected, |s| {
            let w = if is_selected {
                border_px.max(1.5)
            } else {
                border_px
            };
            s.border(px(w)).border_color(if is_selected {
                colors.accent
            } else {
                colors.border
            })
        })
        .overflow_hidden()
        .cursor_pointer()
        .hover(|s| s.border_color(colors.accent).shadow_md())
        .on_mouse_down(MouseButton::Right, move |event, window, cx| {
            cx.stop_propagation();
            on_right_click(event, window, cx);
        })
        .on_click(on_click)
        .child(
            div()
                .id(ElementId::from(SharedString::from(format!(
                    "manga-cover-{}",
                    book.id
                ))))
                .relative()
                .w_full()
                .h(px(cover_h))
                .rounded_t(inner_radius)
                .bg(with_alpha(colors.foreground, 0.04))
                .overflow_hidden()
                .child(
                    img(props.cover_path)
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .rounded_t(inner_radius),
                )
                .children(is_selected.then(|| {
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(with_alpha(colors.accent, 0.22))
                })),
        )
        .child(
            div()
                .p_2p5()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.foreground)
                        .line_clamp(2)
                        .child(book.title.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .text_xs()
                        .text_color(colors.muted)
                        .child(if book.opened {
                            format!("Ch. {current} of {total} · {:.0}%", progress * 100.0)
                        } else {
                            format!("{total} pages")
                        }),
                )
                .when(book.opened, |this| {
                    this.child(
                        div()
                            .w_full()
                            .h(px(3.))
                            .rounded_full()
                            .bg(with_alpha(colors.foreground, 0.08))
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(progress))
                                    .rounded_full()
                                    .bg(colors.accent),
                            ),
                    )
                }),
        )
        .into_any_element()
}

/// Renders a single book list row.
pub fn render_book_list_row(
    book: &Book,
    cover_path: PathBuf,
    radius: Pixels,
    border_px: f32,
    is_selected: bool,
    colors: UiColors,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    on_right_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui_kit::App) + 'static,
) -> AnyElement {
    let progress = book.progress();
    let total = book.page_count;
    let current = if book.opened {
        book.current_page + 1
    } else {
        0
    };
    let display_title = book.title.clone();

    div()
        .id(ElementId::from(SharedString::from(format!(
            "manga-item-{}",
            book.id
        ))))
        .w_full()
        .h(px(64.))
        .px_3p5()
        .rounded(radius)
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .bg(if is_selected {
            with_alpha(colors.accent, 0.15)
        } else {
            with_alpha(colors.surface, 0.5)
        })
        .when(border_px > 0.0 || is_selected, |s| {
            let w = if is_selected {
                border_px.max(1.5)
            } else {
                border_px
            };
            s.border(px(w)).border_color(if is_selected {
                colors.accent
            } else {
                colors.border
            })
        })
        .cursor_pointer()
        .hover(|s| {
            s.bg(if is_selected {
                with_alpha(colors.accent, 0.22)
            } else {
                with_alpha(colors.surface, 0.8)
            })
            .border_color(colors.accent)
        })
        .on_mouse_down(MouseButton::Right, move |event, window, cx| {
            cx.stop_propagation();
            on_right_click(event, window, cx);
        })
        .on_click(on_click)
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .min_w_0()
                .flex_1()
                .children(is_selected.then(|| {
                    div()
                        .size(px(18.))
                        .rounded(px(4.))
                        .bg(colors.accent)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::Check)
                                .size(px(12.))
                                .text_color(colors.accent_foreground),
                        )
                }))
                .child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "manga-cover-{}",
                            book.id
                        ))))
                        .w(px(36.))
                        .h(px(50.))
                        .flex_none()
                        .rounded(px(f32::from(radius).min(4.)))
                        .bg(with_alpha(colors.foreground, 0.04))
                        .overflow_hidden()
                        .child(
                            img(cover_path)
                                .size_full()
                                .rounded(px(f32::from(radius).min(4.)))
                                .object_fit(ObjectFit::Cover),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.foreground)
                                .truncate()
                                .child(display_title),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_xs()
                                .text_color(colors.muted)
                                .children(book.series.as_deref().map(|s| {
                                    div()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded(px(3.))
                                        .bg(with_alpha(colors.foreground, 0.06))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(colors.foreground)
                                        .truncate()
                                        .child(s.to_string())
                                }))
                                .child(div().truncate().child(if book.opened {
                                    format!("Ch. {current} of {total} · {:.0}%", progress * 100.0)
                                } else {
                                    format!("{total} pages")
                                })),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .flex_none()
                .children(book.opened.then(|| {
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .w(px(90.))
                                .h(px(4.))
                                .rounded_full()
                                .bg(with_alpha(colors.foreground, 0.08))
                                .child(
                                    div()
                                        .h_full()
                                        .w(relative(progress))
                                        .rounded_full()
                                        .bg(colors.accent),
                                ),
                        )
                        .child(
                            div()
                                .w(px(34.))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(colors.muted)
                                .child(format!("{:.0}%", progress * 100.0)),
                        )
                }))
                .child(if progress >= 1.0 {
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_full()
                        .bg(with_alpha(colors.accent, 0.15))
                        .text_color(colors.accent)
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Completed")
                } else if book.opened {
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_full()
                        .bg(with_alpha(colors.foreground, 0.08))
                        .text_color(colors.foreground)
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Reading")
                } else {
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_full()
                        .bg(with_alpha(colors.foreground, 0.04))
                        .text_color(colors.muted)
                        .text_xs()
                        .child("Unread")
                }),
        )
        .into_any_element()
}

/// Renders a series or directory folder grid card.
pub fn render_folder_card(
    group_key: &str,
    group_name: &str,
    count: usize,
    cover_path: PathBuf,
    card_w: f32,
    cover_h: f32,
    radius: Pixels,
    border_px: f32,
    colors: UiColors,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    on_right_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui_kit::App) + 'static,
) -> AnyElement {
    let inner_radius = (radius - px(border_px)).max(px(0.));

    div()
        .id(SharedString::from(format!("folder-{}", group_key)))
        .w(px(card_w))
        .flex_none()
        .flex()
        .flex_col()
        .rounded(radius)
        .bg(with_alpha(colors.surface, 0.6))
        .when(border_px > 0.0, |s| {
            s.border(px(border_px)).border_color(colors.border)
        })
        .overflow_hidden()
        .cursor_pointer()
        .hover(|s| s.border_color(colors.accent).shadow_md())
        .on_mouse_down(MouseButton::Right, move |event, window, cx| {
            cx.stop_propagation();
            on_right_click(event, window, cx);
        })
        .on_click(on_click)
        .child(
            div()
                .relative()
                .w_full()
                .h(px(cover_h))
                .rounded_t(inner_radius)
                .bg(with_alpha(colors.foreground, 0.04))
                .overflow_hidden()
                .child(
                    img(cover_path)
                        .size_full()
                        .rounded_t(inner_radius)
                        .object_fit(ObjectFit::Cover),
                )
                .child(
                    div()
                        .absolute()
                        .top_2()
                        .right_2()
                        .child(stat_badge(count, false, colors, radius)),
                ),
        )
        .child(
            div()
                .p_2p5()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.foreground)
                        .truncate()
                        .child(group_name.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.muted)
                        .truncate()
                        .child(format!("{count} volumes · Series")),
                ),
        )
        .into_any_element()
}

/// Renders a series or directory folder list row.
pub fn render_folder_list_row(
    group_key: &str,
    group_name: &str,
    count: usize,
    cover_path: PathBuf,
    radius: Pixels,
    border_px: f32,
    colors: UiColors,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    on_right_click: impl Fn(&MouseDownEvent, &mut Window, &mut gpui_kit::App) + 'static,
) -> AnyElement {
    div()
        .id(SharedString::from(format!("folder-{}", group_key)))
        .w_full()
        .h(px(64.))
        .px_3p5()
        .rounded(radius)
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .bg(with_alpha(colors.surface, 0.5))
        .when(border_px > 0.0, |s| {
            s.border(px(border_px)).border_color(colors.border)
        })
        .cursor_pointer()
        .hover(|s| {
            s.bg(with_alpha(colors.surface, 0.8))
                .border_color(colors.accent)
        })
        .on_mouse_down(MouseButton::Right, move |event, window, cx| {
            cx.stop_propagation();
            on_right_click(event, window, cx);
        })
        .on_click(on_click)
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .w(px(36.))
                        .h(px(50.))
                        .flex_none()
                        .rounded(px(f32::from(radius).min(4.)))
                        .bg(with_alpha(colors.foreground, 0.04))
                        .overflow_hidden()
                        .child(
                            img(cover_path)
                                .size_full()
                                .rounded(px(f32::from(radius).min(4.)))
                                .object_fit(ObjectFit::Cover),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    Icon::new(IconName::Folder)
                                        .size(px(14.))
                                        .text_color(colors.accent),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(colors.foreground)
                                        .truncate()
                                        .child(group_name.to_string()),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.muted)
                                .child(format!("{count} volumes · Series folder")),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2p5()
                .flex_none()
                .child(stat_badge(count, false, colors, radius))
                .child(
                    Icon::new(IconName::ChevronRight)
                        .size(px(16.))
                        .text_color(colors.muted),
                ),
        )
        .into_any_element()
}
