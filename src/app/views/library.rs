//! Library, downloads, book cards, and folder items view presentation adapters.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::ScrollBounce;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::empty::{
    Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::v_virtual_list;
use gpui_kit::component::{Icon, Sizable, Size, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, ClickEvent, Context, Entity, Pixels, Window, div, px, size};

use crate::app::state::Yomu;
use crate::ui;
use crate::ui::library_view::{
    BookCardProps, LibraryItem, directory_groups, render_book_card, render_book_list_row,
    render_folder_card, render_folder_list_row, sort_groups,
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
        selection_state: ui::library_view::GroupSelectionState,
        member_ids: Arc<[String]>,
        yomu: &Entity<Self>,
    ) -> AnyElement {
        let key_arc: Arc<str> = Arc::from(group_key);
        let name_arc: Arc<str> = Arc::from(group_name);

        let yomu_click = yomu.clone();
        let key_click = Arc::clone(&key_arc);
        let member_ids_click = Arc::clone(&member_ids);
        let on_click = move |event: &ClickEvent, _window: &mut Window, cx: &mut App| {
            let ctrl = event.modifiers().control || event.modifiers().platform;
            yomu_click.update(cx, |this, cx| {
                if !this.selection.is_empty() || ctrl {
                    this.toggle_group_selection(&member_ids_click, cx);
                    return;
                }
                let key = key_click.to_string();
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
            selection_state,
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
        selection_state: ui::library_view::GroupSelectionState,
        member_ids: Arc<[String]>,
        colors: UiColors,
        yomu: &Entity<Self>,
    ) -> AnyElement {
        let border_px = self.border_px();
        let key_arc: Arc<str> = Arc::from(group_key);
        let name_arc: Arc<str> = Arc::from(group_name);

        let yomu_click = yomu.clone();
        let key_click = Arc::clone(&key_arc);
        let member_ids_click = Arc::clone(&member_ids);
        let on_click = move |event: &ClickEvent, _window: &mut Window, cx: &mut App| {
            let ctrl = event.modifiers().control || event.modifiers().platform;
            yomu_click.update(cx, |this, cx| {
                if !this.selection.is_empty() || ctrl {
                    this.toggle_group_selection(&member_ids_click, cx);
                    return;
                }
                let key = key_click.to_string();
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
            selection_state,
            colors,
            on_click,
            context_menu,
        )
    }

    fn render_empty_library_state(
        &self,
        radius: Pixels,
        colors: UiColors,
        yomu: &Entity<Self>,
        is_empty_library: bool,
    ) -> AnyElement {
        let (title, desc) = if is_empty_library {
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
        Empty::new()
            .header(
                EmptyHeader::new()
                    .media(
                        EmptyMedia::new()
                            .with_variant(EmptyMediaVariant::Icon)
                            .child(
                                Icon::new(if is_empty_library {
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
            .when(is_empty_library, |this| {
                let yomu1 = yomu.clone();
                let yomu2 = yomu.clone();
                let btn_r =
                    ui::components::button_radius(radius, ui::components::CONTROL_HEIGHT_PRIMARY);
                this.child(
                    h_flex()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .child(
                            div()
                                .debug_selector(|| "empty-import-files-btn".into())
                                .child(
                                    Button::new("empty-import-files-btn")
                                        .primary()
                                        .icon(IconName::FilePlus)
                                        .label("Import Files")
                                        .accessibility_label("Import manga files")
                                        .with_size(Size::Size(
                                            ui::components::CONTROL_HEIGHT_PRIMARY,
                                        ))
                                        .rounded(ButtonRounded::Size(btn_r))
                                        .on_click(move |_, _, cx| {
                                            yomu1.update(cx, |this, cx| {
                                                this.import_dialog(false, cx);
                                            });
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .debug_selector(|| "empty-import-folder-btn".into())
                                .child(
                                    Button::new("empty-import-folder-btn")
                                        .outline()
                                        .icon(IconName::FolderPlus)
                                        .label("Import Folder")
                                        .accessibility_label("Import manga folder")
                                        .with_size(Size::Size(
                                            ui::components::CONTROL_HEIGHT_PRIMARY,
                                        ))
                                        .rounded(ButtonRounded::Size(btn_r))
                                        .on_click(move |_, _, cx| {
                                            yomu2.update(cx, |this, cx| {
                                                this.import_dialog(true, cx);
                                            });
                                        }),
                                ),
                        ),
                )
            })
            .into_any_element()
    }

    pub(crate) fn render_all_manga_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let radius = px(self.radius_px());
        let yomu = cx.entity();

        let query = self.search_query.trim();

        // If the library is completely empty and no search query is active, show empty state immediately
        if self.library.books().is_empty() && query.is_empty() {
            return self.render_empty_library_state(radius, colors, &yomu, true);
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
        let cols = if is_grid { grid.cols } else { 1 };
        let gap = if is_grid { grid.gap } else { 8.0 };
        let pad_x = if is_grid {
            grid.pad_x
        } else {
            grid.pad_x.max(16.0)
        };
        let row_height = if is_grid { cover_h + 64.0 } else { 64.0 };
        let row_slot_h = row_height + gap;
        let content_w = (f32::from(window.viewport_size().width)
            - if !sidebar_collapsed { 240.0 } else { 48.0 })
        .max(200.0);

        let mutation_seq = self.library.mutation_seq();
        let save_seq = self.save_seq;
        let books_len = self.library.books().len();
        let sort_by = self.library.settings().sort_by;
        let sort_direction = self.library.settings().sort_direction;
        let group_series = self.library.settings().group_series;
        let open_group_str = self.open_group.as_deref();

        let mut cache = self.library_row_cache.borrow_mut();
        let is_cached = cache.is_valid(
            mutation_seq,
            save_seq,
            books_len,
            query,
            sort_by,
            sort_direction,
            group_series,
            open_group_str,
            is_grid,
            cols,
            card_w,
            cover_h,
            gap,
            pad_x,
        );

        if !is_cached {
            let has_query = !query.is_empty();
            let matches_query = |b: &Book| {
                if !has_query {
                    return true;
                }
                yomu::search::matches(&b.title, &b.path, b.series.as_deref(), query)
            };

            let sorted_books = self.get_sorted_books();

            let books: Vec<&Book> = if group_series && let Some(open_key) = open_group_str {
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
                *cache = ui::library_view::LibraryRowCache {
                    mutation_seq,
                    save_seq,
                    books_len,
                    search_query: query.to_string(),
                    sort_by,
                    sort_direction,
                    group_series,
                    open_group: self.open_group.clone(),
                    view_mode_grid: is_grid,
                    cols,
                    card_w,
                    cover_h,
                    gap,
                    pad_x,
                    rows: Vec::new(),
                    item_sizes: std::rc::Rc::new(Vec::new()),
                };
                return self.render_empty_library_state(radius, colors, &yomu, query.is_empty());
            }

            let mut items: Vec<LibraryItem> = Vec::new();
            if group_series && self.open_group.is_none() {
                let mut groups = directory_groups(books.iter().copied());
                sort_groups(&mut groups, sort_by, sort_direction);
                let mut grouped_ids: HashSet<&str> = HashSet::new();

                for group in &groups {
                    for b in &group.books {
                        grouped_ids.insert(b.id.as_str());
                    }
                    let first = group.books[0];
                    let count = group.books.len();
                    let member_ids: Arc<[String]> = group
                        .books
                        .iter()
                        .map(|b| b.id.clone())
                        .collect::<Vec<_>>()
                        .into();
                    items.push(LibraryItem::Folder {
                        key: group.key.clone(),
                        name: group.name.clone(),
                        count,
                        cover_id: first.id.clone(),
                        member_ids,
                    });
                }

                for book in books
                    .iter()
                    .filter(|b| !grouped_ids.contains(b.id.as_str()))
                {
                    let index = self
                        .library
                        .books()
                        .iter()
                        .position(|b| b.id == book.id)
                        .unwrap_or(0);
                    items.push(LibraryItem::Book {
                        id: book.id.clone(),
                        index,
                    });
                }
            } else {
                for book in &books {
                    let index = self
                        .library
                        .books()
                        .iter()
                        .position(|b| b.id == book.id)
                        .unwrap_or(0);
                    items.push(LibraryItem::Book {
                        id: book.id.clone(),
                        index,
                    });
                }
            }

            let mut rows: Vec<Vec<LibraryItem>> =
                Vec::with_capacity((items.len() + cols - 1) / cols.max(1));
            for chunk in items.chunks(cols.max(1)) {
                rows.push(chunk.to_vec());
            }

            let row_count = rows.len();
            let item_sizes = std::rc::Rc::new(vec![size(px(content_w), px(row_slot_h)); row_count]);

            *cache = ui::library_view::LibraryRowCache {
                mutation_seq,
                save_seq,
                books_len,
                search_query: query.to_string(),
                sort_by,
                sort_direction,
                group_series,
                open_group: self.open_group.clone(),
                view_mode_grid: is_grid,
                cols,
                card_w,
                cover_h,
                gap,
                pad_x,
                rows,
                item_sizes,
            };
        }

        if cache.rows.is_empty() {
            return self.render_empty_library_state(radius, colors, &yomu, query.is_empty());
        }

        let item_sizes = cache.item_sizes.clone();
        drop(cache);

        let in_group = self.open_group.is_some();
        let scroll_handle = if in_group {
            &self.group_scroll
        } else {
            &self.library_scroll
        };

        let virtual_list = v_virtual_list(
            yomu.clone(),
            if in_group {
                "group-virtual-grid"
            } else {
                "library-virtual-grid"
            },
            item_sizes,
            move |this, visible_range, _window, cx| {
                let cache = this.library_row_cache.borrow();
                let yomu = cx.entity();
                let mut rendered = Vec::with_capacity(visible_range.len());

                for row_ix in visible_range {
                    let Some(row) = cache.rows.get(row_ix) else {
                        continue;
                    };

                    let row_element = if is_grid {
                        div()
                            .id(("grid-row", row_ix))
                            .w_full()
                            .h(px(row_slot_h))
                            .pb(px(gap))
                            .px(px(pad_x))
                            .flex()
                            .flex_row()
                            .items_start()
                            .gap(px(gap))
                            .children(row.iter().map(|item| match item {
                                LibraryItem::Folder {
                                    key,
                                    name,
                                    count,
                                    cover_id,
                                    member_ids,
                                } => {
                                    let cover_path = this.dirs.cover_for_size(cover_id, thumb_size);
                                    let selection_state =
                                        ui::library_view::compute_group_selection_state(
                                            member_ids.iter().map(String::as_str),
                                            &this.selection,
                                        );
                                    this.render_folder_card(
                                        key,
                                        name,
                                        *count,
                                        cover_path,
                                        card_w,
                                        cover_h,
                                        radius,
                                        selection_state,
                                        Arc::clone(member_ids),
                                        colors,
                                        &yomu,
                                    )
                                }
                                LibraryItem::Book { id, index } => {
                                    let book = this
                                        .library
                                        .books()
                                        .get(*index)
                                        .filter(|b| b.id == *id)
                                        .or_else(|| this.library.get(id));
                                    if let Some(book) = book {
                                        this.render_book_card(
                                            book, card_w, cover_h, radius, colors, &yomu,
                                        )
                                    } else {
                                        div().into_any_element()
                                    }
                                }
                            }))
                            .into_any_element()
                    } else {
                        div()
                            .id(("list-row", row_ix))
                            .w_full()
                            .max_w(px(1040.))
                            .mx_auto()
                            .px(px(pad_x))
                            .h(px(row_slot_h))
                            .pb(px(8.0))
                            .children(row.first().map(|item| match item {
                                LibraryItem::Folder {
                                    key,
                                    name,
                                    count,
                                    cover_id,
                                    member_ids,
                                } => {
                                    let cover_path = this.dirs.cover_for_size(cover_id, thumb_size);
                                    let selection_state =
                                        ui::library_view::compute_group_selection_state(
                                            member_ids.iter().map(String::as_str),
                                            &this.selection,
                                        );
                                    this.render_folder_list_row(
                                        key,
                                        name,
                                        *count,
                                        cover_path,
                                        radius,
                                        colors,
                                        border_px,
                                        selection_state,
                                        Arc::clone(member_ids),
                                        &yomu,
                                    )
                                }
                                LibraryItem::Book { id, index } => {
                                    let book = this
                                        .library
                                        .books()
                                        .get(*index)
                                        .filter(|b| b.id == *id)
                                        .or_else(|| this.library.get(id));
                                    if let Some(book) = book {
                                        this.render_book_list_row(book, radius, colors, &yomu)
                                    } else {
                                        div().into_any_element()
                                    }
                                }
                            }))
                            .into_any_element()
                    };

                    rendered.push(row_element);
                }

                rendered
            },
        )
        .track_scroll(scroll_handle)
        .pt(px(if is_grid { 24.0 } else { 16.0 }))
        .pb(px(if is_grid { 24.0 } else { 16.0 }));

        div()
            .id(if in_group {
                "group-grid"
            } else {
                "all-manga-container"
            })
            .size_full()
            .relative()
            .overflow_hidden()
            .child(
                ScrollBounce::new(
                    if in_group {
                        "group-scroll-bounce"
                    } else {
                        "library-scroll-bounce"
                    },
                    scroll_handle,
                    virtual_list,
                )
                .enabled(true)
                .scroll_wheel(false),
            )
            .child(Scrollbar::vertical(scroll_handle).viewport_from_layout())
            .into_any_element()
    }
}
