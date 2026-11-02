//! Settings dialog view presentation adapter and preferences wiring.

use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Window, div, px};

use crate::app::state::Yomu;
use crate::ui;
use yomu::util::format_size;

impl Yomu {
    pub(crate) fn render_settings_content(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cache_bytes = self.dirs.cache_size_bytes();
        let yomu = cx.entity();

        let props = ui::settings_view::SettingsViewProps {
            scroll_handle: &self.settings_scroll,
            current_tab: self.settings_tab,
            settings: self.library.settings(),
            available_schemes: ui::theme::available_color_schemes(&self.custom_themes),
            cache_size_str: format_size(cache_bytes),
            vim_mode: self.is_vim_enabled(),
            colors: self.colors,
            radius: px(self.radius_xl_px()),
        };

        let view = ui::settings_view::render_settings_view(
            props,
            {
                let yomu = yomu.clone();
                move |tab, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.settings_tab = tab;
                        cx.notify();
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |layout, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_page_layout(layout, cx);
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |dir, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_reading_direction(dir, cx);
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |cover, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_first_page_is_cover(cover, cx);
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |threshold, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_spread_ratio_threshold(threshold, cx);
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_continuous(Some(checked), cx);
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_webcomics(Some(checked), cx);
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.toggle_group_series(checked, cx);
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.library.settings_mut().remember_zoom = checked;
                        this.save_library(cx);
                        cx.notify();
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.toggle_vim_mode(checked, cx);
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |mode, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_theme_mode(mode, cx);
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |scheme, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_color_scheme(&scheme, cx);
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |size, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_thumbnail_size(size, cx);
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |rounding, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_corner_rounding(rounding, cx);
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |border, window, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_border_size(border, cx);
                    });
                    window.refresh();
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.request_clear_thumbnail_cache(cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.request_reset_settings(cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.request_clear_all_data(cx);
                    });
                }
            },
        );

        div()
            .w(px(840.))
            .h(px(520.))
            .size_full()
            .overflow_hidden()
            .child(view)
            .into_any_element()
    }
}
