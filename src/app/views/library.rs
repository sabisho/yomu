//! Library, downloads, book cards, and folder items view presentation adapters.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::empty::{
    Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable, Size};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, ClickEvent, Context, Entity, Pixels, Window, div, px};

use crate::app::state::Yomu;
use crate::ui;
use crate::ui::library_view::{
    BookCardProps, directory_groups, render_book_card, render_book_list_row, render_folder_card,
    render_folder_list_row, sort_groups,
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
        let book_id: Arc<str> = Arc::from(book.id.as_str());
        let thumb_size = self.library.settings().thumbnail_size;
        let cover_path = self.dirs.cover_for_size(&book.id, thumb_size);
        let is_selected = self.selection.contains(&book.id);
        let border_px = self.border_px();

        let yomu_click = yomu.clone();
        let id_click = Arc::clone(&book_id);
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

        let yomu_menu = yomu.clone();
        let id_menu = Arc::clone(&book_id);
        let context_menu =
            move |menu: PopupMenu, _window: &mut Window, cx: &mut Context<PopupMenu>| {
                let id = Arc::clone(&id_menu);
                let yomu = yomu_menu.clone();
                yomu_menu.update(cx, |this, _cx| {
                    this.build_book_context_menu(&id, menu, yomu)
                })
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
            context_menu,
        )
    }

    pub(crate) fn render_book_list_row(
        &self,
        book: &Book,
        radius: Pixels,
        colors: UiColors,
        yomu: &Entity<Self>,
    ) -> AnyElement {
        let book_id: Arc<str> = Arc::from(book.id.as_str());
        let thumb_size = self.library.settings().thumbnail_size;
        let cover_path = self.dirs.cover_for_size(&book.id, thumb_size);
        let is_selected = self.selection.contains(&book.id);
        let border_px = self.border_px();

        let yomu_click = yomu.clone();
        let id_click = Arc::clone(&book_id);
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

        let yomu_menu = yomu.clone();
        let id_menu = book_id;
        let context_menu =
            move |menu: PopupMenu, _window: &mut Window, cx: &mut Context<PopupMenu>| {
                let id = Arc::clone(&id_menu);
                let yomu = yomu_menu.clone();
                yomu_menu.update(cx, |this, _cx| {
                    this.build_book_context_menu(&id, menu, yomu)
                })
            };

        render_book_list_row(
            book,
            cover_path,
            radius,
            border_px,
            is_selected,
            colors,
            on_click,
            context_menu,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_folder_list_row(
        &self,
        group_key: &str,
        group_name: &str,
        count: usize,
        cover_path: PathBuf,
        radius: Pixels,
        colors: UiColors,
        border_px: f32,
        yomu: &Entity<Self>,
    ) -> AnyElement {
        let key_arc: Arc<str> = Arc::from(group_key);
        let name_arc: Arc<str> = Arc::from(group_name);

        let yomu_click = yomu.clone();
        let key_click = Arc::clone(&key_arc);
        let on_click = move |_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
            let key = key_click.to_string();
            yomu_click.update(cx, |this, cx| {
                this.enter_group(key, cx);
            });
        };

        let yomu_menu = yomu.clone();
        let key_menu = Arc::clone(&key_arc);
        let name_menu = Arc::clone(&name_arc);
        let context_menu =
            move |menu: PopupMenu, _window: &mut Window, cx: &mut Context<PopupMenu>| {
                let key = Arc::clone(&key_menu);
                let name = Arc::clone(&name_menu);
                let yomu = yomu_menu.clone();
                yomu_menu.update(cx, |this, _cx| {
                    this.build_group_context_menu(&key, &name, menu, yomu)
                })
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
            context_menu,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_folder_card(
        &self,
        group_key: &str,
        group_name: &str,
        count: usize,
        cover_path: PathBuf,
        card_w: f32,
        cover_h: f32,
        radius: Pixels,
        colors: UiColors,
        yomu: &Entity<Self>,
    ) -> AnyElement {
        let border_px = self.border_px();
        let key_arc: Arc<str> = Arc::from(group_key);
        let name_arc: Arc<str> = Arc::from(group_name);

        let yomu_click = yomu.clone();
        let key_click = Arc::clone(&key_arc);
        let on_click = move |_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
            let key = key_click.to_string();
            yomu_click.update(cx, |this, cx| {
                this.enter_group(key, cx);
            });
        };

        let yomu_menu = yomu.clone();
        let key_menu = key_arc;
        let name_menu = name_arc;
        let context_menu =
            move |menu: PopupMenu, _window: &mut Window, cx: &mut Context<PopupMenu>| {
                let key = Arc::clone(&key_menu);
                let name = Arc::clone(&name_menu);
                let yomu = yomu_menu.clone();
                yomu_menu.update(cx, |this, _cx| {
                    this.build_group_context_menu(&key, &name, menu, yomu)
                })
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
            context_menu,
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

        let query = self.search_query.trim().to_lowercase();
        let matches_query = |b: &Book| {
            query.is_empty()
                || b.title.to_lowercase().contains(&query)
                || b.series
                    .as_ref()
                    .is_some_and(|s| s.to_lowercase().contains(&query))
        };

        let sorted_books = self.get_sorted_books();

        let books: Vec<&Book> = if self.library.settings().group_series
            && let Some(open_key) = self.open_group.as_deref()
        {
            let groups = directory_groups(sorted_books);
            if let Some(group) = groups.into_iter().find(|g| g.key == open_key) {
                group
                    .books
                    .into_iter()
                    .filter(|b| matches_query(b))
                    .collect()
            } else {
                self.get_sorted_books()
                    .into_iter()
                    .filter(|b| matches_query(b))
                    .collect()
            }
        } else {
            sorted_books
                .into_iter()
                .filter(|b| matches_query(b))
                .collect()
        };

        if books.is_empty() {
            let (title, desc) = if query.is_empty() {
                (
                    "Your Library is Empty",
                    "Import comic archives or manga folders to start reading.",
                )
            } else {
                (
                    "No matching manga found",
                    "Try adjusting your search query.",
                )
            };
            return Empty::new()
                .header(
                    EmptyHeader::new()
                        .media(
                            EmptyMedia::new()
                                .with_variant(EmptyMediaVariant::Icon)
                                .child(
                                    Icon::new(if query.is_empty() {
                                        IconName::BookOpen
                                    } else {
                                        IconName::Search
                                    })
                                    .size(px(48.))
                                    .text_color(colors.muted),
                                ),
                        )
                        .title(EmptyTitle::new().child(title))
                        .description(EmptyDescription::new().child(desc)),
                )
                .when(query.is_empty(), |this| {
                    let yomu = yomu.clone();
                    this.child(
                        Button::new("empty-import-btn")
                            .primary()
                            .icon(IconName::Plus)
                            .label("Import Manga")
                            .with_size(Size::Size(ui::components::CONTROL_HEIGHT_PRIMARY))
                            .rounded(ButtonRounded::Size(ui::components::button_radius(
                                radius,
                                ui::components::CONTROL_HEIGHT_PRIMARY,
                            )))
                            .on_click(move |_, _, cx| {
                                yomu.update(cx, |this, cx| {
                                    this.import_dialog(false, cx);
                                });
                            }),
                    )
                })
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
            let mut groups = directory_groups(books.iter().copied());
            let sort_by = self.library.settings().sort_by;
            let sort_direction = self.library.settings().sort_direction;
            sort_groups(&mut groups, sort_by, sort_direction);
            let mut grouped_ids: HashSet<&str> = HashSet::new();

            for group in &groups {
                for b in &group.books {
                    grouped_ids.insert(b.id.as_str());
                }
                let first = group.books[0];
                let count = group.books.len();
                let cover_path = self.dirs.cover_for_size(&first.id, thumb_size);

                if is_grid {
                    cards.push(self.render_folder_card(
                        &group.key,
                        &group.name,
                        count,
                        cover_path,
                        card_w,
                        cover_h,
                        radius,
                        colors,
                        &yomu,
                    ));
                } else {
                    cards.push(self.render_folder_list_row(
                        &group.key,
                        &group.name,
                        count,
                        cover_path,
                        radius,
                        colors,
                        border_px,
                        &yomu,
                    ));
                }
            }

            for book in books
                .iter()
                .filter(|b| !grouped_ids.contains(b.id.as_str()))
            {
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
                    .items_start()
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
}
