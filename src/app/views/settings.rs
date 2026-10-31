//! Settings dialog view presentation adapter and preferences wiring.

use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, Decorations, MouseButton, MouseDownEvent, Window, div, px, relative,
};

use crate::app::state::Yomu;
use crate::ui;
use yomu::util::format_size;

impl Yomu {
    pub(crate) fn render_settings(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let border_px = self.border_px();
        let cache_bytes = self.dirs.cache_size_bytes();
        let yomu = cx.entity();

        let is_client_decorated = cfg!(target_os = "linux")
            && matches!(window.window_decorations(), Decorations::Client { .. });

        let props = ui::settings_view::SettingsViewProps {
            scroll_handle: &self.settings_scroll,
            current_tab: self.settings_tab,
            settings: self.library.settings(),
            available_schemes: ui::theme::available_color_schemes(&self.custom_themes),
            cache_size_str: format_size(cache_bytes),
            is_client_decorated,
            system_reduced_motion: self.reduce_motion,
            vim_mode: self.is_vim_enabled(),
            colors: self.colors,
            radius: px(self.radius_xl_px()),
        };

        let view = ui::settings_view::render_settings_view(
            props,
            {
                let yomu = yomu.clone();
                move |tab, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.settings_tab = tab;
                        cx.notify();
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |_, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.close_overlay(cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_continuous(Some(checked), cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_webcomics(Some(checked), cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.toggle_group_series(checked, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.toggle_disable_animations(checked, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.library.settings_mut().remember_zoom = checked;
                        this.save_library(cx);
                        cx.notify();
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |checked, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.toggle_vim_mode(checked, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |mode, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_theme_mode(mode, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |scheme, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_color_scheme(&scheme, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |size, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_thumbnail_size(size, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |rounding, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_corner_rounding(rounding, cx);
                    });
                }
            },
            {
                let yomu = yomu.clone();
                move |border, _, cx| {
                    yomu.update(cx, |this, cx| {
                        this.set_border_size(border, cx);
                    });
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

        let yomu_scrim = yomu.clone();
        ui::overlays::scrim(true, self.colors, move |_, cx| {
            yomu_scrim.update(cx, |this, cx| this.close_overlay(cx));
        })
        .child(
            div()
                .w(px(860.))
                .min_w(px(640.))
                .max_w(relative(0.92))
                .h(px(620.))
                .min_h(px(460.))
                .max_h(relative(0.90))
                .rounded(px(self.radius_xl_px()))
                .when(border_px > 0.0, |this| {
                    this.border(px(border_px)).border_color(self.colors.border)
                })
                .bg(self.colors.background)
                .shadow_xl()
                .overflow_hidden()
                .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                })
                .child(view),
        )
        .into_any_element()
    }
}
