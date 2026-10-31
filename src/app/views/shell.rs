//! Two-pane application shell, sidebar/toolbar adapters, and main view layout.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Decorations, MouseButton, ScrollEvent, Window, div, px};

use crate::app::state::Yomu;
use crate::ui;

impl Yomu {
    pub(crate) fn render_two_pane_shell(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors;
        let radius = px(self.radius_px());
        let yomu = cx.entity();

        let auto_collapse = window.viewport_size().width < px(720.);
        let is_collapsed = self.sidebar_collapsed || auto_collapse;

        // 1. Sidebar Props
        let continue_reading_count = self
            .library
            .books()
            .iter()
            .filter(|b| b.has_progress())
            .count();
        let all_manga_count = self.library.books().len();
        let collections_count = self.library.collections.len() + 4;
        let downloads_count = self
            .library
            .books()
            .iter()
            .filter(|b| b.path.exists())
            .count();

        let sidebar_props = ui::sidebar::SidebarProps {
            scroll_handle: &self.sidebar_scroll,
            current_section: self.active_section,
            continue_reading_count,
            all_manga_count,
            collections_count,
            downloads_count,
            colors,
            radius,
        };

        let sidebar = ui::sidebar::render_sidebar(
            sidebar_props,
            {
                let yomu = yomu.clone();
                move |sec, _, cx| {
                    yomu.update(cx, |this, cx| {
                        let view = match sec {
                            ui::sidebar::SidebarSection::ContinueReading => {
                                ui::sidebar::MainView::ContinueReading
                            }
                            ui::sidebar::SidebarSection::AllManga => {
                                ui::sidebar::MainView::AllManga
                            }
                            ui::sidebar::SidebarSection::Collections => {
                                ui::sidebar::MainView::Collections
                            }
                            ui::sidebar::SidebarSection::History => ui::sidebar::MainView::History,
                            ui::sidebar::SidebarSection::Downloads => {
                                ui::sidebar::MainView::Downloads
                            }
                        };
                        this.navigate_to(view, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.open_settings(cx);
                    });
                }
            },
        );

        // 2. Toolbar Props
        let is_client_decorated = cfg!(target_os = "linux")
            && matches!(window.window_decorations(), Decorations::Client { .. });
        let is_library_view = matches!(
            self.active_view,
            ui::sidebar::MainView::AllManga
                | ui::sidebar::MainView::History
                | ui::sidebar::MainView::Downloads
        );
        let is_browsing =
            is_library_view || matches!(self.active_view, ui::sidebar::MainView::ContinueReading);
        let show_search = is_library_view;
        let show_view_switches = is_library_view;
        let show_grouping = is_library_view;
        let show_import = is_browsing;

        let search_input = self.search_input.as_ref();
        let toolbar_props = ui::toolbar::MainToolbarProps {
            can_go_back: self.open_group.is_some(),
            sidebar_collapsed: is_collapsed,
            is_scrolled: self.is_current_view_scrolled(),
            show_search,
            search_input,
            show_view_switches,
            is_grid_mode: self.view_mode_grid,
            show_grouping,
            is_group_series: self.library.settings().group_series,
            show_import,
            is_client_decorated,
            selection_count: self.selection.len(),
            on_select_all: Some(Rc::new({
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.select_all(cx))
            })),
            on_remove_selected: Some(Rc::new({
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.request_remove_selected(cx))
            })),
            on_cancel_selection: Some(Rc::new({
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.selection.clear();
                        cx.notify();
                    })
                }
            })),
            colors,
            radius,
        };

        let toolbar = ui::toolbar::render_main_toolbar(
            toolbar_props,
            {
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.toggle_sidebar(cx))
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.exit_group(cx))
            },
            {
                let yomu = yomu.clone();
                move |grid, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.view_mode_grid = grid;
                        this.library.settings_mut().view_mode_grid = grid;
                        this.save_library(cx);
                        cx.notify();
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |group, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.toggle_group_series(group, cx);
                    })
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.import_dialog(false, cx))
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.import_dialog(true, cx))
            },
            {
                let yomu = yomu.clone();
                Some(Rc::new(move |_, _, cx| {
                    yomu.update(cx, |this, cx| this.request_quit(cx));
                }))
            },
        );

        // 3. Main Content
        let content: AnyElement = match &self.active_view {
            ui::sidebar::MainView::ContinueReading => self.render_continue_reading_view(window, cx),
            ui::sidebar::MainView::AllManga => self.render_all_manga_view(window, cx),
            ui::sidebar::MainView::Collections => self.render_collections_screen(window, cx),
            ui::sidebar::MainView::History => self.render_history_view(window, cx),
            ui::sidebar::MainView::Downloads => self.render_downloads_view(window, cx),
            ui::sidebar::MainView::Details(book_id) => {
                let id = book_id.clone();
                self.render_details_screen(&id, window, cx)
            }
            ui::sidebar::MainView::Settings => self.render_settings(window, cx),
        };

        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(colors.background)
            .when(!is_collapsed, |s| s.child(sidebar))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(toolbar)
                    .child(
                        div()
                            .id("main-view-content")
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                                let scroll_handle = this.current_scroll_handle();
                                let animations_enabled = this.animations_enabled();
                                if this.library_scroll_controller.handle_wheel(
                                    event,
                                    &scroll_handle,
                                    animations_enabled,
                                ) {
                                    this.wake();
                                    cx.notify();
                                }
                            }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, _| {
                                    this.library_scroll_controller.cancel_inertia();
                                }),
                            )
                            .child(content),
                    ),
            )
            .into_any_element()
    }
}
