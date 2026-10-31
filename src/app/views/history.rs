//! History view presentation adapter.

use gpui_kit::{AnyElement, Context, Window, px};

use crate::app::state::Yomu;
use crate::ui;
use yomu::library::Book;

impl Yomu {
    pub(crate) fn render_history_view(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut history_books: Vec<&Book> = self
            .library
            .books()
            .iter()
            .filter(|b| b.has_progress())
            .collect();
        history_books.sort_by_key(|a| std::cmp::Reverse(a.last_read));

        let sidebar_collapsed = self.is_sidebar_collapsed(window);
        let props = ui::history_view::HistoryProps {
            scroll_handle: &self.library_scroll,
            history_books: &history_books,
            dirs: &self.dirs,
            colors: self.colors,
            radius: px(self.radius_px()),
            border_px: self.border_px(),
            radius_px: self.radius_px(),
            viewport_w: f32::from(window.viewport_size().width),
            sidebar_visible: !sidebar_collapsed,
            is_grid: self.view_mode_grid,
            thumb_size: self.library.settings().thumbnail_size,
        };

        let yomu = cx.entity();
        ui::history_view::render_history_view(props, move |book_id, _, cx| {
            yomu.update(cx, |this, cx| {
                this.navigate_to(ui::sidebar::MainView::Details(book_id), cx);
            });
        })
    }
}
