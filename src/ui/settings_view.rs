//! Split-pane Settings view matching macOS Human Interface Guidelines.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::label::Label;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::sidebar::{Sidebar, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Icon, Side, Sizable, Size, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, AnyElement, App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement, Pixels,
    ScrollHandle, SharedString, Styled, Window, div, img, px,
};

use super::components::{
    CONTROL_HEIGHT_TOOLBAR, open_shortcut_str, settings_shortcut_str, with_alpha, zoom_shortcut_str,
};
use super::overlays::SettingsTab;
use super::theme::{UiColors, is_standalone_scheme};
use crate::library::{
    BorderSize, CornerRounding, PageLayout, ReadingDirection, Settings, ThemeModePreference,
    ThumbnailSize,
};

pub struct SettingsViewProps<'a> {
    pub scroll_handle: &'a ScrollHandle,
    pub current_tab: SettingsTab,
    pub settings: &'a Settings,
    pub available_schemes: Vec<String>,
    pub cache_size_str: String,
    pub vim_mode: bool,
    pub colors: UiColors,
    pub radius: Pixels,
}

pub fn render_settings_view(
    props: SettingsViewProps,
    on_select_tab: impl Fn(SettingsTab, &mut Window, &mut App) + 'static,
    on_set_page_layout: impl Fn(PageLayout, &mut Window, &mut App) + 'static,
    on_set_reading_direction: impl Fn(ReadingDirection, &mut Window, &mut App) + 'static,
    on_toggle_first_page_is_cover: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_set_spread_ratio_threshold: impl Fn(f32, &mut Window, &mut App) + 'static,
    on_toggle_continuous: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_webcomics: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_group_series: impl Fn(bool, &mut Window, &mut App) + 'static,
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
    let on_set_page_layout = std::rc::Rc::new(on_set_page_layout);
    let on_set_reading_direction = std::rc::Rc::new(on_set_reading_direction);
    let on_set_theme_mode = std::rc::Rc::new(on_set_theme_mode);
    let on_set_color_scheme = std::rc::Rc::new(on_set_color_scheme);
    let on_set_thumbnail_size = std::rc::Rc::new(on_set_thumbnail_size);
    let on_set_corner_rounding = std::rc::Rc::new(on_set_corner_rounding);
    let on_set_border_size = std::rc::Rc::new(on_set_border_size);
    let colors = props.colors;

    let tabs = [
        (SettingsTab::General, IconName::Settings),
        (SettingsTab::Appearance, IconName::Palette),
        (SettingsTab::Reader, IconName::BookOpen),
        (SettingsTab::Shortcuts, IconName::Keyboard),
        (SettingsTab::Advanced, IconName::Database),
        (SettingsTab::About, IconName::Info),
    ];

    div()
        .size_full()
        .rounded(props.radius)
        .flex()
        .flex_row()
        .items_stretch()
        .bg(colors.background)
        // Left Native Settings Navigation Sidebar
        .child(
            Sidebar::new("settings-sidebar")
                .rounded_l(props.radius)
                .border_l_1()
                .border_t_1()
                .border_b_1()
                .border_color(colors.border)
                .side(Side::Left)
                .w(px(200.))
                .h_full()
                .collapsible(false)
                .collapsed(false)
                .header(
                    h_flex().w_full().h(px(28.)).items_center().px_1().child(
                        Label::new("Preferences")
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colors.foreground),
                    ),
                )
                .child(SidebarMenu::new().children(tabs.iter().map(|&(tab, icon)| {
                    let on_select_tab = on_select_tab.clone();
                    let is_active = props.current_tab == tab;
                    SidebarMenuItem::new(tab.label())
                        .icon(Icon::new(icon).size(px(16.)))
                        .active(is_active)
                        .on_click(move |_, window, cx| on_select_tab(tab, window, cx))
                }))),
        )
        // Right Settings Content Pane
        .child(
            div()
                .id("settings-content-scroll")
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scrollbar()
                .track_scroll(props.scroll_handle)
                .scroll_bounce(true)
                .scroll_wheel(false)
                .pl_6()
                .pr_10()
                .pt_4()
                .pb_6()
                .flex()
                .flex_col()
                .gap_4()
                .child({
                    let tab_content = match props.current_tab {
                        SettingsTab::General => render_tab_general(
                            props.settings,
                            colors,
                            on_toggle_continuous,
                            on_toggle_group_series,
                            on_set_thumbnail_size,
                        ),
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
                            colors,
                            on_set_page_layout,
                            on_set_reading_direction,
                            on_toggle_first_page_is_cover,
                            on_set_spread_ratio_threshold,
                            on_toggle_continuous,
                            on_toggle_webcomics,
                            on_toggle_remember_zoom,
                            on_toggle_vim_mode,
                        ),
                        SettingsTab::Shortcuts => render_tab_shortcuts(colors, props.vim_mode),
                        SettingsTab::Advanced => render_tab_advanced(
                            &props.cache_size_str,
                            colors,
                            on_clear_cache,
                            on_reset_settings,
                            on_reset_all_data,
                        ),
                        SettingsTab::About => render_tab_about(colors),
                    };

                    let duration = std::time::Duration::from_millis(140);
                    gpui_kit::base::animation::EffectTransition::new(duration)
                        .ease(gpui_kit::base::animation::ease_out_cubic)
                        .fade(0.85, 1.0)
                        .apply(
                            tab_content,
                            ("settings-tab-fade", props.current_tab.to_index()),
                        )
                }),
        )
        .into_any_element()
}

fn settings_card(colors: UiColors) -> GroupBox {
    GroupBox::new()
        .outline()
        .rounded(px(8.))
        .bg(with_alpha(colors.surface, 0.65))
        .border_color(colors.border)
}

fn settings_divider(colors: UiColors) -> Separator {
    Separator::horizontal().color(colors.border)
}

fn settings_row(
    title: &'static str,
    description: impl Into<SharedString>,
    colors: UiColors,
) -> Div {
    div()
        .w_full()
        .min_h(px(40.))
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
                    Label::new(title)
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors.foreground),
                )
                .child(Label::new(description).text_xs().text_color(colors.muted)),
        )
}

pub fn macos_dropdown_button<T: Clone + PartialEq + 'static>(
    id: impl Into<ElementId>,
    label: &str,
    options: Vec<(String, T, bool)>,
    on_select: std::rc::Rc<dyn Fn(T, &mut Window, &mut App)>,
) -> AnyElement {
    Button::new(id)
        .with_size(Size::Size(CONTROL_HEIGHT_TOOLBAR))
        .outline()
        .dropdown_caret(true)
        .rounded(ButtonRounded::Size(px(6.)))
        .label(label.to_string())
        .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
            menu = menu
                .check_side(Side::Left)
                .min_w(px(160.))
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
    on_toggle_group_series: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_set_thumbnail_size: std::rc::Rc<dyn Fn(ThumbnailSize, &mut Window, &mut App)>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            settings_card(colors)
                .child(
                    settings_row(
                        "Default Reading Layout",
                        "Toggle between single-page book turns and continuous vertical strip.",
                        colors,
                    )
                    .child(
                        Switch::new("gen-continuous-mode-switch")
                            .accessibility_label("Continuous vertical reading mode")
                            .small()
                            .checked(settings.continuous)
                            .on_click(move |val, window, cx| on_toggle_continuous(*val, window, cx)),
                    ),
                )
                .child(settings_divider(colors))
                .child(
                    settings_row(
                        "Folder & Series Grouping",
                        "Group volumes sharing a common directory or series name into single cards.",
                        colors,
                    )
                    .child(
                        Switch::new("group-series-switch")
                            .accessibility_label("Folder grouping")
                            .small()
                            .checked(settings.group_series)
                            .on_click(move |val, window, cx| on_toggle_group_series(*val, window, cx)),
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
                        .child(Tag::secondary().child("v26.10.0"))
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
                .px_4()
                .pb_2()
                .flex()
                .items_center()
                .gap_1p5()
                .child(
                    Icon::new(IconName::Info)
                        .size(px(13.))
                        .text_color(colors.muted),
                )
                .child(
                    Label::new(format!(
                        "“{}” uses its own desktop-managed standalone palette.",
                        settings.color_scheme
                    ))
                    .text_xs()
                    .text_color(colors.muted),
                ),
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
        // Consolidated Theme Section: Appearance Mode + Color Scheme
        .child(
            settings_card(colors)
                .child(
                    settings_row(
                        "Appearance Mode",
                        "Match desktop system theme or lock Light / Dark.",
                        colors,
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
                .child(settings_divider(colors))
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
                .children(standalone_note),
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
    colors: UiColors,
    on_set_page_layout: std::rc::Rc<dyn Fn(PageLayout, &mut Window, &mut App)>,
    on_set_reading_direction: std::rc::Rc<dyn Fn(ReadingDirection, &mut Window, &mut App)>,
    on_toggle_first_page_is_cover: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_set_spread_ratio_threshold: impl Fn(f32, &mut Window, &mut App) + 'static,
    on_toggle_continuous: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_webcomics: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_remember_zoom: impl Fn(bool, &mut Window, &mut App) + 'static,
    on_toggle_vim_mode: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Div {
    let on_set_spread_ratio = std::rc::Rc::new(on_set_spread_ratio_threshold);
    div().flex().flex_col().gap_4().child(
        settings_card(colors)
            .child(
                settings_row(
                    "Reading Layout",
                    "Choose between single-page display and dual-page side-by-side spreads.",
                    colors,
                )
                .child(macos_dropdown_button(
                    "layout-dropdown-settings",
                    settings.page_layout.label(),
                    PageLayout::ALL
                        .into_iter()
                        .map(|l| (l.label().to_string(), l, l == settings.page_layout))
                        .collect(),
                    on_set_page_layout,
                )),
            )
            .child(settings_divider(colors))
            .child(
                settings_row(
                    "Reading Direction",
                    "Right-to-left for standard manga; left-to-right for western comics.",
                    colors,
                )
                .child(macos_dropdown_button(
                    "direction-dropdown-settings",
                    settings.reading_direction.label(),
                    ReadingDirection::ALL
                        .into_iter()
                        .map(|d| (d.label().to_string(), d, d == settings.reading_direction))
                        .collect(),
                    on_set_reading_direction,
                )),
            )
            .child(settings_divider(colors))
            .child(
                settings_row(
                    "First Page as Cover",
                    "Display the first page as an isolated single cover spread before pairing begins.",
                    colors,
                )
                .child(
                    Switch::new("first-page-cover-switch")
                        .accessibility_label("First page as cover")
                        .small()
                        .checked(settings.first_page_is_cover)
                        .on_click(move |val, window, cx| {
                            on_toggle_first_page_is_cover(*val, window, cx)
                        }),
                ),
            )
            .child(settings_divider(colors))
            .child(
                settings_row(
                    "Spread Detection Ratio",
                    "Aspect ratio threshold (W/H) to automatically detect single-image spreads.",
                    colors,
                )
                .child({
                    let ratio_label = if (settings.spread_ratio_threshold - 1.25f32).abs() < 1e-3 {
                        "1.25 (Default)".to_string()
                    } else {
                        format!("{:.2}", settings.spread_ratio_threshold)
                    };
                    macos_dropdown_button(
                        "spread-ratio-dropdown-settings",
                        &ratio_label,
                        [1.15f32, 1.20f32, 1.25f32, 1.30f32, 1.40f32]
                            .into_iter()
                            .map(|r| {
                                (
                                    if (r - 1.25f32).abs() < 1e-3 {
                                        format!("{r:.2} (Default)")
                                    } else {
                                        format!("{r:.2}")
                                    },
                                    r,
                                    (r - settings.spread_ratio_threshold).abs() < 1e-3,
                                )
                            })
                            .collect(),
                        on_set_spread_ratio,
                    )
                }),
            ),
    )
    .child(
        settings_card(colors)
            .child(
                settings_row(
                    "Continuous Vertical Reading",
                    "Scroll through manga volumes vertically instead of single page turns.",
                    colors,
                )
                .child(
                    Switch::new("continuous-mode-switch")
                        .accessibility_label("Continuous vertical reading mode")
                        .small()
                        .checked(settings.continuous)
                        .on_click(move |val, window, cx| {
                            on_toggle_continuous(*val, window, cx)
                        }),
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
                    Switch::new("webcomics-mode-switch")
                        .accessibility_label("Webcomics zero-gap mode")
                        .small()
                        .checked(settings.webcomics)
                        .on_click(move |val, window, cx| on_toggle_webcomics(*val, window, cx)),
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
                    Switch::new("remember-zoom-switch")
                        .accessibility_label("Remember per-book zoom")
                        .small()
                        .checked(settings.remember_zoom)
                        .on_click(move |val, window, cx| {
                            on_toggle_remember_zoom(*val, window, cx)
                        }),
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
                    Switch::new("vim-mode-switch")
                        .accessibility_label("Vim keyboard navigation")
                        .small()
                        .checked(settings.vim_mode)
                        .on_click(move |val, window, cx| on_toggle_vim_mode(*val, window, cx)),
                ),
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

    let mut shortcuts: Vec<(&'static str, String)> = vec![
        ("Continue / Open Manga", "Enter or Left Click".to_string()),
        ("Import Manga Files / Folder", open_sc.to_string()),
        ("Search Library", search_sc.to_string()),
        ("Toggle Fullscreen", "F or F11".to_string()),
    ];
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
        shortcuts.push(("Jump to Page", "{count}G".to_string()));
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
    shortcuts.push(("Toggle Double Page", "d".to_string()));
    shortcuts.push(("Cycle Spread Override", "s".to_string()));
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

    div().w_full().child(
        settings_card(colors).child(
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
                            .child(TableCell::new().child(Tag::secondary().child(shortcut)))
                    },
                ))),
        ),
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

fn tech_col(label: &'static str, value: impl Into<SharedString>, colors: UiColors) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            Label::new(label)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.muted),
        )
        .child(
            Label::new(value)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.foreground),
        )
}

fn about_action_row(label: &'static str, action: impl IntoElement, colors: UiColors) -> Div {
    div()
        .w_full()
        .min_h(px(32.))
        .px_4()
        .py_1()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
            Label::new(label)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.muted),
        )
        .child(action)
}

fn render_tab_about(colors: UiColors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        // 1. Application Identity
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .child(
                    div()
                        .size(px(48.))
                        .rounded(px(10.))
                        .overflow_hidden()
                        .child(
                            img(crate::app::desktop::app_logo_image())
                                .size_full()
                                .object_fit(gpui_kit::ObjectFit::Contain),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Label::new("Yomu")
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(colors.foreground),
                        )
                        .child(Tag::secondary().child(format!("v{}", env!("CARGO_PKG_VERSION")))),
                )
                .child(
                    Label::new("Manga & Comic Reader")
                        .text_xs()
                        .text_color(colors.muted),
                ),
        )
        // 2. Subtle Divider
        .child(settings_divider(colors))
        // 3, 4 & 5. Compact Technology Information, Developer Links & License
        .child(
            settings_card(colors)
                .child(
                    div()
                        .w_full()
                        .min_h(px(28.))
                        .px_4()
                        .py_1()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_4()
                        .child(tech_col("Programming Language", "Rust", colors))
                        .child(Separator::vertical().color(colors.border))
                        .child(tech_col("UI Framework", "GPUI", colors)),
                )
                .child(settings_divider(colors))
                .child(
                    div()
                        .w_full()
                        .min_h(px(28.))
                        .px_4()
                        .py_1()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_4()
                        .child(tech_col("Component Toolkit", "GPUI Kit", colors))
                        .child(Separator::vertical().color(colors.border))
                        .child(tech_col(
                            "Application Type",
                            "Native Desktop Application",
                            colors,
                        )),
                )
                .child(settings_divider(colors))
                .child(about_action_row(
                    "Developer",
                    Button::new("about-author-link-btn")
                        .secondary()
                        .outline()
                        .icon(IconName::Github)
                        .label("sabisho")
                        .small()
                        .rounded(ButtonRounded::Size(px(6.)))
                        .tooltip("Open GitHub profile")
                        .on_click(|_, _, cx| {
                            cx.open_url("https://github.com/sabisho");
                        }),
                    colors,
                ))
                .child(settings_divider(colors))
                .child(about_action_row(
                    "Repository",
                    Button::new("about-repo-link-btn")
                        .secondary()
                        .outline()
                        .icon(IconName::ExternalLink)
                        .label("sabisho/yomu")
                        .small()
                        .rounded(ButtonRounded::Size(px(6.)))
                        .tooltip("Open GitHub repository")
                        .on_click(|_, _, cx| {
                            cx.open_url("https://github.com/sabisho/yomu");
                        }),
                    colors,
                ))
                .child(settings_divider(colors))
                .child(about_action_row(
                    "License",
                    Tag::secondary().child("MIT License"),
                    colors,
                )),
        )
}
