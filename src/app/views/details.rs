//! Manga details inspector view presentation adapter.

use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Window, div, point, px};

use crate::app::state::Yomu;
use crate::ui;

impl Yomu {
    pub(crate) fn render_details_screen(
        &self,
        book_id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(book) = self.library.get(book_id) else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child("Manga not found")
                .into_any_element();
        };

        let colors = self.colors;
        let radius = px(self.radius_px());
        let yomu = cx.entity();
        let cover_path = self.dirs.cover(&book.id);

        let props = ui::details::MangaDetailsProps {
            scroll_handle: &self.details_scroll,
            book,
            cover_path,
            selected_tab: self.details_tab_index,
            synopsis_expanded: self.details_synopsis_expanded,
            colors,
            radius,
        };

        ui::details::render_manga_details(
            props,
            {
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.navigate_back(cx))
            },
            {
                let yomu = yomu.clone();
                let book_id = book.id.clone();
                move |_, window, cx| {
                    yomu.update(cx, |this, cx| this.open_book(&book_id, window, cx))
                }
            },
            {
                let yomu = yomu.clone();
                move |idx, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.details_tab_index = idx;
                        cx.notify();
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.details_synopsis_expanded = !this.details_synopsis_expanded;
                        cx.notify();
                    })
                }
            },
            {
                let yomu = yomu.clone();
                let book_id = book.id.clone();
                move |_, pos, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.open_book_menu(&book_id, point(px(pos.0), px(pos.1)), cx)
                    })
                }
            },
        )
    }
}
