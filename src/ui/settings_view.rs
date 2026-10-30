//! Split-pane Settings view matching macOS Human Interface Guidelines.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable, Icon, Side, Sizable, Size};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement, Pixels,
    ScrollHandle, SharedString, Styled, Window, div, px,
};

use super::components::{
    macos_sidebar_bg, open_shortcut_str, settings_shortcut_str, with_alpha, zoom_shortcut_str,
};
use super::overlays::SettingsTab;
use super::theme::{UiColors, is_standalone_scheme};
use crate::library::{BorderSize, CornerRounding, Settings, ThemeModePreference, ThumbnailSize};

pub struct SettingsViewProps<'a> {
    pub scroll_handle: &'a ScrollHandle,
    pub current_tab: SettingsTab,
    pub settings: &'a Settings,
    pub available_schemes: Vec<String>,
    pub cache_size_str: String,
    pub is_client_decorated: bool,
    pub system_reduced_motion: bool,
    pub vim_mode: bool,
    pub colors: UiColors,
    pub radius: Pixels,
}

pub fn render_settings_view(
    props: SettingsViewProps,
    on_select_tab: impl Fn(SettingsTab, &mut Window, &mut App) + 'static,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_toggle_continuous: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_webcomics: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_group_series: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_disable_animations: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_remember_zoom: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_vim_mode: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_set_theme_mode: impl Fn(ThemeModePreference, &mut Window, &mut App) + 'static,
    on_set_color_scheme: impl Fn(String, &mut Window, &mut App) + 'static,
    on_set_thumbnail_size: impl Fn(ThumbnailSize, &mut Window, &mut App) + 'static,
    on_set_corner_rounding: impl Fn(CornerRounding, &mut Window, &mut App) + 'static,
    on_set_border_size: impl Fn(BorderSize, &mut Window, &mut App) + 'static,
    on_clear_cache: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_reset_settings: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_reset_all_data: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_select_tab = std::rc::Rc::new(on_select_tab);
    let on_close = std::rc::Rc::new(on_close);
    let on_set_theme_mode = std::rc::Rc::new(on_set_theme_mode);
    let on_set_color_scheme = std::rc::Rc::new(on_set_color_scheme);
    let on_set_thumbnail_size = std::rc::Rc::new(on_set_thumbnail_size);
    let on_set_corner_rounding = std::rc::Rc::new(on_set_corner_rounding);
    let on_set_border_size = std::rc::Rc::new(on_set_border_size);
    let colors = props.colors;

    let tabs = [
        (SettingsTab::General, IconName::Settings, "General"),
        (SettingsTab::Appearance, IconName::Palette, "Appearance"),
        (SettingsTab::Reader, IconName::BookOpen, "Reader"),
        (SettingsTab::Library, IconName::Library, "Library"),
        (SettingsTab::Shortcuts, IconName::Keyboard, "Shortcuts"),
        (SettingsTab::Advanced, IconName::Database, "Advanced"),
    ];

    div()
        .size_full()
        .rounded(props.radius)
        .overflow_hidden()
        .flex()
        .flex_row()
        .bg(colors.background)
        // Left macOS System Settings Navigation Sidebar
        .child(
            div()
                .w(px(210.))
                .flex_none()
                .h_full()
                .rounded_l(props.radius)
                .overflow_hidden()
                .p_3()
                .border_r_1()
                .border_color(with_alpha(colors.border, 0.45))
                .bg(macos_sidebar_bg(colors))
                .flex()
                .flex_col()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        // Platform-aware header: close button placement adapts to OS & decoration mode
                        .child({
                            let is_macos = cfg!(target_os = "macos");
                            let is_windows = cfg!(target_os = "windows");
                            let close_btn = div()
                                .id("settings-close-btn")
                                .size(px(20.))
                                .rounded(px(4.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(|s| s.bg(with_alpha(colors.foreground, 0.08)))
                                .tooltip(|window, cx| {
                                    Tooltip::new("Close settings (Esc)").build(window, cx)
                                })
                                .on_click({
                                    let on_close = on_close.clone();
                                    move |e, window, cx| on_close(e, window, cx)
                                })
                                .child(
                                    Icon::new(IconName::Close)
                                        .size(px(14.))
                                        .text_color(colors.muted),
                                );
                            let title = div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.foreground)
                                .child("Settings");

                            if is_macos {
                                div()
                                    .px_2()
                                    .pt_1p5()
                                    .pb_3()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(close_btn)
                                    .child(title)
                            } else if is_windows || props.is_client_decorated {
                                div()
                                    .px_2()
                                    .pt_1p5()
                                    .pb_3()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(title)
                                    .child(close_btn)
                            } else {
                                div()
                                    .px_2()
                                    .pt_1p5()
                                    .pb_3()
                                    .flex()
                                    .items_center()
                                    .child(title)
                            }
                        })
                        .children(tabs.iter().map(|&(tab, icon, label)| {
                            let on_select_tab = on_select_tab.clone();
                            let active = props.current_tab == tab;
                            let (bg, text_col, icon_col) = if active {
                                (
                                    colors.accent.into(),
                                    colors.accent_foreground,
                                    colors.accent_foreground,
                                )
                            } else {
                                (
                                    with_alpha(colors.foreground, 0.0),
                                    colors.foreground,
                                    colors.muted,
                                )
                            };

                            div()
                                .id(ElementId::from(SharedString::from(format!(
                                    "settings-tab-item-{}",
                                    tab.label()
                                ))))
                                .h(px(32.))
                                .px_3()
                                .rounded(props.radius)
                                .flex()
                                .items_center()
                                .gap_2p5()
                                .bg(bg)
                                .text_color(text_col)
                                .cursor_pointer()
                                .when(!active, |this| {
                                    this.hover(|style| style.bg(with_alpha(colors.foreground, 0.06)))
                                })
                                .on_click(move |_, window, cx| on_select_tab(tab, window, cx))
                                .child(Icon::new(icon).size(px(16.)).text_color(icon_col))
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(if active {
                                            FontWeight::SEMIBOLD
                                        } else {
                                            FontWeight::NORMAL
                                        })
                                        .child(label),
                                )
                        })),
                ),
        )
        // Right macOS Settings Content Pane
        .child(
            div()
                .id("settings-content-scroll")
                .flex_1()
                .min_w_0()
                .h_full()
                .rounded_r(props.radius)
                .overflow_y_scrollbar()
                .track_scroll(props.scroll_handle)
                .scroll_bounce(true)
                .scroll_wheel(false)
                .px_7()
                .py_6()
                .flex()
                .flex_col()
                .gap_4()
                    .child(
                        div()
                            .pb_1()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(colors.foreground)
                                    .child(props.current_tab.label()),
                            )
                            .child(
                                div().text_xs().text_color(colors.muted).child(match props
                                    .current_tab
                                {
                                    SettingsTab::General => {
                                        "Configure general reading defaults and view application information."
                                    }
                                    SettingsTab::Appearance => {
                                        "Customize theme colors, card cover dimensions, rounding, and borders."
                                    }
                                    SettingsTab::Reader => {
                                        "Control reader transitions, layout scrolling, zoom retention, and navigation."
                                    }
                                    SettingsTab::Library => {
                                        "Manage library organization, folder grouping, and cover size presets."
                                    }
                                    SettingsTab::Shortcuts => {
                                        "Quick keyboard shortcut reference for navigation and reading controls."
                                    }
                                    SettingsTab::Advanced => {
                                        "Manage local thumbnail cover cache and reset library data."
                                    }
                                }),
                            ),
                    )
                    .child(match props.current_tab {
                        SettingsTab::General => {
                            render_tab_general(props.settings, colors, on_toggle_continuous)
                        }
                        SettingsTab::Appearance => render_tab_appearance(
                            props.settings,
                            &props.available_schemes,
                            colors,
                            on_set_theme_mode.clone(),
                            on_set_color_scheme,
                            on_set_corner_rounding,
                            on_set_border_size,
                        ),
                        SettingsTab::Reader => render_tab_reader(
                            props.settings,
                            props.system_reduced_motion,
                            colors,
                            on_toggle_continuous,
                            on_toggle_webcomics,
                            on_toggle_disable_animations,
                            on_toggle_remember_zoom,
                            on_toggle_vim_mode,
                        ),
                        SettingsTab::Library => render_tab_library(
                            props.settings,
                            colors,
                            on_toggle_group_series,
                            on_set_thumbnail_size,
                        ),
                        SettingsTab::Shortcuts => {
                            render_tab_shortcuts(colors, props.vim_mode)
                        }
                        SettingsTab::Advanced => render_tab_advanced(
                            &props.cache_size_str,
                            colors,
                            on_clear_cache,
                            on_reset_settings,
                            on_reset_all_data,
                        ),
                    }),
            )
            .into_any_element()
}

fn settings_card(colors: UiColors) -> Div {
    div()
        .w_full()
        .rounded(px(10.))
        .bg(with_alpha(colors.surface, 0.65))
        .border_1()
        .border_color(with_alpha(colors.border, 0.45))
        .overflow_hidden()
        .flex()
        .flex_col()
}

fn settings_divider(colors: UiColors) -> Div {
    div()
        .pl_4()
        .w_full()
        .child(div().w_full().h(px(1.)).bg(with_alpha(colors.border, 0.25)))
}

fn settings_row(
    title: &'static str,
    description: impl Into<SharedString>,
    colors: UiColors,
) -> Div {
    div()
        .w_full()
        .min_h(px(44.))
        .px_4()
        .py_2p5()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(colors.foreground)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.muted)
                        .child(description.into()),
                ),
        )
}

pub fn macos_dropdown_button<T: Clone + PartialEq + 'static>(
    id: impl Into<ElementId>,
    label: &str,
    options: Vec<(String, T, bool)>,
    on_select: std::rc::Rc<dyn Fn(T, &mut Window, &mut App)>,
) -> AnyElement {
    Button::new(id)
        .small()
        .outline()
        .dropdown_caret(true)
        .rounded(ButtonRounded::Size(px(6.)))
        .label(label.to_string())
        .dropdown_menu(move |mut menu, _, _| {
            menu = menu
                .check_side(Side::Left)
                .min_w(px(180.))
                .max_h(px(280.))
                .scrollable(true);
            for (opt_label, val, checked) in &options {
                let on_select = on_select.clone();
                let val = val.clone();
                menu = menu.item(
                    PopupMenuItem::new(opt_label.clone())
                        .checked(*checked)
                        .on_click(move |_, window, cx| on_select(val.clone(), window, cx)),
                );
            }
            menu
        })
        .into_any_element()
}

fn render_tab_general(
    settings: &Settings,
    colors: UiColors,
    on_toggle_continuous: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            settings_card(colors).child(
                settings_row(
                    "Default Reading Layout",
                    "Toggle between single-page book turns and continuous vertical strip.",
                    colors,
                )
                .child(
                    div().cursor_pointer().child(
                        Switch::new("gen-continuous-mode-switch")
                            .accessibility_label("Continuous vertical reading mode")
                            .small()
                            .checked(settings.continuous)
                            .on_click(move |val, window, cx| on_toggle_continuous(*val, window, cx)),
                    ),
                ),
            ),
        )
        .child(
            settings_card(colors).child(
                settings_row(
                    "About Yomu",
                    "Yomu v26.10.0 · High performance desktop comic reader built with GPUI Kit and Rust.",
                    colors,
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .px_2p5()
                                .py_1()
                                .rounded(px(6.))
                                .bg(with_alpha(colors.foreground, 0.08))
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.muted)
                                .child("v26.10.0"),
                        )
                        .child(
                            Button::new("about-github-repo-btn")
                                .icon(IconName::Github)
                                .with_size(Size::Size(px(28.)))
                                .rounded(ButtonRounded::Size(px(6.)))
                                .outline()
                                .tooltip("GitHub Repository")
                                .on_click(|_, _, cx| {
                                    cx.open_url("https://github.com/sabisho/yomu");
                                }),
                        )
                        .child(
                            Button::new("about-github-profile-btn")
                                .icon(IconName::User)
                                .with_size(Size::Size(px(28.)))
                                .rounded(ButtonRounded::Size(px(6.)))
                                .outline()
                                .tooltip("sabisho on GitHub")
                                .on_click(|_, _, cx| {
                                    cx.open_url("https://github.com/sabisho");
                                }),
                        ),
                ),
            ),
        )
}

fn render_tab_appearance(
    settings: &Settings,
    available_schemes: &[String],
    colors: UiColors,
    on_set_theme_mode: std::rc::Rc<dyn Fn(ThemeModePreference, &mut Window, &mut App)>,
    on_set_color_scheme: std::rc::Rc<dyn Fn(String, &mut Window, &mut App)>,
    on_set_corner_rounding: std::rc::Rc<dyn Fn(CornerRounding, &mut Window, &mut App)>,
    on_set_border_size: std::rc::Rc<dyn Fn(BorderSize, &mut Window, &mut App)>,
) -> Div {
    let is_custom_standalone = is_standalone_scheme(&settings.color_scheme);
    let standalone_note = if is_custom_standalone {
        Some(
            div()
                .pt_2()
                .flex()
                .items_center()
                .gap_1p5()
                .child(
                    Icon::new(IconName::Info)
                        .size(px(13.))
                        .text_color(colors.muted),
                )
                .child(div().text_xs().text_color(colors.muted).child(format!(
                    "“{}” uses its own desktop-managed standalone palette.",
                    settings.color_scheme
                ))),
        )
    } else {
        None
    };

    let theme_mode_label = match settings.theme_mode {
        ThemeModePreference::System => "System",
        ThemeModePreference::Light => "Light",
        ThemeModePreference::Dark => "Dark",
    };

    div()
        .flex()
        .flex_col()
        .gap_4()
        // Section: Appearance Mode (System / Light / Dark)
        .child(
            settings_card(colors).child(
                div()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(200.))
                                    .flex()
                                    .flex_col()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(colors.foreground)
                                            .child("Appearance"),
                                    )
                                    .child(
                                        div().text_xs().text_color(colors.muted).child(
                                            "Match desktop system theme or lock Light / Dark.",
                                        ),
                                    ),
                            )
                            .child(macos_dropdown_button(
                                "theme-mode-dropdown",
                                theme_mode_label,
                                vec![
                                    (
                                        "System".to_string(),
                                        ThemeModePreference::System,
                                        settings.theme_mode == ThemeModePreference::System,
                                    ),
                                    (
                                        "Light".to_string(),
                                        ThemeModePreference::Light,
                                        settings.theme_mode == ThemeModePreference::Light,
                                    ),
                                    (
                                        "Dark".to_string(),
                                        ThemeModePreference::Dark,
                                        settings.theme_mode == ThemeModePreference::Dark,
                                    ),
                                ],
                                on_set_theme_mode,
                            )),
                    )
                    .children(standalone_note),
            ),
        )
        // Section: Theme & Palette
        .child(
            settings_card(colors)
                .child(
                    settings_row(
                        "Color Scheme",
                        "Switch between Catppuccin, Gruvbox, and custom desktop themes.",
                        colors,
                    )
                    .child(macos_dropdown_button(
                        "color-scheme-dropdown",
                        settings.color_scheme.as_str(),
                        available_schemes
                            .iter()
                            .map(|s| (s.clone(), s.clone(), s == &settings.color_scheme))
                            .collect(),
                        on_set_color_scheme,
                    )),
                )
                .child(settings_divider(colors))
                .child(
                    settings_row(
                        "Active Variant",
                        "Theme variant currently resolved and applied to the interface.",
                        colors,
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(colors.muted)
                            .child(settings.theme_name.clone()),
                    ),
                ),
        )
        // Section: Display Geometry
        .child(
            settings_card(colors)
                .child(
                    settings_row(
                        "Corner Rounding",
                        "Border curvature across cards, covers, and dialogs.",
                        colors,
                    )
                    .child(macos_dropdown_button(
                        "rounding-dropdown",
                        settings.corner_rounding.label(),
                        [
                            CornerRounding::None,
                            CornerRounding::Standard,
                            CornerRounding::Rounded,
                        ]
                        .into_iter()
                        .map(|r| (r.label().to_string(), r, r == settings.corner_rounding))
                        .collect(),
                        on_set_corner_rounding,
                    )),
                )
                .child(settings_divider(colors))
                .child(
                    settings_row(
                        "Card Border Thickness",
                        "Border outline width on cards and dialog panels.",
                        colors,
                    )
                    .child(macos_dropdown_button(
                        "border-dropdown",
                        settings.border_size.label(),
                        [BorderSize::None, BorderSize::Thin, BorderSize::Thick]
                            .into_iter()
                            .map(|b| (b.label().to_string(), b, b == settings.border_size))
                            .collect(),
                        on_set_border_size,
                    )),
                ),
        )
}

fn render_tab_reader(
    settings: &Settings,
    system_reduced_motion: bool,
    colors: UiColors,
    on_toggle_continuous: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_webcomics: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_disable_animations: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_remember_zoom: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_vim_mode: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Div {
    div().flex().flex_col().gap_4().child(
        settings_card(colors)
            .child(
                settings_row(
                    "Continuous Vertical Reading",
                    "Scroll through manga volumes vertically instead of single page turns.",
                    colors,
                )
                .child(
                    div().cursor_pointer().child(
                        Switch::new("continuous-mode-switch")
                            .accessibility_label("Continuous vertical reading mode")
                            .small()
                            .checked(settings.continuous)
                            .on_click(move |val, window, cx| {
                                on_toggle_continuous(*val, window, cx)
                            }),
                    ),
                ),
            )
            .child(settings_divider(colors))
            .child(
                settings_row(
                    "Webcomics Mode",
                    "Force continuous strip with zero gap between consecutive panels.",
                    colors,
                )
                .child(
                    div().cursor_pointer().child(
                        Switch::new("webcomics-mode-switch")
                            .accessibility_label("Webcomics zero-gap mode")
                            .small()
                            .checked(settings.webcomics)
                            .on_click(move |val, window, cx| on_toggle_webcomics(*val, window, cx)),
                    ),
                ),
            )
            .child(settings_divider(colors))
            .child(
                settings_row(
                    "Disable Animations",
                    if system_reduced_motion {
                        "All nonessential animations are disabled because your operating system's reduced-motion preference is on."
                    } else {
                        "Disable nonessential transitions, glides, and decorative motion across the application."
                    },
                    colors,
                )
                .child(
                    div()
                        .when(!system_reduced_motion, |s| s.cursor_pointer())
                        .child(
                            Switch::new("disable-animations-switch")
                                .accessibility_label("Disable animations")
                                .small()
                                .checked(if system_reduced_motion {
                                    true
                                } else {
                                    settings.disable_animations
                                })
                                .disabled(system_reduced_motion)
                                .on_click(move |val, window, cx| {
                                    on_toggle_disable_animations(*val, window, cx)
                                }),
                        ),
                ),
            )
            .child(settings_divider(colors))
            .child(
                settings_row(
                    "Remember Zoom Across Books",
                    "Retain reader zoom magnification across comic books and sessions.",
                    colors,
                )
                .child(
                    div().cursor_pointer().child(
                        Switch::new("remember-zoom-switch")
                            .accessibility_label("Remember per-book zoom")
                            .small()
                            .checked(settings.remember_zoom)
                            .on_click(move |val, window, cx| {
                                on_toggle_remember_zoom(*val, window, cx)
                            }),
                    ),
                ),
            )
            .child(settings_divider(colors))
            .child(
                settings_row(
                    "Vim Keyboard Navigation",
                    "Enable Vim modal motions (j/k, gg, G, :) in reader and library.",
                    colors,
                )
                .child(
                    div().cursor_pointer().child(
                        Switch::new("vim-mode-switch")
                            .accessibility_label("Vim keyboard navigation")
                            .small()
                            .checked(settings.vim_mode)
                            .on_click(move |val, window, cx| on_toggle_vim_mode(*val, window, cx)),
                    ),
                ),
            ),
    )
}

fn render_tab_library(
    settings: &Settings,
    colors: UiColors,
    on_toggle_group_series: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_set_thumbnail_size: std::rc::Rc<dyn Fn(ThumbnailSize, &mut Window, &mut App)>,
) -> Div {
    div().flex().flex_col().gap_4().child(
        settings_card(colors)
            .child(
                settings_row(
                    "Folder & Series Grouping",
                    "Group volumes sharing a common directory or series name into single cards.",
                    colors,
                )
                .child(
                    div().cursor_pointer().child(
                        Switch::new("group-series-switch")
                            .accessibility_label("Folder grouping")
                            .small()
                            .checked(settings.group_series)
                            .on_click(move |val, window, cx| {
                                on_toggle_group_series(*val, window, cx)
                            }),
                    ),
                ),
            )
            .child(settings_divider(colors))
            .child(
                settings_row(
                    "Thumbnail Size",
                    "Size of book covers in the library grid view.",
                    colors,
                )
                .child(macos_dropdown_button(
                    "cycle-thumb-btn-lib",
                    settings.thumbnail_size.label(),
                    ThumbnailSize::ALL
                        .into_iter()
                        .map(|s| (s.label().to_string(), s, s == settings.thumbnail_size))
                        .collect(),
                    on_set_thumbnail_size,
                )),
            ),
    )
}

fn render_tab_shortcuts(colors: UiColors, vim_mode: bool) -> Div {
    let search_sc = if cfg!(target_os = "macos") {
        if vim_mode {
            "⌘K or / or ⌘F"
        } else {
            "⌘K or ⌘F"
        }
    } else if vim_mode {
        "Ctrl+K or / or Ctrl+F"
    } else {
        "Ctrl+K or Ctrl+F"
    };
    let zoom_sc = zoom_shortcut_str();
    let settings_sc = settings_shortcut_str();
    let open_sc = open_shortcut_str();

    let mut shortcuts: Vec<(&'static str, String)> = Vec::new();
    shortcuts.push(("Continue / Open Manga", "Enter or Left Click".to_string()));
    shortcuts.push(("Import Manga Files / Folder", open_sc.to_string()));
    shortcuts.push(("Search Library", search_sc.to_string()));
    if vim_mode {
        shortcuts.push(("Command Prompt", ":".to_string()));
    }
    shortcuts.push(("Toggle Fullscreen", "F or F11".to_string()));
    if vim_mode {
        shortcuts.push((
            "Next Page / Scroll Down",
            "j or J or Space or Down or Right or PageDown".to_string(),
        ));
        shortcuts.push((
            "Previous Page / Scroll Up",
            "k or K or Shift+Space or Up or Left or PageUp".to_string(),
        ));
        shortcuts.push(("First Page / Cover", "gg or Home".to_string()));
        shortcuts.push(("Last Page", "G or End".to_string()));
        shortcuts.push(("Jump to Page", "{count}G or :{count}".to_string()));
        shortcuts.push(("Set / Jump to Mark", "m{a-z} / '{a-z}".to_string()));
    } else {
        shortcuts.push((
            "Next Page / Scroll Down",
            "Space or Down or Right or PageDown".to_string(),
        ));
        shortcuts.push((
            "Previous Page / Scroll Up",
            "Shift+Space or Up or Left or PageUp".to_string(),
        ));
        shortcuts.push(("First Page / Cover", "Home".to_string()));
        shortcuts.push(("Last Page", "End".to_string()));
    }
    shortcuts.push(("Toggle Continuous Vertical", "v".to_string()));
    shortcuts.push(("Toggle Webcomics Mode", "w".to_string()));
    if vim_mode {
        shortcuts.push(("Zoom In / Out", format!("{zoom_sc} or zi / zo")));
        shortcuts.push(("Reset Zoom", "0 or z0".to_string()));
    } else {
        shortcuts.push(("Zoom In / Out", zoom_sc.to_string()));
        shortcuts.push(("Reset Zoom", "0".to_string()));
    }
    shortcuts.push(("Settings", settings_sc.to_string()));
    if vim_mode {
        shortcuts.push(("Close / Back to Library", "Esc or q".to_string()));
    } else {
        shortcuts.push(("Close / Back to Library", "Esc".to_string()));
    }

    div()
        .w_full()
        .rounded(px(10.))
        .border_1()
        .border_color(with_alpha(colors.border, 0.45))
        .bg(with_alpha(colors.surface, 0.65))
        .p_4()
        .child(
            Table::new()
                .child(
                    TableHeader::new().child(
                        TableRow::new()
                            .child(TableHead::new().child("ACTION"))
                            .child(TableHead::new().child("KEYBOARD SHORTCUT")),
                    ),
                )
                .child(TableBody::new().children(shortcuts.into_iter().map(
                    |(action, shortcut)| {
                        TableRow::new()
                            .child(
                                TableCell::new()
                                    .child(action)
                                    .text_xs()
                                    .font_weight(FontWeight::NORMAL),
                            )
                            .child(
                                TableCell::new().child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded(px(4.))
                                        .bg(with_alpha(colors.foreground, 0.08))
                                        .border_1()
                                        .border_color(with_alpha(colors.border, 0.5))
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(colors.accent)
                                        .child(shortcut),
                                ),
                            )
                    },
                ))),
        )
}

fn render_tab_advanced(
    cache_size: &str,
    colors: UiColors,
    on_clear_cache: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_reset_settings: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_reset_all_data: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            settings_card(colors).child(
                settings_row(
                    "Cover Image Cache",
                    format!("Currently using {cache_size} of local disk storage."),
                    colors,
                )
                .child(
                    Button::new("clear-cache-btn")
                        .secondary()
                        .outline()
                        .label("Clear Cache")
                        .rounded(ButtonRounded::Size(px(6.)))
                        .small()
                        .on_click(on_clear_cache),
                ),
            ),
        )
        .child(
            settings_card(colors)
                .child(
                    settings_row(
                        "Reset Application Settings",
                        "Restores all appearance, theme, and reading preferences to factory defaults.",
                        colors,
                    )
                    .child(
                        Button::new("reset-settings-btn")
                            .secondary()
                            .outline()
                            .label("Reset Settings")
                            .rounded(ButtonRounded::Size(px(6.)))
                            .small()
                            .on_click(on_reset_settings),
                    ),
                )
                .child(settings_divider(colors))
                .child(
                    settings_row(
                        "Reset All Library Data",
                        "Removes all imported books, marks, and reading progress. Comic files on disk are never touched.",
                        colors,
                    )
                    .child(
                        Button::new("reset-all-data-btn")
                            .danger()
                            .outline()
                            .label("Reset All Data")
                            .rounded(ButtonRounded::Size(px(6.)))
                            .small()
                            .on_click(on_reset_all_data),
                    ),
                ),
        )
}
