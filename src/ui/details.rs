//! Dedicated Manga Details view matching the macOS reference design.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, FontWeight, InteractiveElement, MouseButton, MouseDownEvent,
    Pixels, ScrollHandle, ScrollWheelEvent, SharedString, Styled, Window, div, img, px,
};
use yomu::anilist::AniListMedia;
use yomu::library::Book;

use super::components::{genre_tag, star_rating, with_alpha};
use super::theme::UiColors;

pub struct MangaDetailsProps<'a> {
    pub scroll_handle: &'a ScrollHandle,
    pub book: &'a Book,
    pub cover_path: PathBuf,
    pub anilist_meta: Option<&'a AniListMedia>,
    pub selected_tab: usize,
    pub synopsis_expanded: bool,
    pub colors: UiColors,
    pub radius: Pixels,
}

pub fn render_manga_details(
    props: MangaDetailsProps,
    on_scroll_wheel: impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static,
    on_mouse_down: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    on_back: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_continue_reading: impl Fn(&str, &mut Window, &mut App) + 'static,
    on_select_tab: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_toggle_synopsis: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_anilist_action: impl Fn(&str, &mut Window, &mut App) + 'static,
    on_sync_progress: impl Fn(&str, &mut Window, &mut App) + 'static,
    on_more_menu: impl Fn(&str, (f32, f32), &mut Window, &mut App) + 'static,
) -> AnyElement {
    let colors = props.colors;
    let book = props.book;
    let book_id = book.id.clone();
    let book_id_clone2 = book_id.clone();
    let book_id_clone3 = book_id.clone();

    let display_title = props
        .anilist_meta
        .map(|m| m.title.preferred())
        .unwrap_or(book.title.as_str());

    let subtitle = props.anilist_meta.and_then(|m| m.title.subtitle());

    let year_str = props
        .anilist_meta
        .and_then(|m| m.start_year)
        .map(|y| y.to_string())
        .unwrap_or_else(|| "2024".to_string());

    let status_str = props
        .anilist_meta
        .and_then(|m| m.status.as_deref())
        .unwrap_or("Ongoing");

    let metadata_line = format!("Manga · {year_str} · {status_str}");

    let genres = props
        .anilist_meta
        .map(|m| m.genres.clone())
        .unwrap_or_else(|| vec!["Manga".to_string(), "Comics".to_string()]);

    let synopsis = props
        .anilist_meta
        .map(|m| m.clean_description())
        .unwrap_or_else(|| "Imported local manga volume. Synopsis unavailable.".to_string());

    let chapters_total = props
        .anilist_meta
        .and_then(|m| m.chapters)
        .unwrap_or(book.page_count as i32);

    let current_pos = book.current_page as i32 + 1;
    let progress_val = book.progress();

    div()
        .id("manga-details-container")
        .track_scroll(props.scroll_handle)
        .size_full()
        .overflow_y_scroll()
        .on_scroll_wheel(on_scroll_wheel)
        .on_mouse_down(MouseButton::Left, on_mouse_down)
        .p_6()
        .flex()
        .flex_col()
        .gap_6()
        // Top Back navigation bar
        .child(
            div().flex().items_center().gap_2().child(
                Button::new("details-back-btn")
                    .ghost()
                    .label("← Back to Library")
                    .rounded(ButtonRounded::Medium)
                    .small()
                    .tooltip("Back to library")
                    .on_click(on_back),
            ),
        )
        // Main Upper Layout: Left Cover + Right Information
        .child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_8()
                // Left Column: Cover artwork
                .child(
                    div()
                        .w(px(240.))
                        .h(px(340.))
                        .flex_none()
                        .rounded(props.radius)
                        .overflow_hidden()
                        .border_1()
                        .border_color(colors.border)
                        .shadow_lg()
                        .bg(with_alpha(colors.surface, 0.7))
                        .child(
                            img(props.cover_path)
                                .size_full()
                                .object_fit(gpui_kit::ObjectFit::Cover),
                        ),
                )
                // Right Column: Details & Actions
                .child(
                    div()
                        .flex_1()
                        .min_w(px(260.))
                        .flex()
                        .flex_col()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1p5()
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
                                    div().pt_1().child(star_rating(
                                        props
                                            .anilist_meta
                                            .and_then(|m| m.average_score)
                                            .unwrap_or(85),
                                        props.anilist_meta.and_then(|m| m.popularity),
                                        colors,
                                    )),
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
                                    Button::new("details-anilist-btn")
                                        .secondary()
                                        .outline()
                                        .label(if props.anilist_meta.is_some() {
                                            "✓ In AniList"
                                        } else {
                                            "+ Link AniList"
                                        })
                                        .rounded(ButtonRounded::Medium)
                                        .tooltip(if props.anilist_meta.is_some() {
                                            "Linked with AniList"
                                        } else {
                                            "Link with AniList"
                                        })
                                        .on_click(move |_, window, cx| {
                                            on_anilist_action(&book_id_clone2, window, cx)
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
                                            on_more_menu(&book_id_clone3, pos, window, cx);
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
                .child(Tab::new().label("Chapters"))
                .child(Tab::new().label("Characters"))
                .child(Tab::new().label("Related")),
        )
        // Tab Content: Overview Metadata Grid (4 cols x 2 rows)
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
                            "Chapters",
                            format!("{chapters_total}+"),
                            colors,
                        ))
                        .child(metadata_cell("Status", status_str, colors))
                        .child(metadata_cell("Published", year_str, colors))
                        .child(metadata_cell(
                            "Authors",
                            props
                                .anilist_meta
                                .and_then(|m| m.author.as_deref())
                                .unwrap_or("Kanehito Yamada"),
                            colors,
                        ))
                        .child(metadata_cell(
                            "Artists",
                            props
                                .anilist_meta
                                .and_then(|m| m.artist.as_deref())
                                .unwrap_or("Tsukasa Abe"),
                            colors,
                        ))
                        .child(metadata_cell(
                            "Serialization",
                            props
                                .anilist_meta
                                .and_then(|m| m.serialization.as_deref())
                                .unwrap_or("Weekly Shōnen"),
                            colors,
                        )),
                ),
        )
        // Bottom Tracking Card (My Progress, AniList Status, Chapters Read + Sync)
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
                                .child(format!("{current_pos} / {chapters_total}")),
                        ),
                )
                // Center: AniList Status
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.foreground)
                                .child("AniList Status"),
                        )
                        .child(
                            div()
                                .px_3()
                                .py_1p5()
                                .rounded(props.radius)
                                .bg(with_alpha(colors.foreground, 0.06))
                                .border_1()
                                .border_color(colors.border)
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(colors.foreground)
                                .child(
                                    props
                                        .anilist_meta
                                        .and_then(|m| m.user_status.as_ref())
                                        .map(|s| s.label())
                                        .unwrap_or("Reading ⌵"),
                                ),
                        ),
                )
                // Right: Chapters Read + Sync Button
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(colors.foreground)
                                        .child("Chapters Read"),
                                )
                                .child(
                                    div()
                                        .text_base()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(colors.foreground)
                                        .child(current_pos.to_string()),
                                ),
                        )
                        .child({
                            let sync_book_id = props.book.id.clone();
                            Button::new("details-sync-btn")
                                .secondary()
                                .label("🔄 Sync")
                                .rounded(ButtonRounded::Medium)
                                .small()
                                .on_click(move |_, window, cx| {
                                    on_sync_progress(&sync_book_id, window, cx)
                                })
                        }),
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
