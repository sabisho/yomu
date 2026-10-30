//! Collections view matching the macOS design reference.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, FontWeight, InteractiveElement, Pixels, ScrollHandle,
    SharedString, Styled, Window, div, px,
};
use yomu::collections::Collection;

use super::components::with_alpha;
use super::theme::UiColors;

pub struct CollectionsProps<'a> {
    pub scroll_handle: &'a ScrollHandle,
    pub favorites_count: usize,
    pub reading_count: usize,
    pub want_to_read_count: usize,
    pub completed_count: usize,
    pub custom_collections: &'a [Collection],
    pub colors: UiColors,
    pub radius: Pixels,
}

pub fn render_collections_view(
    props: CollectionsProps,
    on_select_smart: impl Fn(&'static str, &mut Window, &mut App) + 'static,
    on_select_custom: impl Fn(&str, &mut Window, &mut App) + 'static,
    on_new_collection: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_collection_menu: impl Fn(&str, (f32, f32), &mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_select_smart = std::rc::Rc::new(on_select_smart);
    let on_select_custom = std::rc::Rc::new(on_select_custom);
    let on_collection_menu = std::rc::Rc::new(on_collection_menu);
    let colors = props.colors;

    let smart_card =
        |id: &'static str, title: &'static str, count: usize, icon: IconName| -> AnyElement {
            let on_select_smart = on_select_smart.clone();
            div()
                .id(ElementId::from(id))
                .flex_1()
                .h(px(84.))
                .p_4()
                .rounded(props.radius)
                .bg(with_alpha(colors.surface, 0.65))
                .border_1()
                .border_color(colors.border)
                .cursor_pointer()
                .hover(|style| {
                    style
                        .bg(with_alpha(colors.foreground, 0.05))
                        .border_color(colors.accent)
                })
                .on_click(move |_, window, cx| on_select_smart(id, window, cx))
                .flex()
                .flex_row()
                .items_center()
                .gap_4()
                .child(
                    div()
                        .size(px(40.))
                        .rounded(props.radius)
                        .bg(with_alpha(colors.foreground, 0.08))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(icon).size(px(20.)).text_color(colors.foreground)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(colors.foreground)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.muted)
                                .child(format!("{count} manga")),
                        ),
                )
                .into_any_element()
        };

    div()
        .id("collections-container")
        .size_full()
        .overflow_y_scrollbar()
        .track_scroll(props.scroll_handle)
        .scroll_bounce(true)
        .scroll_wheel(false)
        .p_6()
        .flex()
        .flex_col()
        .gap_6()
        // Top Toolbar inside view
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::BOLD)
                        .text_color(colors.foreground)
                        .child("Collections"),
                )
                .child(
                    Button::new("new-collection-btn")
                        .secondary()
                        .outline()
                        .label("+ New Collection")
                        .rounded(ButtonRounded::Medium)
                        .small()
                        .on_click(on_new_collection),
                ),
        )
        // Default / Smart Collections Grid (2x2)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .gap_4()
                        .child(smart_card(
                            "favorites",
                            "Favorites",
                            props.favorites_count,
                            IconName::Heart,
                        ))
                        .child(smart_card(
                            "reading",
                            "Currently Reading",
                            props.reading_count,
                            IconName::BookOpen,
                        )),
                )
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .gap_4()
                        .child(smart_card(
                            "want_to_read",
                            "Want to Read",
                            props.want_to_read_count,
                            IconName::Clock,
                        ))
                        .child(smart_card(
                            "completed",
                            "Completed",
                            props.completed_count,
                            IconName::Check,
                        )),
                ),
        )
        // Section: My Collections
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .pt_3()
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::BOLD)
                        .text_color(colors.foreground)
                        .child("My Collections"),
                )
                .child(if props.custom_collections.is_empty() {
                    div()
                        .w_full()
                        .p_8()
                        .rounded(props.radius)
                        .border_1()
                        .border_color(colors.border)
                        .bg(with_alpha(colors.surface, 0.4))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(colors.foreground)
                                .child("No Custom Collections Yet"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.muted)
                                .child("Click '+ New Collection' above to create categories like Seinen, Shonen, etc."),
                        )
                        .into_any_element()
                } else {
                    div()
                        .w_full()
                        .flex()
                        .flex_wrap()
                        .gap_4()
                        .children(props.custom_collections.iter().map(|col| {
                            let on_select_custom = on_select_custom.clone();
                            let on_collection_menu = on_collection_menu.clone();
                            let col_id = col.id.clone();
                            let col_id_clone = col_id.clone();
                            let card_elem_id = ElementId::from(SharedString::from(format!("custom-card-{}", col.id)));
                            let menu_elem_id = ElementId::from(SharedString::from(format!("custom-menu-{}", col.id)));
                            div()
                                .id(card_elem_id)
                                .w(px(260.))
                                .h(px(84.))
                                .p_4()
                                .rounded(props.radius)
                                .bg(with_alpha(colors.surface, 0.65))
                                .border_1()
                                .border_color(colors.border)
                                .cursor_pointer()
                                .hover(|style| {
                                    style
                                        .bg(with_alpha(colors.foreground, 0.05))
                                        .border_color(colors.accent)
                                })
                                .on_click(move |_, window, cx| {
                                    on_select_custom(&col_id, window, cx)
                                })
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_3p5()
                                        .child(
                                            div()
                                                .size(px(40.))
                                                .rounded(props.radius)
                                                .bg(with_alpha(colors.foreground, 0.08))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .child(
                                                    Icon::new(IconName::Folder)
                                                        .size(px(20.))
                                                        .text_color(colors.foreground),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .gap_0p5()
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(colors.foreground)
                                                        .child(col.name.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(colors.muted)
                                                        .child(format!("{} manga", col.book_ids.len())),
                                                ),
                                        ),
                                )
                                .child(
                                    div()
                                        .id(menu_elem_id)
                                        .size(px(28.))
                                        .rounded(props.radius)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|s| s.bg(with_alpha(colors.foreground, 0.1)))
                                        .on_click(move |event: &ClickEvent, window, cx| {
                                            let pos: (f32, f32) = (event.position().x.into(), event.position().y.into());
                                            on_collection_menu(&col_id_clone, pos, window, cx);
                                        })
                                        .child(
                                            Icon::new(IconName::Ellipsis)
                                                .size(px(14.))
                                                .text_color(colors.muted),
                                        ),
                                )
                        }))
                        .into_any_element()
                }),
        )
        .into_any_element()
}
