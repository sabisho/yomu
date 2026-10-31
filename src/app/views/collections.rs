//! Collections screen view presentation adapter.

use gpui_kit::component::input::InputState;
use gpui_kit::{AnyElement, AppContext, Context, Window, px};

use crate::app::state::Yomu;
use crate::ui;

impl Yomu {
    pub(crate) fn render_collections_screen(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let radius = px(self.radius_px());
        let yomu = cx.entity();

        let favorites_count = self.library.favorites.len();
        let reading_count = self
            .library
            .books()
            .iter()
            .filter(|b| b.has_progress())
            .count();
        let want_to_read_count = self
            .library
            .books()
            .iter()
            .filter(|b| !b.has_progress())
            .count();
        let completed_count = self
            .library
            .books()
            .iter()
            .filter(|b| b.is_finished())
            .count();

        let props = ui::collections_view::CollectionsProps {
            scroll_handle: &self.collections_scroll,
            favorites_count,
            reading_count,
            want_to_read_count,
            completed_count,
            custom_collections: self.library.collections.as_slice(),
            colors,
            radius,
        };

        ui::collections_view::render_collections_view(
            props,
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.navigate_to(ui::sidebar::MainView::AllManga, cx);
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |id, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.selected_collection_id = Some(id.to_string());
                        this.navigate_to(ui::sidebar::MainView::AllManga, cx);
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |_, window, cx| {
                    yomu.update(cx, |this, cx| {
                        let input = cx
                            .new(|cx| InputState::new(window, cx).placeholder("Collection Name…"));
                        this.overlay = Some(ui::overlays::Overlay::Collection(
                            ui::overlays::CollectionDialog {
                                is_new: true,
                                collection_id: None,
                                input,
                            },
                        ));
                        cx.notify();
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |id, _pos, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.overlay = Some(ui::overlays::Overlay::Confirm(
                            ui::overlays::ConfirmDialog {
                                title: "Delete Collection?".into(),
                                message: "This collection will be deleted. Underlying manga files will not be affected.".into(),
                                confirm_label: "Delete",
                                action: ui::overlays::ConfirmAction::DeleteCollection(id.to_string()),
                            },
                        ));
                        cx.notify();
                    })
                }
            },
        )
    }
}
