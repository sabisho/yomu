//! Library, downloads, book cards, and folder items view presentation adapters.

use std::collections::HashSet;
use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Entity, MouseDownEvent, Pixels, Window, div, px,
};

use crate::app::state::Yomu;
use crate::ui;
use crate::ui::library_view::{
    BookCardProps, directory_groups, render_book_card, render_book_list_row, render_folder_card,
    render_folder_list_row,
};
use crate::ui::theme::UiColors;
use yomu::library::Book;

impl Yomu {
    pub(crate) fn render_book_card(
        &self,
        book: &Book,
        card_w: f32,
        cover_h: f32,
        radius: Pixels,
        colors: UiColors,
        yomu: &Entity<Self>,
    ) -> AnyElement {
        let book_id = book.id.clone();
        let thumb_size = self.library.settings().thumbnail_size;
        let cover_path = self.dirs.cover_for_size(&book.id, thumb_size);
        let is_selected = self.selection.contains(&book.id);
        let border_px = self.border_px();

        let yomu_click = yomu.clone();
        let id_click = book_id.clone();
        let on_click = move |event: &ClickEvent, window: &mut Window, cx: &mut App| {
            let ctrl = event.modifiers().control || event.modifiers().platform;
            let shift = event.modifiers().shift;
            yomu_click.update(cx, |this, cx| {
                if this.select_click(&id_click, ctrl, shift, cx) {
                    return;
                }
                this.open_book(&id_click, window, cx);
            });
        };

        let yomu_right = yomu.clone();
        let id_right = book_id;
        let on_right_click = move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            yomu_right.update(cx, |this, cx| {
                this.open_context_menu(&id_right, event.position, window, cx);
            });
        };

        render_book_card(
            BookCardProps {
                book,
                cover_path,
                card_w,
                cover_h,
                radius,
                border_px,
                is_selected,
                colors,
            },
            on_click,
            on_right_click,
        )
    }

    pub(crate) fn render_book_list_row(
        &self,
        book: &Book,
        radius: Pixels,
        colors: UiColors,
        yomu: &Entity<Self>,
    ) -> AnyElement {
        let book_id = book.id.clone();
        let thumb_size = self.library.settings().thumbnail_size;
        let cover_path = self.dirs.cover_for_size(&book.id, thumb_size);
        let is_selected = self.selection.contains(&book.id);
        let border_px = self.border_px();

        let yomu_click = yomu.clone();
        let id_click = book_id.clone();
        let on_click = move |event: &ClickEvent, window: &mut Window, cx: &mut App| {
            let ctrl = event.modifiers().control || event.modifiers().platform;
            let shift = event.modifiers().shift;
            yomu_click.update(cx, |this, cx| {
                if this.select_click(&id_click, ctrl, shift, cx) {
                    return;
                }
                this.open_book(&id_click, window, cx);
            });
        };

        let yomu_right = yomu.clone();
        let id_right = book_id;
        let on_right_click = move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
            yomu_right.update(cx, |this, cx| {
                this.open_context_menu(&id_right, event.position, window, cx);
            });
        };

        render_book_list_row(
            book,
            cover_path,
            radius,
            border_px,
            is_selected,
            colors,
            on_click,
            on_right_click,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_folder_list_row(
        &self,
        group_key: &str,
        group_name: &str,
        book_ids: &[String],
        count: usize,
        cover_path: PathBuf,
        radius: Pixels,
        colors: UiColors,
        border_px: f32,
        yomu: &Entity<Self>,
    ) -> AnyElement {
        let yomu_click = yomu.clone();
        let group_key_click = group_key.to_string();
        let on_click = move |_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
            let key = group_key_click.clone();
            yomu_click.update(cx, |this, cx| {
                this.enter_group(key, cx);
            });
        };

        let yomu_right = yomu.clone();
        let group_key_right = group_key.to_string();
        let group_name_right = group_name.to_string();
        let book_ids_right = book_ids.to_vec();
        let on_right_click = move |event: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
            let key = group_key_right.clone();
            let name = group_name_right.clone();
            let ids = book_ids_right.clone();
            yomu_right.update(cx, |this, cx| {
                this.open_group_context_menu(&key, &name, &ids, event.position, cx);
            });
        };

        render_folder_list_row(
            group_key,
            group_name,
            count,
            cover_path,
            radius,
            border_px,
            colors,
            on_click,
            on_right_click,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_folder_card(
        &self,
        group_key: &str,
        group_name: &str,
        book_ids: &[String],
        count: usize,
        cover_path: PathBuf,
        card_w: f32,
        cover_h: f32,
        radius: Pixels,
        colors: UiColors,
        yomu: &Entity<Self>,
    ) -> AnyElement {
        let border_px = self.border_px();
        let yomu_click = yomu.clone();
        let group_key_click = group_key.to_string();
        let on_click = move |_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
            let key = group_key_click.clone();
            yomu_click.update(cx, |this, cx| {
                this.enter_group(key, cx);
            });
        };

        let yomu_right = yomu.clone();
        let group_key_right = group_key.to_string();
        let group_name_right = group_name.to_string();
        let book_ids_right = book_ids.to_vec();
        let on_right_click = move |event: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
            let key = group_key_right.clone();
            let name = group_name_right.clone();
            let ids = book_ids_right.clone();
            yomu_right.update(cx, |this, cx| {
                this.open_group_context_menu(&key, &name, &ids, event.position, cx);
            });
        };

        render_folder_card(
            group_key,
            group_name,
            count,
            cover_path,
            card_w,
            cover_h,
            radius,
            border_px,
            colors,
            on_click,
            on_right_click,
        )
    }

    pub(crate) fn render_all_manga_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let radius = px(self.radius_px());
        let yomu = cx.entity();

        let order = self.current_view_order();
        let query = self.search_query.trim().to_lowercase();
        let books: Vec<&Book> = order
            .iter()
            .filter_map(|id| self.library.get(id))
            .filter(|b| {
                if query.is_empty() {
                    true
                } else {
                    b.title.to_lowercase().contains(&query)
                        || b.series
                            .as_ref()
                            .is_some_and(|s| s.to_lowercase().contains(&query))
                }
            })
            .collect();

        if books.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(colors.muted)
                .child(Icon::new(IconName::BookOpen).size(px(48.)))
                .child(div().text_base().child(if query.is_empty() {
                    "Your library is empty"
                } else {
                    "No matching manga found"
                }))
                .into_any_element();
        }

        let thumb_size = self.library.settings().thumbnail_size;
        let sidebar_collapsed = self.is_sidebar_collapsed(window);
        let grid = ui::components::compute_responsive_grid(
            f32::from(window.viewport_size().width),
            !sidebar_collapsed,
            thumb_size,
        );
        let card_w = grid.card_w;
        let cover_h = grid.cover_h;

        let border_px = self.border_px();
        let is_grid = self.view_mode_grid;
        let mut cards: Vec<AnyElement> = Vec::new();

        if self.library.settings().group_series && self.open_group.is_none() {
            let groups = directory_groups(books.iter().copied());
            let mut grouped_ids = HashSet::new();

            for group in &groups {
                for b in &group.books {
                    grouped_ids.insert(b.id.clone());
                }
                let first = group.books[0];
                let group_key = group.key.clone();
                let group_name = group.name.clone();
                let count = group.books.len();
                let cover_path = self.dirs.cover_for_size(&first.id, thumb_size);

                if is_grid {
                    let book_ids: Vec<String> = group.books.iter().map(|b| b.id.clone()).collect();
                    cards.push(self.render_folder_card(
                        &group_key,
                        &group_name,
                        &book_ids,
                        count,
                        cover_path,
                        card_w,
                        cover_h,
                        radius,
                        colors,
                        &yomu,
                    ));
                } else {
                    let book_ids: Vec<String> = group.books.iter().map(|b| b.id.clone()).collect();
                    cards.push(self.render_folder_list_row(
                        &group_key,
                        &group_name,
                        &book_ids,
                        count,
                        cover_path,
                        radius,
                        colors,
                        border_px,
                        &yomu,
                    ));
                }
            }

            for book in books.iter().filter(|b| !grouped_ids.contains(&b.id)) {
                if is_grid {
                    cards.push(self.render_book_card(book, card_w, cover_h, radius, colors, &yomu));
                } else {
                    cards.push(self.render_book_list_row(book, radius, colors, &yomu));
                }
            }
        } else {
            for book in &books {
                if is_grid {
                    cards.push(self.render_book_card(book, card_w, cover_h, radius, colors, &yomu));
                } else {
                    cards.push(self.render_book_list_row(book, radius, colors, &yomu));
                }
            }
        }

        let in_group = self.open_group.is_some();
        let scroll_handle = if in_group {
            &self.group_scroll
        } else {
            &self.library_scroll
        };

        div()
            .id(if in_group {
                "group-grid"
            } else {
                "all-manga-container"
            })
            .size_full()
            .overflow_y_scrollbar()
            .track_scroll(scroll_handle)
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

    pub(crate) fn render_downloads_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_all_manga_view(window, cx)
    }
}
