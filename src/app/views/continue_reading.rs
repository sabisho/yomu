//! Continue reading dashboard view presentation adapter.

use std::collections::HashSet;

use gpui_kit::{AnyElement, Context, Window, point, px};

use crate::app::state::Yomu;
use crate::ui;
use crate::ui::library_view::{LibraryGroup, directory_groups};
use yomu::library::Book;

impl Yomu {
    pub(crate) fn render_continue_reading_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let radius = px(self.radius_px());
        let yomu = cx.entity();

        let books = self.library.books();
        let hero_book = books
            .iter()
            .filter(|b| b.has_progress())
            .max_by_key(|b| b.last_read)
            .or_else(|| books.first());

        let hero = hero_book.map(|book| {
            let cover_path = self.dirs.cover(&book.id);
            let display_title = book.title.as_str();
            let total = book.page_count;
            let current = if book.opened {
                book.current_page + 1
            } else {
                0
            };
            let progress = book.progress();
            let chapter_label = format!("Ch. {current} of {total}");
            let genres = book
                .series
                .as_ref()
                .map(|s| vec![s.clone()])
                .unwrap_or_default();

            ui::continue_reading::ContinueReadingItem {
                book,
                cover_path,
                display_title,
                chapter_label,
                progress,
                genres,
            }
        });

        let mut sorted_queue: Vec<&Book> = books.iter().filter(|b| b.has_progress()).collect();
        sorted_queue.sort_by_key(|a| std::cmp::Reverse(a.last_read));

        let queue: Vec<ui::continue_reading::ContinueReadingItem> = sorted_queue
            .iter()
            .skip(1)
            .take(3)
            .map(|book| {
                let cover_path = self.dirs.cover(&book.id);
                let display_title = book.title.as_str();
                let _total = book.page_count;
                let current = if book.opened {
                    book.current_page + 1
                } else {
                    0
                };
                let progress = book.progress();
                let chapter_label = format!("Ch. {current}");

                ui::continue_reading::ContinueReadingItem {
                    book,
                    cover_path,
                    display_title,
                    chapter_label,
                    progress,
                    genres: Vec::new(),
                }
            })
            .collect();

        let thumb_size = self.library.settings().thumbnail_size;
        let sidebar_collapsed = self.is_sidebar_collapsed(window);
        let grid = ui::components::compute_responsive_grid(
            f32::from(window.viewport_size().width),
            !sidebar_collapsed,
            thumb_size,
        );
        let card_w = grid.card_w;
        let cover_h = grid.cover_h;

        let mut cards: Vec<AnyElement> = Vec::new();
        if self.library.settings().group_series {
            let groups = directory_groups(books.iter());
            let mut grouped_ids = HashSet::new();
            for group in &groups {
                for b in &group.books {
                    grouped_ids.insert(b.id.clone());
                }
            }

            enum RecentEntry<'a> {
                Group(&'a LibraryGroup<'a>, u64),
                Book(&'a Book, u64),
            }

            let mut entries: Vec<RecentEntry> = Vec::new();
            for group in &groups {
                let latest_added = group.books.iter().map(|b| b.added_at).max().unwrap_or(0);
                entries.push(RecentEntry::Group(group, latest_added));
            }
            for book in books.iter().filter(|b| !grouped_ids.contains(&b.id)) {
                entries.push(RecentEntry::Book(book, book.added_at));
            }

            entries.sort_by(|a, b| {
                let a_time = match a {
                    RecentEntry::Group(_, t) => *t,
                    RecentEntry::Book(_, t) => *t,
                };
                let b_time = match b {
                    RecentEntry::Group(_, t) => *t,
                    RecentEntry::Book(_, t) => *t,
                };
                b_time.cmp(&a_time)
            });

            for entry in entries.into_iter().take(grid.cols) {
                match entry {
                    RecentEntry::Group(group, _) => {
                        let first = group.books[0];
                        let cover_path = self.dirs.cover_for_size(&first.id, thumb_size);
                        let book_ids: Vec<String> =
                            group.books.iter().map(|b| b.id.clone()).collect();
                        cards.push(self.render_folder_card(
                            &group.key,
                            &group.name,
                            &book_ids,
                            group.books.len(),
                            cover_path,
                            card_w,
                            cover_h,
                            radius,
                            colors,
                            &yomu,
                        ));
                    }
                    RecentEntry::Book(book, _) => {
                        cards.push(
                            self.render_book_card(book, card_w, cover_h, radius, colors, &yomu),
                        );
                    }
                }
            }
        } else {
            let mut sorted_books: Vec<&Book> = books.iter().collect();
            sorted_books.sort_by_key(|a| std::cmp::Reverse(a.added_at));
            for book in sorted_books.into_iter().take(grid.cols) {
                cards.push(self.render_book_card(book, card_w, cover_h, radius, colors, &yomu));
            }
        }

        let props = ui::continue_reading::ContinueReadingProps {
            scroll_handle: &self.continue_reading_scroll,
            hero,
            queue,
            recent_cards: cards,
            recent_gap: px(grid.gap),
            pad_x: px(grid.pad_x),
            colors,
            radius,
            border_px: self.border_px(),
        };

        ui::continue_reading::render_continue_reading_view(
            props,
            {
                let yomu = yomu.clone();
                move |id, window, cx| yomu.update(cx, |this, cx| this.open_book(id, window, cx))
            },
            {
                let yomu = yomu.clone();
                move |id, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.navigate_to(ui::sidebar::MainView::Details(id.to_string()), cx)
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.navigate_to(ui::sidebar::MainView::AllManga, cx)
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |id, pos, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.open_book_menu(id, point(px(pos.0), px(pos.1)), cx)
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.import_dialog(false, cx);
                    });
                }
            },
        )
    }
}
