//! Reading history view showing in-progress and completed books.

use gpui_kit::assets::IconName;
use gpui_kit::component::empty::{
    Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ElementId, FontWeight, InteractiveElement, Pixels, ScrollHandle, SharedString,
    Styled, Window, div, img, px,
};
use yomu::library::{Book, DataDirs, ThumbnailSize};

use super::components::{compute_responsive_grid, with_alpha};
use super::theme::UiColors;

pub struct HistoryProps<'a> {
    pub scroll_handle: &'a ScrollHandle,
    pub history_books: &'a [&'a Book],
    pub dirs: &'a DataDirs,
    pub colors: UiColors,
    pub radius: Pixels,
    pub border_px: f32,
    pub radius_px: f32,
    pub viewport_w: f32,
    pub sidebar_visible: bool,
    pub is_grid: bool,
    pub thumb_size: ThumbnailSize,
}

pub fn render_history_view(
    props: HistoryProps<'_>,
    on_select: impl Fn(String, &mut Window, &mut App) + 'static + Clone,
) -> AnyElement {
    let colors = props.colors;
    let radius = props.radius;
    let border_px = props.border_px;
    let is_grid = props.is_grid;

    if props.history_books.is_empty() {
        return Empty::new()
            .header(
                EmptyHeader::new()
                    .media(
                        EmptyMedia::new()
                            .with_variant(EmptyMediaVariant::Icon)
                            .child(Icon::new(IconName::Clock).size(px(48.))),
                    )
                    .title(EmptyTitle::new().child("No reading history yet"))
                    .description(EmptyDescription::new().child(
                        "Books you open will appear here so you can pick up where you left off.",
                    )),
            )
            .into_any_element();
    }

    let grid = compute_responsive_grid(props.viewport_w, props.sidebar_visible, props.thumb_size);
    let card_w = grid.card_w;
    let cover_h = grid.cover_h;

    let cards: Vec<AnyElement> = props
        .history_books
        .iter()
        .map(|book| {
            let book_id = book.id.clone();
            let cover_path = props.dirs.cover_for_size(&book.id, props.thumb_size);
            let display_title = book.title.clone();
            let total = book.page_count;
            let current = if book.opened {
                book.current_page + 1
            } else {
                0
            };
            let progress = book.progress();

            if is_grid {
                let on_select = on_select.clone();
                let book_id_click = book_id.clone();
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "hist-item-{}",
                        book.id
                    ))))
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
                    .on_click(move |_, window, cx| on_select(book_id_click.clone(), window, cx))
                    .child(
                        div()
                            .id(ElementId::from(SharedString::from(format!(
                                "hist-cover-{}",
                                book.id
                            ))))
                            .relative()
                            .w_full()
                            .h(px(cover_h))
                            .rounded_t(radius)
                            .bg(with_alpha(colors.foreground, 0.04))
                            .overflow_hidden()
                            .child(
                                img(cover_path)
                                    .size_full()
                                    .rounded_t(radius)
                                    .object_fit(gpui_kit::ObjectFit::Cover),
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
                                    .child(display_title),
                            )
                            .child(div().text_xs().text_color(colors.muted).truncate().child(
                                format!("Ch. {current} of {total} · {:.0}%", progress * 100.0),
                            ))
                            .child(
                                div().w_full().pt_1().child(
                                    Progress::new(ElementId::from(SharedString::from(format!(
                                        "hist-prog-{}",
                                        book.id
                                    ))))
                                    .value(progress * 100.0)
                                    .h(px(3.))
                                    .rounded(radius),
                                ),
                            ),
                    )
                    .into_any_element()
            } else {
                let on_select = on_select.clone();
                let book_id_click = book_id.clone();
                ListItem::new(ElementId::from(SharedString::from(format!(
                    "hist-item-{}",
                    book.id
                ))))
                .h(px(64.))
                .px_3p5()
                .rounded(radius)
                .when(border_px > 0.0, |s| {
                    s.border(px(border_px)).border_color(colors.border)
                })
                .on_click(move |_, window, cx| on_select(book_id_click.clone(), window, cx))
                .child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .min_w_0()
                                .flex_1()
                                .child(
                                    div()
                                        .id(ElementId::from(SharedString::from(format!(
                                            "hist-cover-{}",
                                            book.id
                                        ))))
                                        .w(px(36.))
                                        .h(px(50.))
                                        .flex_none()
                                        .rounded(px(props.radius_px.min(4.)))
                                        .bg(with_alpha(colors.foreground, 0.04))
                                        .overflow_hidden()
                                        .child(
                                            img(cover_path)
                                                .size_full()
                                                .rounded(px(props.radius_px.min(4.)))
                                                .object_fit(gpui_kit::ObjectFit::Cover),
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
                                                    Tag::secondary().small().child(s.to_string())
                                                }))
                                                .child(div().truncate().child(format!(
                                                    "Ch. {current} of {total} · {:.0}%",
                                                    progress * 100.0
                                                ))),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .flex_none()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            Progress::new(ElementId::from(SharedString::from(
                                                format!("hist-prog-list-{}", book.id),
                                            )))
                                            .value(progress * 100.0)
                                            .w(px(90.)),
                                        )
                                        .child(
                                            div()
                                                .w(px(34.))
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(colors.muted)
                                                .child(format!("{:.0}%", progress * 100.0)),
                                        ),
                                )
                                .child(if progress >= 1.0 {
                                    Tag::success().small().child("Completed")
                                } else {
                                    Tag::primary().small().child("Reading")
                                })
                                .child(
                                    Icon::new(IconName::ChevronRight)
                                        .size(px(16.))
                                        .text_color(colors.muted),
                                ),
                        ),
                )
                .into_any_element()
            }
        })
        .collect();

    div()
        .id("history-container")
        .size_full()
        .overflow_y_scrollbar()
        .track_scroll(props.scroll_handle)
        .scroll_bounce(true)
        .scroll_wheel(false)
        .flex()
        .flex_col()
        .child(if is_grid {
            div()
                .px(px(grid.pad_x))
                .py_6()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(grid.gap))
                .children(cards)
        } else {
            div()
                .w_full()
                .max_w(px(1040.))
                .mx_auto()
                .px(px(grid.pad_x.max(16.0)))
                .py_4()
                .flex()
                .flex_col()
                .gap_2()
                .children(cards)
        })
        .into_any_element()
}
