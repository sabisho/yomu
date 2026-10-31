//! Overlay definitions and rendering: menus, dialogs, settings tabs, and modal states.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, ThemeConfig};
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, Animation, AnimationExt, AnyElement, App, Entity, FontWeight, Hsla, InteractiveElement,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, SharedString, Stateful,
    Styled, Window, anchored, deferred, div, ease_out_quint, px,
};
use yomu::menu::{BookAction, Entry, Item};

use crate::ui::components::{ButtonKind, overlay_shade, panel_button, with_alpha};
use crate::ui::theme::{BUILT_IN_THEMES, UiColors};

#[derive(Clone)]
pub struct BookMenu {
    pub book_id: String,
    pub target_ids: Vec<String>,
    pub entries: Vec<Entry>,
    pub position: Point<Pixels>,
    pub selected: Option<usize>,
}

#[allow(dead_code)]
pub enum ConfirmAction {
    Book(BookAction, String),
    RemoveBooks(Vec<String>),
    RemoveSelected,
    ResetSettings,
    ClearThumbnailCache,
    ClearAllData,
    DeleteCollection(String),
    ForceQuit,
}

pub struct ConfirmDialog {
    pub title: SharedString,
    pub message: SharedString,
    pub confirm_label: &'static str,
    pub action: ConfirmAction,
}

pub struct PropertiesDialog {
    pub title: SharedString,
    pub rows: Vec<(String, String)>,
}

pub struct SeriesDialog {
    pub book_ids: Vec<String>,
    pub input: Entity<InputState>,
}

pub struct CollectionDialog {
    pub is_new: bool,
    pub collection_id: Option<String>,
    pub input: Entity<InputState>,
}

#[derive(Default)]
pub struct LibraryMenu {
    pub selected: usize,
}

/// At most one menu or dialog is open at a time; it sits on top of whatever view is showing.
#[allow(dead_code)]
pub enum Overlay {
    Menu(BookMenu),
    LibraryMenu(LibraryMenu),
    Confirm(ConfirmDialog),
    Properties(PropertiesDialog),
    Series(SeriesDialog),
    Collection(CollectionDialog),
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum SettingsTab {
    #[default]
    General,
    Appearance,
    Reader,
    Library,
    Shortcuts,
    Advanced,
}

impl SettingsTab {
    #[allow(dead_code)]
    pub fn from_index(idx: usize) -> Self {
        match idx {
            0 => Self::General,
            1 => Self::Appearance,
            2 => Self::Reader,
            3 => Self::Library,
            4 => Self::Shortcuts,
            5 => Self::Advanced,
            _ => Self::General,
        }
    }

    #[allow(dead_code)]
    pub fn to_index(self) -> usize {
        match self {
            Self::General => 0,
            Self::Appearance => 1,
            Self::Reader => 2,
            Self::Library => 3,
            Self::Shortcuts => 4,
            Self::Advanced => 5,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Reader => "Reader",
            Self::Library => "Library",
            Self::Shortcuts => "Shortcuts",
            Self::Advanced => "Advanced",
        }
    }
}

/// Full-window layer behind a dialog: swallows clicks meant for the view underneath and
/// closes the dialog when clicked. `dim` darkens the view behind it.
pub fn scrim(
    dim: bool,
    colors: UiColors,
    on_close: impl Fn(&mut Window, &mut App) + Clone + 'static,
) -> Stateful<gpui_kit::Div> {
    let on_close_right = on_close.clone();
    div()
        .id("overlay-scrim")
        .occlude()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(with_alpha(
            overlay_shade(colors),
            if dim { 0.50 } else { 0.0 },
        ))
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            on_close(window, cx);
        })
        .on_mouse_down(MouseButton::Right, move |_, window, cx| {
            on_close_right(window, cx);
        })
}

pub fn panel(
    width: f32,
    border_px: f32,
    rounding_xl: f32,
    colors: UiColors,
    surface_bg: Hsla,
) -> Stateful<gpui_kit::Div> {
    div()
        .id(SharedString::from(format!("panel-{}", width as u32)))
        .occlude()
        .w(px(width))
        .max_w_full()
        .flex()
        .flex_col()
        .gap_4()
        .p_6()
        .rounded(px(rounding_xl))
        .bg(surface_bg)
        .when(border_px > 0.0, |this| {
            this.border(px(border_px)).border_color(colors.accent)
        })
        .shadow_lg()
        .text_sm()
        .text_color(colors.foreground)
        .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
        .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
}

pub fn animate_popup<E>(
    id: &'static str,
    enabled: bool,
    surface: E,
) -> gpui_kit::AnimationElement<E>
where
    E: IntoElement + Styled + 'static,
{
    surface.with_animation(
        id,
        Animation::new(Duration::from_millis(if !enabled { 1 } else { 180 }))
            .with_easing(ease_out_quint()),
        |surface, progress| {
            surface
                .opacity(progress)
                .relative()
                .top(px((1.0 - progress) * 9.0))
        },
    )
}

#[derive(Clone, Copy)]
pub struct OverlayStyle {
    pub colors: UiColors,
    pub border_px: f32,
    pub radius_px: f32,
    pub rounding_lg: f32,
    pub rounding_xl: f32,
    pub surface_bg: Hsla,
    pub animations_enabled: bool,
}

pub fn render_confirm_dialog(
    dialog: &ConfirmDialog,
    style: OverlayStyle,
    on_close: impl Fn(&mut Window, &mut App) + Clone + 'static,
    on_accept: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_cancel = on_close.clone();
    scrim(true, style.colors, on_close)
        .child(
            animate_popup(
                "confirm-dialog",
                style.animations_enabled,
                panel(
                    380.,
                    style.border_px,
                    style.rounding_xl,
                    style.colors,
                    style.surface_bg,
                ),
            )
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(dialog.title.clone()),
            )
            .child(
                div()
                    .text_color(style.colors.muted)
                    .child(dialog.message.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(panel_button(
                        "confirm-cancel",
                        "Cancel",
                        ButtonKind::Normal,
                        style.colors,
                        true,
                        move |_, window, cx| on_cancel(window, cx),
                    ))
                    .child(panel_button(
                        "confirm-accept",
                        dialog.confirm_label,
                        if matches!(
                            dialog.action,
                            ConfirmAction::ClearAllData | ConfirmAction::ForceQuit
                        ) {
                            ButtonKind::Danger
                        } else {
                            ButtonKind::Primary
                        },
                        style.colors,
                        true,
                        move |_, window, cx| on_accept(window, cx),
                    )),
            ),
        )
        .into_any_element()
}

pub fn render_properties_dialog(
    dialog: &PropertiesDialog,
    style: OverlayStyle,
    on_close: impl Fn(&mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let rows = dialog.rows.iter().map(|(label, value)| {
        div()
            .flex()
            .flex_row()
            .gap_4()
            .child(
                div()
                    .w(px(84.))
                    .flex_none()
                    .text_color(style.colors.muted)
                    .child(label.clone()),
            )
            .child(div().flex_1().min_w_0().truncate().child(value.clone()))
    });

    let on_close_btn = on_close.clone();
    scrim(true, style.colors, on_close)
        .child(
            animate_popup(
                "properties-dialog",
                style.animations_enabled,
                panel(
                    480.,
                    style.border_px,
                    style.rounding_xl,
                    style.colors,
                    style.surface_bg,
                ),
            )
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(dialog.title.clone()),
            )
            .child(div().flex().flex_col().gap_2().children(rows))
            .child(div().flex().flex_row().justify_end().child(panel_button(
                "properties-close",
                "Close",
                ButtonKind::Normal,
                style.colors,
                true,
                move |_, window, cx| on_close_btn(window, cx),
            ))),
        )
        .into_any_element()
}

pub fn render_series_dialog(
    dialog: &SeriesDialog,
    style: OverlayStyle,
    on_close: impl Fn(&mut Window, &mut App) + Clone + 'static,
    on_save: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_cancel = on_close.clone();
    scrim(true, style.colors, on_close)
        .child(
            animate_popup(
                "series-dialog",
                style.animations_enabled,
                panel(
                    480.,
                    style.border_px,
                    style.rounding_xl,
                    style.colors,
                    style.surface_bg,
                ),
            )
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(style.colors.foreground)
                    .child(if dialog.book_ids.len() == 1 {
                        "Set Series"
                    } else {
                        "Set Series for Selected Books"
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(style.colors.muted)
                    .child("Assign a series name to group these volumes."),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(style.colors.foreground)
                            .child("Series Name:"),
                    )
                    .child(
                        div()
                            .w_full()
                            .rounded(px(style.radius_px))
                            .border_1()
                            .border_color(style.colors.accent)
                            .child(
                                Input::new(&dialog.input)
                                    .id("series-name-input")
                                    .cleanable(true)
                                    .bordered(false)
                                    .focus_bordered(false)
                                    .w_full(),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(style.colors.muted)
                            .child("Enter to save · Esc to cancel"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .child(panel_button(
                                "series-cancel",
                                "Cancel",
                                ButtonKind::Normal,
                                style.colors,
                                true,
                                move |_, window, cx| on_cancel(window, cx),
                            ))
                            .child(panel_button(
                                "series-save",
                                "Save",
                                ButtonKind::Primary,
                                style.colors,
                                true,
                                move |_, window, cx| on_save(window, cx),
                            )),
                    ),
            ),
        )
        .into_any_element()
}

pub fn render_collection_dialog(
    dialog: &CollectionDialog,
    style: OverlayStyle,
    on_close: impl Fn(&mut Window, &mut App) + Clone + 'static,
    on_save: impl Fn(&str, Option<&str>, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let is_edit = !dialog.is_new;
    let editing_id = dialog.collection_id.clone();
    let input_state = dialog.input.clone();
    let on_cancel = on_close.clone();

    scrim(true, style.colors, on_close)
        .child(
            div()
                .w(px(380.))
                .rounded(px(style.rounding_xl))
                .when(style.border_px > 0.0, |this| {
                    this.border(px(style.border_px))
                        .border_color(style.colors.border)
                })
                .bg(style.colors.background)
                .p_5()
                .flex()
                .flex_col()
                .gap_4()
                .shadow_xl()
                .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                    cx.stop_propagation()
                })
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::BOLD)
                        .text_color(style.colors.foreground)
                        .child(if is_edit {
                            "Rename Collection"
                        } else {
                            "New Collection"
                        }),
                )
                .child(Input::new(&dialog.input))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("collection-dialog-cancel")
                                .label("Cancel")
                                .rounded(ButtonRounded::Medium)
                                .outline()
                                .on_click(move |_, window: &mut Window, cx: &mut App| {
                                    on_cancel(window, cx)
                                }),
                        )
                        .child(
                            Button::new("collection-dialog-save")
                                .primary()
                                .label(if is_edit { "Rename" } else { "Create" })
                                .rounded(ButtonRounded::Medium)
                                .bg(style.colors.accent)
                                .text_color(style.colors.accent_foreground)
                                .on_click(move |_, window: &mut Window, cx: &mut App| {
                                    let name = input_state.read(cx).value().to_string();
                                    if !name.trim().is_empty() {
                                        on_save(name.trim(), editing_id.as_deref(), window, cx);
                                    }
                                }),
                        ),
                ),
        )
        .into_any_element()
}

pub fn render_book_menu(
    menu: &BookMenu,
    style: OverlayStyle,
    on_close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + Clone + 'static,
    on_hover: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    on_action: impl Fn(BookAction, &str, &[String], &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let mut list = div()
        .min_w(px(200.))
        .max_w(px(320.))
        .w_auto()
        .flex()
        .flex_col()
        .p_1()
        .rounded(px(style.rounding_lg))
        .bg(style.surface_bg)
        .when(style.border_px > 0.0, |this| {
            this.border(px(style.border_px))
                .border_color(style.colors.accent)
        })
        .shadow_lg()
        .text_sm()
        .text_color(style.colors.foreground)
        .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
        .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        });

    for (ix, entry) in menu.entries.iter().enumerate() {
        list = match entry {
            Entry::Separator => list,
            Entry::Item(item) => list.child(render_book_menu_item(
                menu,
                ix,
                item,
                style.colors,
                style.radius_px,
                on_hover.clone(),
                on_action.clone(),
            )),
        };
    }

    #[cfg(test)]
    let list = list.debug_selector(|| "book-menu".into());

    let on_close_right = on_close.clone();
    deferred(
        div()
            .id("book-menu-scrim")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(MouseButton::Left, on_close)
            .on_mouse_down(MouseButton::Right, on_close_right)
            .child(
                anchored()
                    .position(menu.position)
                    .anchor(Anchor::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(list),
            ),
    )
    .with_priority(1)
    .into_any_element()
}

fn render_book_menu_item(
    menu: &BookMenu,
    ix: usize,
    item: &Item,
    colors: UiColors,
    rounding_md: f32,
    on_hover: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_action: impl Fn(BookAction, &str, &[String], &mut Window, &mut App) + 'static,
) -> AnyElement {
    let icon = match item.action {
        BookAction::Open => IconName::BookOpen,
        BookAction::StartOver | BookAction::ResetProgress => IconName::RotateCcw,
        BookAction::MarkRead => IconName::CircleCheck,
        BookAction::MarkUnread => IconName::CircleX,
        BookAction::RevealFile => IconName::FolderOpen,
        BookAction::Properties => IconName::Info,
        BookAction::SetSeries => IconName::Library,
        BookAction::Remove | BookAction::RemoveSelected => IconName::Trash,
        BookAction::ClearSelection => IconName::X,
    };
    let is_selected = menu.selected == Some(ix);
    let item_color = if !item.enabled {
        colors.muted
    } else if is_selected {
        colors.accent_foreground
    } else if item.danger {
        colors.danger
    } else {
        colors.foreground
    };
    let base = div()
        .id(SharedString::from(format!("menu-item-{ix}")))
        .h(px(28.))
        .px_3()
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(rounding_md))
        .child(
            div()
                .w(px(20.))
                .h(px(20.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(icon).size_4().text_color(item_color)),
        )
        .child(item.label.clone());

    if !item.enabled {
        return base
            .text_color(colors.muted)
            .opacity(0.6)
            .into_any_element();
    }

    let action = item.action;
    let book_id = menu.book_id.clone();
    let target_ids = menu.target_ids.clone();
    base.text_color(if is_selected {
        colors.accent_foreground
    } else if item.danger {
        colors.danger
    } else {
        colors.foreground
    })
    .cursor_pointer()
    .when(is_selected, |row| row.bg(colors.accent))
    .when(!is_selected, |row| {
        row.hover(|style| style.bg(colors.surface_hover))
    })
    .on_mouse_move(move |_, window, cx| on_hover(ix, window, cx))
    .on_click(move |_, window, cx| {
        on_action(action, &book_id, &target_ids, window, cx);
    })
    .into_any_element()
}

pub fn render_library_menu(
    menu: &LibraryMenu,
    group_series: bool,
    style: OverlayStyle,
    on_close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + Clone + 'static,
    on_hover: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    on_action: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let entries = [
        (
            IconName::FilePlus,
            "Add files",
            "Import CBZ, CBR and ZIP comics",
        ),
        (
            IconName::FolderPlus,
            "Add folder",
            "Import a folder of images",
        ),
        (
            IconName::Settings,
            "Settings",
            "Themes and reader preferences",
        ),
        (
            IconName::Folder,
            if group_series {
                "Show individual volumes"
            } else {
                "Group into folders"
            },
            if group_series {
                "Display books ungrouped"
            } else {
                "Group volumes by folder"
            },
        ),
        (IconName::LogOut, "Quit", "Close yomu"),
    ];
    let mut list = div()
        .min_w(px(240.))
        .max_w(px(360.))
        .w_auto()
        .flex()
        .flex_col()
        .gap_1()
        .p_2()
        .rounded(px(style.rounding_lg))
        .bg(style.surface_bg)
        .when(style.border_px > 0.0, |this| {
            this.border(px(style.border_px))
                .border_color(style.colors.accent)
        })
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
        .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        });

    for (index, (icon, label, description)) in entries.into_iter().enumerate() {
        let selected = menu.selected == index;
        let on_hover_item = on_hover.clone();
        let on_action_item = on_action.clone();
        list = list.child(
            div()
                .id(SharedString::from(format!("library-menu-{index}")))
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded(px(style.radius_px))
                .cursor_pointer()
                .when(selected, |row| row.bg(style.colors.accent))
                .when(!selected, |row| {
                    row.hover(|s| s.bg(style.colors.surface_hover))
                })
                .child(Icon::new(icon).size_4().text_color(if selected {
                    style.colors.accent_foreground
                } else {
                    style.colors.foreground
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_sm()
                                .text_color(if selected {
                                    style.colors.accent_foreground
                                } else {
                                    style.colors.foreground
                                })
                                .child(label),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(if selected {
                                    style.colors.accent_foreground
                                } else {
                                    style.colors.muted
                                })
                                .truncate()
                                .child(description),
                        ),
                )
                .on_mouse_move(move |_, window, cx| on_hover_item(index, window, cx))
                .on_click(move |_, window, cx| on_action_item(index, window, cx)),
        );
    }

    let on_close_right = on_close.clone();
    deferred(
        div()
            .id("library-menu-scrim")
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(MouseButton::Left, on_close)
            .on_mouse_down(MouseButton::Right, on_close_right)
            .child(
                div()
                    .absolute()
                    .top(px(52.))
                    .right(px(18.))
                    .child(animate_popup(
                        "library-menu-popup",
                        style.animations_enabled,
                        list,
                    )),
            ),
    )
    .with_priority(2)
    .into_any_element()
}

/// Suggestions for the reader's vim `:` command box:
/// returns (completed command text, display label, short explanation).
pub fn command_suggestions(
    input: &str,
    custom_themes: &HashMap<String, Rc<ThemeConfig>>,
) -> Vec<(String, String, &'static str)> {
    const COMMANDS: &[(&str, &str)] = &[
        ("fit", "Choose page fit mode"),
        ("zoom", "Set zoom percentage or reset"),
        ("animate", "Toggle page animations"),
        ("continuous", "Toggle continuous scrolling"),
        ("webcomics", "Toggle gapless webcomics mode"),
        ("remember-zoom", "Toggle retaining zoom across books"),
        ("theme", "Change color theme"),
        ("goto", "Jump to a page number"),
        ("mark", "Set a letter mark"),
        ("settings", "Open Settings"),
        ("info", "Toggle page information"),
        ("help", "Show keyboard help"),
        ("fullscreen", "Toggle fullscreen"),
        ("q", "Return to the library"),
        ("qa", "Quit yomu"),
    ];
    const FITS: &[(&str, &str)] = &[
        ("page", "Fit the whole page"),
        ("width", "Fit the page width"),
        ("height", "Fit the page height"),
        ("original", "Show at original size"),
    ];
    const TOGGLES: &[(&str, &str)] = &[("on", "Enable"), ("off", "Disable")];
    const ZOOM: &[(&str, &str)] = &[
        ("reset", "Reset zoom"),
        ("150", "Set zoom to 150% (edit the number as needed)"),
    ];

    let trimmed = input.trim_start();
    let mut words = trimmed.split_whitespace();
    let command = words.next().unwrap_or_default();
    let has_separator = trimmed.ends_with(char::is_whitespace);
    if let Some((_, _)) = COMMANDS.iter().find(|(name, _)| *name == command)
        && (has_separator || input.contains(' '))
    {
        if command == "theme" {
            let partial = trimmed
                .strip_prefix(command)
                .unwrap_or("")
                .trim_start()
                .to_lowercase();
            let mut themes: Vec<(String, &'static str)> = Vec::new();
            for (name, config) in &*BUILT_IN_THEMES {
                let desc = match config.mode {
                    gpui_kit::component::ThemeMode::Light => "Light theme",
                    gpui_kit::component::ThemeMode::Dark => "Dark theme",
                };
                themes.push((name.clone(), desc));
            }
            for (name, config) in custom_themes {
                if !BUILT_IN_THEMES.contains_key(name) {
                    let desc = match config.mode {
                        gpui_kit::component::ThemeMode::Light => "Custom light theme",
                        gpui_kit::component::ThemeMode::Dark => "Custom dark theme",
                    };
                    themes.push((name.clone(), desc));
                }
            }
            themes.sort_by_key(|a| a.0.to_lowercase());
            return themes
                .into_iter()
                .filter(|(name, _)| name.to_lowercase().starts_with(&partial))
                .map(|(name, desc)| (format!("theme {name}"), name, desc))
                .collect();
        }

        let options: &[(&str, &str)] = match command {
            "fit" => FITS,
            "animate" | "continuous" | "webcomics" | "remember-zoom" => TOGGLES,
            "zoom" => ZOOM,
            _ => &[],
        };
        let partial = trimmed
            .strip_prefix(command)
            .unwrap_or("")
            .trim_start()
            .to_lowercase();
        return options
            .iter()
            .filter(|(choice, _)| choice.starts_with(&partial))
            .map(|(choice, desc)| (format!("{command} {choice}"), choice.to_string(), *desc))
            .collect();
    }

    COMMANDS
        .iter()
        .filter(|(name, _)| name.starts_with(command))
        .map(|(name, desc)| {
            let space = if matches!(*name, "q" | "qa" | "help" | "settings" | "info") {
                ""
            } else {
                " "
            };
            (format!("{name}{space}"), name.to_string(), *desc)
        })
        .collect()
}

pub fn render_command_box(
    input: &str,
    command_selection: usize,
    custom_themes: &HashMap<String, Rc<ThemeConfig>>,
    style: OverlayStyle,
    on_close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + Clone + 'static,
    on_select: impl Fn(usize, &mut Window, &mut App) + Clone + 'static,
    on_submit: impl Fn(String, &mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let suggestions = command_suggestions(input, custom_themes);
    let border_px = style.border_px;
    let rounding_lg = style.rounding_lg;
    let rounding_md = style.radius_px;
    let inner_radius = if border_px > 0.0 {
        (rounding_lg - border_px).max(0.0)
    } else {
        rounding_lg
    };

    let mut prompt = div()
        .id("command-box-panel")
        .w(px(560.))
        .max_w(px(700.))
        .flex()
        .flex_col()
        .rounded(px(rounding_lg))
        .overflow_hidden()
        .when(border_px > 0.0, |this| {
            this.border(px(border_px)).border_color(style.colors.accent)
        })
        .bg(style.colors.surface)
        .shadow_xl()
        .text_sm()
        .text_color(style.colors.foreground)
        .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
        .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        });

    let mut input_row = div()
        .id("command-box-input")
        .flex()
        .items_center()
        .gap_3()
        .px_4()
        .py_3()
        .rounded_t(px(inner_radius))
        .when(suggestions.is_empty(), |this| {
            this.rounded_b(px(inner_radius))
        })
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(style.colors.accent)
                .text_base()
                .child(":"),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(style.colors.foreground)
                .child(format!("{input}▏")),
        )
        .child(
            div()
                .text_xs()
                .text_color(style.colors.muted)
                .child("↑↓ select · Tab complete · Enter run · Esc cancel"),
        );

    if !suggestions.is_empty() {
        input_row = input_row.border_b_1().border_color(style.colors.border);
    }
    prompt = prompt.child(input_row);

    if !suggestions.is_empty() {
        let mut rows = div()
            .id("command-box-suggestions")
            .p_1()
            .rounded_b(px(inner_radius))
            .max_h(px(320.))
            .overflow_y_scrollbar()
            .scroll_bounce(true)
            .flex()
            .flex_col();

        let item_radius = rounding_md.min(inner_radius);
        for (index, (completion, label, description)) in suggestions.iter().enumerate() {
            let selected = index == command_selection;
            let completion = completion.clone();
            let on_select_item = on_select.clone();
            let on_submit_item = on_submit.clone();
            rows = rows.child(
                div()
                    .id(SharedString::from(format!("command-suggestion-{index}")))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .px_3()
                    .py(px(6.))
                    .rounded(px(item_radius))
                    .cursor_pointer()
                    .when(border_px > 0.0 || selected, |this| {
                        this.border(px(border_px.max(1.0)))
                            .border_color(if selected {
                                with_alpha(style.colors.accent, 0.70)
                            } else {
                                with_alpha(style.colors.accent, 0.0)
                            })
                    })
                    .when(selected, |row| {
                        row.bg(with_alpha(style.colors.accent, 0.20))
                    })
                    .hover(|row| row.bg(with_alpha(style.colors.accent, 0.15)))
                    .child(
                        div()
                            .font_weight(if selected {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if selected {
                                style.colors.accent
                            } else {
                                style.colors.foreground
                            })
                            .child(label.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_right()
                            .text_xs()
                            .text_color(style.colors.muted)
                            .truncate()
                            .child(*description),
                    )
                    .on_mouse_move(move |_, window, cx| on_select_item(index, window, cx))
                    .on_click(move |_, window, cx| on_submit_item(completion.clone(), window, cx)),
            );
        }
        prompt = prompt.child(rows);
    }

    let on_close_right = on_close.clone();
    let scrim = div()
        .id("command-box-scrim")
        .occlude()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(with_alpha(overlay_shade(style.colors), 0.40))
        .on_mouse_down(MouseButton::Left, on_close)
        .on_mouse_down(MouseButton::Right, on_close_right)
        .child(animate_popup(
            "command-prompt-popup",
            style.animations_enabled,
            prompt,
        ));

    deferred(scrim).with_priority(3).into_any_element()
}

pub const HELP: &[(&str, &[(&str, &str)])] = &[
    (
        "Navigation",
        &[
            (
                "j, k, h, l / Arrows",
                "Scroll view by step; turns page at boundary.",
            ),
            ("Space / Shift+Space", "Scroll one viewport down or up."),
            ("Ctrl+D / Ctrl+U", "Scroll half a viewport down or up."),
            ("J, K / PgDn, PgUp", "Navigate to next or previous page."),
            ("gg / G / [N]G", "Jump to first, last, or page N."),
            ("m[a] / '[a] / ''", "Set mark, jump to mark, or jump back."),
        ],
    ),
    (
        "Display & Zoom",
        &[
            (
                "a / s / zh / z1",
                "Fit to page, width, height, or original size.",
            ),
            ("Tab", "Cycle through available fit modes."),
            ("+ / - / 0 / zi / zo", "Zoom in, zoom out, or reset zoom."),
            (
                "Ctrl+Wheel / Pinch / Drag",
                "Zoom at cursor position, or drag to pan.",
            ),
            ("zt / zz / zb", "Align view to top, center, or bottom."),
            (
                "v / w / i / f",
                "Toggle continuous, webcomics, info, or fullscreen.",
            ),
        ],
    ),
    (
        "Commands (:)",
        &[
            (
                ":fit [mode]",
                "Set fit mode (page, width, height, original).",
            ),
            (":zoom [pct]", "Set zoom percentage, or reset to fit."),
            (":q / :qa", "Close current comic, or quit application."),
            (
                ":settings / Ctrl+,",
                "Open settings and preferences dialog.",
            ),
            (
                ":animate [on|off]",
                "Enable or disable smooth page animations.",
            ),
            (
                ":continuous [on|off]",
                "Toggle stacked continuous reading mode.",
            ),
            (
                ":webcomics [on|off]",
                "Toggle gapless webcomics strip mode.",
            ),
            (":theme [name]", "Change color theme or cycle themes."),
        ],
    ),
];

pub fn help_overlay(colors: UiColors, border_px: f32, rounding_xl: f32) -> AnyElement {
    let mut panel = div()
        .w(px(600.))
        .max_w_full()
        .flex()
        .flex_col()
        .gap_3()
        .p_6()
        .rounded(px(rounding_xl))
        .when(border_px > 0.0, |this| {
            this.border(px(border_px)).border_color(colors.border)
        })
        .bg(colors.surface)
        .shadow_lg()
        .text_sm();

    for (title, rows) in HELP {
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.muted)
                        .child(*title),
                )
                .children(rows.iter().map(|(keys, what)| {
                    div()
                        .flex()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .gap_4()
                        .child(div().text_color(colors.foreground).child(*what))
                        .child(div().flex_none().text_color(colors.accent).child(*keys))
                })),
        );
    }
    panel = panel.child(
        div()
            .text_xs()
            .text_color(colors.muted)
            .child("Press ? or Esc to close"),
    );

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(with_alpha(
            overlay_shade(colors),
            (0.5 + colors.widget_opacity * 0.15).min(0.65),
        ))
        .child(panel)
        .into_any_element()
}
