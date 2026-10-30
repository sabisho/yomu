//! Dedicated Manga Details view matching the macOS reference design.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, FontWeight, InteractiveElement, Pixels, ScrollHandle,
    SharedString, Styled, Window, div, img, px,
};
use yomu::archive::SourceKind;
use yomu::library::Book;

use super::components::{genre_tag, with_alpha};
use super::theme::UiColors;

pub struct MangaDetailsProps<'a> {
    pub scroll_handle: &'a ScrollHandle,
    pub book: &'a Book,
    pub cover_path: PathBuf,
    pub selected_tab: usize,
    pub synopsis_expanded: bool,
    pub colors: UiColors,
    pub radius: Pixels,
}

#[allow(clippy::too_many_arguments)]
pub fn render_manga_details(
    props: MangaDetailsProps,
    on_back: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_continue_reading: impl Fn(&str, &mut Window, &mut App) + 'static,
    on_select_tab: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_toggle_synopsis: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_more_menu: impl Fn(&str, (f32, f32), &mut Window, &mut App) + 'static,
) -> AnyElement {
    let colors = props.colors;
    let book = props.book;
    let book_id = book.id.clone();
    let book_id_clone = book_id.clone();

    let display_title = book.title.as_str();
    let subtitle = book.series.as_deref();

    let status_str = if book.is_finished() {
        "Completed"
    } else if book.opened {
        "Reading"
    } else {
        "Unread"
    };

    let kind_str = match book.kind {
        SourceKind::Archive => "Archive",
        SourceKind::Folder => "Folder",
    };

    let metadata_line = format!("Local · {kind_str} · {status_str}");

    let genres = if let Some(series) = book.series.as_ref() {
        vec![series.clone(), kind_str.to_string()]
    } else {
        vec!["Manga".to_string(), kind_str.to_string()]
    };

    let synopsis = if let Some(series) = book.series.as_ref() {
        format!("Part of the {series} series. {} pages.", book.page_count)
    } else {
        format!("Imported local comic book. {} pages.", book.page_count)
    };

    let current_pos = if book.opened {
        (book.current_page + 1).min(book.page_count)
    } else {
        0
    };
    let progress_val = book.progress();

    div()
        .id("manga-details-container")
        .size_full()
        .overflow_y_scrollbar()
        .track_scroll(props.scroll_handle)
        .scroll_bounce(true)
        .scroll_wheel(false)
        .p_6()
        .flex()
        .flex_col()
        .gap_6()
        // Top Back Navigation
        .child(
            div().flex().items_center().gap_2().child(
                Button::new("details-back-btn")
                    .ghost()
                    .small()
                    .label("← Back to Library")
                    .on_click(on_back),
            ),
        )
        // Hero section: Left cover, right rich information
        .child(
            div()
                .flex()
                .flex_row()
                .gap_8()
                .child(
                    // Left: Cover Preview Card
                    div()
                        .w(px(200.))
                        .h(px(290.))
                        .flex_none()
                        .rounded(props.radius)
                        .overflow_hidden()
                        .shadow_lg()
                        .border_1()
                        .border_color(with_alpha(colors.border, 0.6))
                        .bg(with_alpha(colors.surface, 0.4))
                        .child(if props.cover_path.exists() {
                            img(props.cover_path.clone()).size_full().into_any_element()
                        } else {
                            div()
                                .size_full()
                                .bg(with_alpha(colors.foreground, 0.04))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(IconName::BookOpen)
                                        .size(px(48.))
                                        .text_color(colors.muted),
                                )
                                .into_any_element()
                        }),
                )
                .child(
                    // Right: Title, Metadata, Synopsis, Action Buttons
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .text_2xl()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(colors.foreground)
                                        .child(display_title.to_string()),
                                )
                                .children(subtitle.map(|sub| {
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::NORMAL)
                                        .text_color(colors.muted)
                                        .child(sub.to_string())
                                }))
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(colors.muted)
                                        .child(metadata_line),
                                )
                                .child(
                                    div().flex().flex_wrap().gap_1p5().pt_1().children(
                                        genres
                                            .iter()
                                            .map(|g| genre_tag(g.as_str(), colors, props.radius)),
                                    ),
                                )
                                .child(
                                    div()
                                        .pt_2()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .line_height(px(18.))
                                                .text_color(colors.foreground)
                                                .when(!props.synopsis_expanded, |s| s.line_clamp(3))
                                                .child(synopsis),
                                        )
                                        .child(
                                            div()
                                                .id("toggle-synopsis-btn")
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(colors.accent)
                                                .cursor_pointer()
                                                .on_click(on_toggle_synopsis)
                                                .child(if props.synopsis_expanded {
                                                    "Read less ▴"
                                                } else {
                                                    "Read more ⌵"
                                                }),
                                        ),
                                ),
                        )
                        // Action buttons row
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .pt_2()
                                .child(
                                    Button::new("details-continue-btn")
                                        .primary()
                                        .label("Continue Reading")
                                        .rounded(ButtonRounded::Medium)
                                        .bg(colors.accent)
                                        .text_color(colors.accent_foreground)
                                        .tooltip("Start or resume reading")
                                        .on_click(move |_, window, cx| {
                                            on_continue_reading(&book_id, window, cx)
                                        }),
                                )
                                .child(
                                    div()
                                        .id("details-more-menu-btn")
                                        .size(px(32.))
                                        .rounded(props.radius)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .bg(with_alpha(colors.foreground, 0.06))
                                        .cursor_pointer()
                                        .tooltip(|window, cx| {
                                            Tooltip::new("More options").build(window, cx)
                                        })
                                        .on_click(move |event: &ClickEvent, window, cx| {
                                            let pos: (f32, f32) = (
                                                event.position().x.into(),
                                                event.position().y.into(),
                                            );
                                            on_more_menu(&book_id_clone, pos, window, cx);
                                        })
                                        .child(
                                            Icon::new(IconName::Ellipsis)
                                                .size(px(16.))
                                                .text_color(colors.foreground),
                                        ),
                                ),
                        ),
                ),
        )
        // Underline Tab Bar
        .child(
            TabBar::new("manga-details-tabbar")
                .underline()
                .selected_index(props.selected_tab)
                .on_click(move |idx, window, cx| on_select_tab(*idx, window, cx))
                .child(Tab::new().label("Overview"))
                .child(Tab::new().label("Info")),
        )
        // Tab Content: Overview Metadata Grid (4 cols x 1 row)
        .child(
            div()
                .w_full()
                .p_4()
                .rounded(props.radius)
                .bg(with_alpha(colors.surface, 0.5))
                .border_1()
                .border_color(colors.border)
                .child(
                    div()
                        .w_full()
                        .grid()
                        .grid_cols(4u16)
                        .gap_4()
                        .child(metadata_cell(
                            "Pages",
                            format!("{}", book.page_count),
                            colors,
                        ))
                        .child(metadata_cell("Status", status_str, colors))
                        .child(metadata_cell(
                            "Series",
                            book.series.as_deref().unwrap_or("None"),
                            colors,
                        ))
                        .child(metadata_cell("Format", kind_str, colors)),
                ),
        )
        // Bottom Progress Card (Local Progress & Pages Read)
        .child(
            div()
                .w_full()
                .p_4()
                .rounded(props.radius)
                .bg(with_alpha(colors.surface, 0.6))
                .border_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .justify_between()
                .gap_6()
                // Left: Progress
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(colors.foreground)
                                        .child("My Progress"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(colors.muted)
                                        .child(format!("{:.0}%", progress_val * 100.0)),
                                ),
                        )
                        .child(Progress::new("details-progress").value(progress_val * 100.0))
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.muted)
                                .child(format!("{current_pos} / {}", book.page_count)),
                        ),
                )
                // Right: Pages Read
                .child(
                    div().flex().items_center().gap_3().child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(colors.foreground)
                                    .child("Pages Read"),
                            )
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(colors.foreground)
                                    .child(current_pos.to_string()),
                            ),
                    ),
                ),
        )
        .into_any_element()
}

fn metadata_cell(label: &'static str, value: impl Into<SharedString>, colors: UiColors) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_0p5()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.muted)
                .child(label),
        )
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.foreground)
                .child(value.into()),
        )
}
