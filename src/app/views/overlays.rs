//! Modal dialog, context menu, and command prompt view presentation adapters.

use gpui_kit::{AnyElement, App, Context, MouseDownEvent, Window};

use crate::app::state::Yomu;
use crate::ui;
use crate::ui::overlays::{
    BookMenu, ConfirmDialog, LibraryMenu, Overlay, PropertiesDialog, SeriesDialog,
};
use crate::ui::reader::viewport_of;
use yomu::menu::BookAction;

impl Yomu {
    pub(crate) fn render_overlay(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        Some(match self.overlay.as_ref()? {
            Overlay::Menu(menu) => self.render_menu(menu, cx),
            Overlay::LibraryMenu(menu) => self.render_library_menu(menu, cx),
            Overlay::Confirm(dialog) => self.render_confirm(dialog, cx),
            Overlay::Properties(dialog) => self.render_properties(dialog, cx),
            Overlay::Series(dialog) => self.render_series_dialog(dialog, cx),
            Overlay::Collection(dialog) => self.render_collection_dialog(dialog, cx),
            Overlay::Settings => self.render_settings(window, cx),
        })
    }

    pub(crate) fn overlay_style(&self, surface_bg: gpui_kit::Hsla) -> ui::overlays::OverlayStyle {
        ui::overlays::OverlayStyle {
            colors: self.colors,
            border_px: self.border_px(),
            radius_px: self.radius_px(),
            rounding_lg: self.radius_lg_px(),
            rounding_xl: self.radius_xl_px(),
            surface_bg,
            animations_enabled: self.animations_enabled(),
        }
    }

    pub(crate) fn render_menu(&self, menu: &BookMenu, cx: &mut Context<Self>) -> AnyElement {
        let style = self.overlay_style(self.menu_surface());
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.close_overlay(cx));
        };

        let yomu_hover = yomu.clone();
        let on_hover = move |ix: usize, _window: &mut Window, cx: &mut App| {
            yomu_hover.update(cx, |this, cx| this.menu_hover(ix, cx));
        };

        let yomu_action = yomu.clone();
        let on_action = move |action: BookAction,
                              book_id: &str,
                              target_ids: &[String],
                              window: &mut Window,
                              cx: &mut App| {
            let book_id = book_id.to_string();
            let target_ids = target_ids.to_vec();
            yomu_action.update(cx, |this, cx| {
                this.run_book_action(action, &book_id, &target_ids, window, cx);
            });
        };

        ui::overlays::render_book_menu(menu, style, on_close, on_hover, on_action)
    }

    pub(crate) fn render_library_menu(
        &self,
        menu: &LibraryMenu,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let style = self.overlay_style(self.menu_surface());
        let group_series = self.library.settings().group_series;
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.close_overlay(cx));
        };

        let yomu_hover = yomu.clone();
        let on_hover = move |ix: usize, _window: &mut Window, cx: &mut App| {
            yomu_hover.update(cx, |this, cx| {
                if let Some(Overlay::LibraryMenu(menu)) = this.overlay.as_mut() {
                    menu.selected = ix;
                    cx.notify();
                }
            });
        };

        let yomu_action = yomu.clone();
        let on_action = move |ix: usize, window: &mut Window, cx: &mut App| {
            yomu_action.update(cx, |this, cx| {
                this.run_library_menu_action(ix, window, cx);
            });
        };

        ui::overlays::render_library_menu(menu, group_series, style, on_close, on_hover, on_action)
    }

    pub(crate) fn render_confirm(
        &self,
        dialog: &ConfirmDialog,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let style = self.overlay_style(self.widget_surface(self.colors.surface));
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.close_overlay(cx));
        };

        let yomu_accept = yomu.clone();
        let on_accept = move |window: &mut Window, cx: &mut App| {
            yomu_accept.update(cx, |this, cx| {
                this.confirm_accept(window, cx);
            });
        };

        ui::overlays::render_confirm_dialog(dialog, style, on_close, on_accept)
    }

    pub(crate) fn render_properties(
        &self,
        dialog: &PropertiesDialog,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let style = self.overlay_style(self.widget_surface(self.colors.surface));
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.close_overlay(cx));
        };

        ui::overlays::render_properties_dialog(dialog, style, on_close)
    }

    pub(crate) fn render_series_dialog(
        &self,
        dialog: &SeriesDialog,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let style = self.overlay_style(self.widget_surface(self.colors.surface));
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.close_overlay(cx));
        };

        let yomu_save = yomu.clone();
        let on_save = move |_window: &mut Window, cx: &mut App| {
            yomu_save.update(cx, |this, cx| this.save_series_dialog(cx));
        };

        ui::overlays::render_series_dialog(dialog, style, on_close, on_save)
    }

    pub(crate) fn render_collection_dialog(
        &self,
        dialog: &ui::overlays::CollectionDialog,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let style = self.overlay_style(self.colors.background);
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.close_overlay(cx));
        };

        let yomu_save = yomu.clone();
        let on_save =
            move |name: &str, editing_id: Option<&str>, _window: &mut Window, cx: &mut App| {
                let name = name.to_string();
                let editing_id = editing_id.map(|s| s.to_string());
                yomu_save.update(cx, |this, cx| {
                    if let Some(id) = &editing_id {
                        this.library.rename_collection(id, &name);
                    } else {
                        this.library.add_collection(&name);
                    }
                    this.save_library(cx);
                    this.close_overlay(cx);
                });
            };

        ui::overlays::render_collection_dialog(dialog, style, on_close, on_save)
    }

    pub(crate) fn render_command_box(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let input = self.cmdline_text()?;
        let style = self.overlay_style(self.colors.surface);
        let yomu = cx.entity();

        let yomu_close = yomu.clone();
        let on_close = move |_: &MouseDownEvent, _window: &mut Window, cx: &mut App| {
            yomu_close.update(cx, |this, cx| this.cancel_cmdline(cx));
        };

        let yomu_select = yomu.clone();
        let on_select = move |index: usize, _window: &mut Window, cx: &mut App| {
            yomu_select.update(cx, |this, cx| {
                if this.command_selection != index {
                    this.command_selection = index;
                    if let Some(draft) = this.cmdline_text().map(str::to_owned) {
                        this.sync_theme_preview(&draft, cx);
                    }
                    cx.notify();
                }
            });
        };

        let yomu_submit = yomu.clone();
        let on_submit = move |completion: String, window: &mut Window, cx: &mut App| {
            let is_complete = !completion.ends_with(' ');
            yomu_submit.update(cx, |this, cx| {
                this.replace_cmdline(completion.clone());
                if is_complete {
                    let vp = viewport_of(window);
                    let actions = if let Some(reader) = this.reader.as_mut() {
                        reader.vim.feed("enter")
                    } else {
                        this.library_vim.feed("enter")
                    };
                    this.command_selection = 0;
                    this.theme_before_preview = None;
                    for action in actions {
                        this.apply(action, vp, window, cx);
                    }
                } else {
                    this.command_selection = 0;
                    if let Some(draft) = this.cmdline_text().map(str::to_owned) {
                        this.sync_theme_preview(&draft, cx);
                    }
                }
                cx.notify();
            });
        };

        Some(ui::overlays::render_command_box(
            input,
            self.command_selection,
            &self.custom_themes,
            style,
            on_close,
            on_select,
            on_submit,
        ))
    }
}
