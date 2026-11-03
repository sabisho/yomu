//! Two-pane application shell, sidebar/toolbar adapters, and main view layout.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, Decorations, MouseButton, ScrollEvent, SharedString, Window, div, px,
};

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

        let sidebar_props = ui::sidebar::SidebarProps {
            scroll_handle: &self.sidebar_scroll,
            collapsed: is_collapsed,
            current_section: self.active_section,
            continue_reading_count,
            all_manga_count,
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
                        };
                        this.navigate_to(view, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |_, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.open_settings_window(window, cx);
                    });
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
        );

        // 2. Toolbar Props
        let is_client_decorated = cfg!(target_os = "linux")
            && matches!(window.window_decorations(), Decorations::Client { .. });
        let is_library_view = matches!(self.active_view, ui::sidebar::MainView::AllManga);
        let show_search = is_library_view;
        let show_view_switches = is_library_view;
        let show_sort = is_library_view;

        let (view_title, item_count): (Option<SharedString>, Option<SharedString>) =
            match &self.active_view {
                ui::sidebar::MainView::AllManga => {
                    if let Some(open_key) = &self.open_group {
                        let groups = ui::library_view::directory_groups(self.library.books());
                        let (title, count) =
                            if let Some(g) = groups.into_iter().find(|g| g.key == *open_key) {
                                (g.name, g.books.len())
                            } else {
                                (open_key.clone(), 0)
                            };
                        (
                            Some(SharedString::from(title)),
                            Some(SharedString::from(format!(
                                "{} {}",
                                count,
                                if count == 1 { "volume" } else { "volumes" }
                            ))),
                        )
                    } else {
                        let count = all_manga_count;
                        (
                            Some(SharedString::from("All Manga")),
                            Some(SharedString::from(format!(
                                "{} {}",
                                count,
                                if count == 1 { "item" } else { "items" }
                            ))),
                        )
                    }
                }
                ui::sidebar::MainView::ContinueReading => {
                    let count = continue_reading_count;
                    (
                        Some(SharedString::from("Continue Reading")),
                        Some(SharedString::from(format!("{} in progress", count))),
                    )
                }
                ui::sidebar::MainView::Details(id) => {
                    let title = self
                        .library
                        .get(id)
                        .map(|b| b.title.clone())
                        .unwrap_or_else(|| "Manga Details".to_string());
                    (Some(SharedString::from(title)), None)
                }
                ui::sidebar::MainView::Settings => (Some(SharedString::from("Settings")), None),
            };

        let current_order = self.current_view_order();
        let is_all_selected = self.selection.is_all_selected(&current_order);

        let search_input = self.search_input.as_ref();
        let toolbar_props = ui::toolbar::MainToolbarProps {
            can_go_back: self.open_group.is_some(),
            view_title,
            item_count,
            sidebar_collapsed: is_collapsed,
            is_scrolled: self.is_current_view_scrolled(),
            show_search,
            search_input,
            show_view_switches,
            is_grid_mode: self.view_mode_grid,
            show_sort,
            sort_by: self.library.settings().sort_by,
            sort_direction: self.library.settings().sort_direction,
            is_client_decorated,
            selection_count: self.selection.len(),
            is_all_selected,
            on_select_all: Some(Rc::new({
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.select_all(cx))
            })),
            on_unselect_all: Some(Rc::new({
                let yomu = yomu.clone();
                move |_, _, cx| yomu.update(cx, |this, cx| this.unselect_all(cx))
            })),
            on_remove_selected: Some(Rc::new({
                let yomu = yomu.clone();
                move |_, window, cx| {
                    yomu.update(cx, |this, cx| this.request_remove_selected(window, cx))
                }
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
                move |by, dir, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_sort_mode(by, dir, cx);
                    })
                }
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
            ui::sidebar::MainView::Details(book_id) => {
                let id = book_id.clone();
                self.render_details_screen(&id, window, cx)
            }
            ui::sidebar::MainView::Settings => self.render_settings_content(window, cx),
        };

        let view_key = match &self.active_view {
            ui::sidebar::MainView::ContinueReading => "continue-reading",
            ui::sidebar::MainView::AllManga => "all-manga",
            ui::sidebar::MainView::Details(_) => "details",
            ui::sidebar::MainView::Settings => "settings",
        };
        let view_duration = std::time::Duration::from_millis(140);
        let animated_content = gpui_kit::base::animation::EffectTransition::new(view_duration)
            .ease(gpui_kit::base::animation::ease_out_cubic)
            .fade(0.9, 1.0)
            .apply(
                div().size_full().child(content),
                (gpui_kit::ElementId::from("main-view-fade"), view_key),
            );

        gpui_kit::image_cache(self.cover_cache.clone())
            .size_full()
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_row()
                    .bg(colors.background)
                    .child(sidebar)
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
                                        if this
                                            .library_scroll_controller
                                            .handle_wheel(event, &scroll_handle)
                                        {
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
                                    .child(animated_content),
                            ),
                    ),
            )
            .into_any_element()
    }
}
