//! Reading history view showing in-progress and completed books.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ElementId, FontWeight, InteractiveElement, Pixels, ScrollHandle, SharedString,
    Styled, Window, div, img, px, relative,
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
        return div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .text_color(colors.muted)
            .child(Icon::new(IconName::Clock).size(px(48.)))
            .child(div().text_base().child("No reading history yet"))
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
                            )),
                    )
                    .into_any_element()
            } else {
                let on_select = on_select.clone();
                let book_id_click = book_id.clone();
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "hist-item-{}",
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
                    .bg(with_alpha(colors.surface, 0.5))
                    .when(border_px > 0.0, |s| {
                        s.border(px(border_px)).border_color(colors.border)
                    })
                    .cursor_pointer()
                    .hover(|s| {
                        s.bg(with_alpha(colors.surface, 0.8))
                            .border_color(colors.accent)
                    })
                    .on_click(move |_, window, cx| on_select(book_id_click.clone(), window, cx))
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
                                        div().text_xs().text_color(colors.muted).truncate().child(
                                            format!(
                                                "Ch. {current} of {total} · {:.0}%",
                                                progress * 100.0
                                            ),
                                        ),
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
                                    ),
                            )
                            .child(
                                Icon::new(IconName::ChevronRight)
                                    .size(px(16.))
                                    .text_color(colors.muted),
                            ),
                    )
                    .into_any_element()
            }
        })
        .collect();

    div()
        .id("history-container")
        .track_scroll(props.scroll_handle)
        .size_full()
        .overflow_y_scroll()
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
