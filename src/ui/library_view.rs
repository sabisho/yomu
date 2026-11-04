//! Library card, folder, and list row rendering primitives for Yomu's two-pane application shell.

#![allow(clippy::too_many_arguments)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, ClickEvent, Context, ElementId, FontWeight, InteractiveElement, ObjectFit, Pixels,
    SharedString, Styled, Window, div, img, px, relative,
};

use crate::library::{Book, SortBy, SortDirection};
use crate::ui::components::{stat_badge, with_alpha};
use crate::ui::theme::UiColors;
use yomu::selection::Selection;
use yomu::util::natural_cmp;

/// Returns a concise uppercase archive format or source label (e.g. "CBZ", "CBR", "ZIP", "DIR").
pub fn book_format_label(book: &Book) -> &'static str {
    if book.kind == yomu::archive::SourceKind::Folder {
        "DIR"
    } else if let Some(ext) = book.path.extension().and_then(|s| s.to_str()) {
        match ext.to_ascii_uppercase().as_str() {
            "CBZ" => "CBZ",
            "CBR" => "CBR",
            "ZIP" => "ZIP",
            "RAR" => "RAR",
            "PDF" => "PDF",
            _ => "ARC",
        }
    } else {
        "ARC"
    }
}

/// The selection state of a group or series folder card, derived from its member volumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupSelectionState {
    /// No member volumes are selected.
    Unselected,
    /// Some, but not all, member volumes are selected.
    PartiallySelected,
    /// All member volumes are selected.
    FullySelected,
}

/// Derives the selection state of a group card from its member volume IDs.
pub fn compute_group_selection_state<'a>(
    member_ids: impl IntoIterator<Item = &'a str>,
    selection: &Selection,
) -> GroupSelectionState {
    let mut total = 0;
    let mut selected = 0;
    for id in member_ids {
        total += 1;
        if selection.contains(id) {
            selected += 1;
        }
    }
    if total == 0 || selected == 0 {
        GroupSelectionState::Unselected
    } else if selected == total {
        GroupSelectionState::FullySelected
    } else {
        GroupSelectionState::PartiallySelected
    }
}

/// A displayable item in the virtualized library view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryItem {
    Folder {
        key: String,
        name: String,
        count: usize,
        cover_id: String,
        member_ids: Arc<[String]>,
    },
    Book {
        id: String,
        /// Cached index in `Library::books` for fast O(1) lookup.
        index: usize,
    },
}

/// Cached partition of library items into virtualized rows to eliminate
/// per-frame element construction and layout recomputation.
#[derive(Clone, Default)]
pub struct LibraryRowCache {
    pub mutation_seq: u64,
    pub save_seq: u64,
    pub books_len: usize,
    pub search_query: String,
    pub sort_by: SortBy,
    pub sort_direction: SortDirection,
    pub group_series: bool,
    pub open_group: Option<String>,
    pub view_mode_grid: bool,
    pub cols: usize,
    pub card_w: f32,
    pub cover_h: f32,
    pub gap: f32,
    pub pad_x: f32,
    pub rows: Vec<Vec<LibraryItem>>,
    pub item_sizes: std::rc::Rc<Vec<gpui_kit::Size<gpui_kit::Pixels>>>,
}

impl LibraryRowCache {
    pub fn is_valid(
        &self,
        mutation_seq: u64,
        save_seq: u64,
        books_len: usize,
        search_query: &str,
        sort_by: SortBy,
        sort_direction: SortDirection,
        group_series: bool,
        open_group: Option<&str>,
        view_mode_grid: bool,
        cols: usize,
        card_w: f32,
        cover_h: f32,
        gap: f32,
        pad_x: f32,
    ) -> bool {
        self.mutation_seq == mutation_seq
            && self.save_seq == save_seq
            && self.books_len == books_len
            && self.search_query == search_query
            && self.sort_by == sort_by
            && self.sort_direction == sort_direction
            && self.group_series == group_series
            && self.open_group.as_deref() == open_group
            && self.view_mode_grid == view_mode_grid
            && self.cols == cols
            && (self.card_w - card_w).abs() < 0.1
            && (self.cover_h - cover_h).abs() < 0.1
            && (self.gap - gap).abs() < 0.1
            && (self.pad_x - pad_x).abs() < 0.1
    }
}

/// A grouped set of books sharing a parent directory or an explicit series name.
pub struct LibraryGroup<'a> {
    pub key: String,
    #[allow(dead_code)]
    pub name: String,
    pub first_seen: usize,
    pub books: Vec<&'a Book>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum GroupKey<'a> {
    Series(std::borrow::Cow<'a, str>),
    Directory(&'a Path),
}

/// Partitions a slice of books into series or directory-based folders.
pub fn directory_groups<'a>(books: impl IntoIterator<Item = &'a Book>) -> Vec<LibraryGroup<'a>> {
    let mut buckets: std::collections::BTreeMap<
        GroupKey<'a>,
        (String, String, usize, Vec<&'a Book>),
    > = std::collections::BTreeMap::new();
    for (index, book) in books.into_iter().enumerate() {
        let manual_series = book
            .series
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty());

        let group_key = if let Some(series) = manual_series {
            if series.chars().any(char::is_uppercase) {
                GroupKey::Series(std::borrow::Cow::Owned(series.to_lowercase()))
            } else {
                GroupKey::Series(std::borrow::Cow::Borrowed(series))
            }
        } else {
            let parent = book.path.parent().unwrap_or_else(|| Path::new("."));
            GroupKey::Directory(parent)
        };

        if let Some((_, _, _, members)) = buckets.get_mut(&group_key) {
            members.push(book);
        } else {
            let (key_str, name) = match &group_key {
                GroupKey::Series(series) => (
                    format!("series:{series}"),
                    manual_series.unwrap().to_string(),
                ),
                GroupKey::Directory(parent) => {
                    let key = format!("directory:{}", parent.display());
                    let name = parent
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| parent.display().to_string());
                    (key, name)
                }
            };
            buckets.insert(group_key, (key_str, name, index, vec![book]));
        }
    }

    let mut groups: Vec<_> = buckets
        .into_values()
        .filter_map(|(key, name, first_seen, members)| {
            if members.len() < 2 && !key.starts_with("series:") {
                return None;
            }
            Some(LibraryGroup {
                key,
                name,
                first_seen,
                books: members,
            })
        })
        .collect();
    groups.sort_by_key(|group| group.first_seen);
    groups
}

/// Sorts library folder groups according to the selected mode and direction.
pub fn sort_groups<'a>(groups: &mut [LibraryGroup<'a>], by: SortBy, dir: SortDirection) {
    match by {
        SortBy::Default => {
            groups.sort_by_key(|group| group.first_seen);
            if dir == SortDirection::Descending {
                groups.reverse();
            }
        }
        SortBy::Title => {
            groups.sort_by(|a, b| {
                let ord = natural_cmp(&a.name, &b.name);
                if dir == SortDirection::Descending {
                    ord.reverse()
                } else {
                    ord
                }
                .then_with(|| a.first_seen.cmp(&b.first_seen))
            });
        }
        SortBy::DateAdded => {
            groups.sort_by(|a, b| {
                let max_a = a.books.iter().map(|bk| bk.added_at).max().unwrap_or(0);
                let max_b = b.books.iter().map(|bk| bk.added_at).max().unwrap_or(0);
                let ord = if dir == SortDirection::Descending {
                    max_b.cmp(&max_a)
                } else {
                    max_a.cmp(&max_b)
                };
                ord.then_with(|| natural_cmp(&a.name, &b.name))
                    .then_with(|| a.first_seen.cmp(&b.first_seen))
            });
        }
        SortBy::LastRead => {
            groups.sort_by(|a, b| {
                let max_a = a.books.iter().filter_map(|bk| bk.last_read).max();
                let max_b = b.books.iter().filter_map(|bk| bk.last_read).max();
                let ord = match (max_a, max_b) {
                    (Some(ta), Some(tb)) => {
                        if dir == SortDirection::Descending {
                            tb.cmp(&ta)
                        } else {
                            ta.cmp(&tb)
                        }
                    }
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                };
                ord.then_with(|| natural_cmp(&a.name, &b.name))
                    .then_with(|| a.first_seen.cmp(&b.first_seen))
            });
        }
        SortBy::Progress => {
            groups.sort_by(|a, b| {
                let pa = if a.books.is_empty() {
                    0.0
                } else {
                    a.books.iter().map(|bk| bk.progress()).sum::<f32>() / a.books.len() as f32
                };
                let pb = if b.books.is_empty() {
                    0.0
                } else {
                    b.books.iter().map(|bk| bk.progress()).sum::<f32>() / b.books.len() as f32
                };
                let ord = if dir == SortDirection::Descending {
                    pb.total_cmp(&pa)
                } else {
                    pa.total_cmp(&pb)
                };
                ord.then_with(|| natural_cmp(&a.name, &b.name))
                    .then_with(|| a.first_seen.cmp(&b.first_seen))
            });
        }
    }
}

/// Properties needed to render an individual book grid card.
pub struct BookCardProps<'a> {
    pub book: &'a Book,
    pub cover_path: PathBuf,
    pub card_w: f32,
    pub cover_h: f32,
    pub radius: Pixels,
    pub border_px: f32,
    pub is_selected: bool,
    pub colors: UiColors,
}

/// Renders a single book grid card.
pub fn render_book_card(
    props: BookCardProps,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    context_menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> AnyElement {
    let book = props.book;
    let card_w = props.card_w;
    let cover_h = props.cover_h;
    let radius = props.radius;
    let border_px = props.border_px;
    let is_selected = props.is_selected;
    let colors = props.colors;
    let inner_radius = (radius - px(border_px)).max(px(0.));
    let progress = book.progress();
    let total = book.page_count;
    let current = if book.opened {
        book.current_page + 1
    } else {
        0
    };
    let format_label = book_format_label(book);

    div()
        .id((ElementId::from("manga-item"), book.id.as_str()))
        .w(px(card_w))
        .flex_none()
        .flex()
        .flex_col()
        .rounded(radius)
        .bg(if is_selected {
            with_alpha(colors.accent, 0.12)
        } else {
            with_alpha(colors.surface, 0.85)
        })
        .when(border_px > 0.0 || is_selected, |s| {
            let w = if is_selected {
                border_px.max(1.5)
            } else {
                border_px
            };
            s.border(px(w)).border_color(if is_selected {
                colors.accent
            } else {
                colors.border
            })
        })
        .overflow_hidden()
        .cursor_pointer()
        .hover(|s| s.border_color(colors.accent).shadow_md().bg(colors.surface))
        .on_click(on_click)
        .context_menu(context_menu)
        .child(
            div()
                .id((ElementId::from("manga-cover"), book.id.as_str()))
                .relative()
                .w_full()
                .h(px(cover_h))
                .rounded_t(inner_radius)
                .bg(with_alpha(colors.foreground, 0.04))
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(props.cover_path)
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .rounded_t(inner_radius),
                )
                // 1. Format tag overlay (top-left of cover)
                .child(
                    div()
                        .absolute()
                        .top_2()
                        .left_2()
                        .px_1p5()
                        .py(px(1.))
                        .rounded(px(4.))
                        .bg(with_alpha(colors.background, 0.8))
                        .border_1()
                        .border_color(with_alpha(colors.border, 0.6))
                        .text_color(colors.muted)
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .child(format_label),
                )
                // 2. Selection indicator (top-right of cover)
                .children(is_selected.then(|| {
                    div()
                        .absolute()
                        .top_2()
                        .right_2()
                        .size(px(20.))
                        .rounded(px(4.))
                        .bg(colors.accent)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::Check)
                                .size(px(12.))
                                .text_color(colors.accent_foreground),
                        )
                }))
                // 3. Selection tint over cover
                .children(is_selected.then(|| {
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(with_alpha(colors.accent, 0.15))
                }))
                // 4. In-progress bottom accent bar & page indicator pill on cover
                .children((book.opened && total > 0).then(|| {
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .w_full()
                        .flex()
                        .flex_col()
                        .child(
                            div().w_full().flex().justify_end().p_1p5().child(
                                div()
                                    .px_1p5()
                                    .py(px(1.))
                                    .rounded(px(4.))
                                    .bg(with_alpha(colors.background, 0.85))
                                    .text_color(colors.foreground)
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(format!("{current}/{total}")),
                            ),
                        )
                        .child(
                            div()
                                .w_full()
                                .h(px(3.))
                                .bg(with_alpha(colors.background, 0.5))
                                .child(
                                    div()
                                        .h_full()
                                        .w(relative(progress.clamp(0.0, 1.0)))
                                        .bg(colors.accent),
                                ),
                        )
                })),
        )
        .child(
            div()
                .p_3()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.foreground)
                        .truncate()
                        .child(book.title.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(div().text_xs().text_color(colors.muted).truncate().child(
                            if let Some(series) = book.series.as_deref() {
                                format!("{series} · {total} pages")
                            } else if book.opened {
                                format!("Ch. {current} of {total}")
                            } else {
                                format!("{total} pages")
                            },
                        ))
                        .children(book.opened.then(|| {
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(colors.accent)
                                .child(format!("{:.0}%", progress * 100.0))
                        })),
                )
                .children(book.opened.then(|| {
                    div().w_full().pt_1().child(
                        Progress::new(ElementId::from(SharedString::from(format!(
                            "card-prog-{}",
                            book.id
                        ))))
                        .value(progress * 100.0)
                        .h(px(3.))
                        .rounded(radius),
                    )
                })),
        )
        .into_any_element()
}

/// Renders a single book list row.
pub fn render_book_list_row(
    book: &Book,
    cover_path: PathBuf,
    radius: Pixels,
    border_px: f32,
    is_selected: bool,
    colors: UiColors,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    context_menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> AnyElement {
    let progress = book.progress();
    let total = book.page_count;
    let current = if book.opened {
        book.current_page + 1
    } else {
        0
    };
    let display_title = book.title.clone();

    ListItem::new((ElementId::from("manga-item"), book.id.as_str()))
        .h(px(64.))
        .px_3p5()
        .rounded(radius)
        .selected(is_selected)
        .bg(if is_selected {
            with_alpha(colors.accent, 0.12)
        } else {
            with_alpha(colors.surface, 0.85)
        })
        .border(px(if is_selected {
            border_px.max(1.5)
        } else {
            border_px.max(1.0)
        }))
        .border_color(if is_selected {
            colors.accent
        } else {
            colors.border
        })
        .on_click(on_click)
        .context_menu(context_menu)
        .child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .min_w_0()
                        .flex_1()
                        .children(is_selected.then(|| {
                            div()
                                .size(px(18.))
                                .rounded(px(4.))
                                .bg(colors.accent)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(IconName::Check)
                                        .size(px(12.))
                                        .text_color(colors.accent_foreground),
                                )
                        }))
                        .child(
                            div()
                                .id((ElementId::from("manga-cover"), book.id.as_str()))
                                .w(px(38.))
                                .h(px(52.))
                                .flex_none()
                                .rounded(px(f32::from(radius).min(4.)))
                                .bg(with_alpha(colors.foreground, 0.04))
                                .overflow_hidden()
                                .child(
                                    img(cover_path)
                                        .size_full()
                                        .rounded(px(f32::from(radius).min(4.)))
                                        .object_fit(ObjectFit::Contain),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .min_w_0()
                                .flex_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(colors.foreground)
                                        .truncate()
                                        .child(display_title),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .text_xs()
                                        .text_color(colors.muted)
                                        .child(
                                            Tag::secondary().small().child(book_format_label(book)),
                                        )
                                        .children(
                                            book.series.as_deref().map(|s| {
                                                Tag::secondary().small().child(s.to_string())
                                            }),
                                        )
                                        .child(div().truncate().child(if book.opened {
                                            format!(
                                                "Ch. {current} of {total} · {:.0}%",
                                                progress * 100.0
                                            )
                                        } else {
                                            format!("{total} pages")
                                        })),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .flex_none()
                        .children(book.opened.then(|| {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    Progress::new(ElementId::from(SharedString::from(format!(
                                        "list-prog-{}",
                                        book.id
                                    ))))
                                    .value(progress * 100.0)
                                    .w(px(90.)),
                                )
                                .child(
                                    div()
                                        .w(px(34.))
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(colors.muted)
                                        .child(format!("{:.0}%", progress * 100.0)),
                                )
                        }))
                        .child(if progress >= 1.0 {
                            Tag::success().small().child("Completed")
                        } else if book.opened {
                            Tag::primary().small().child("Reading")
                        } else {
                            Tag::secondary().small().child("Unread")
                        }),
                ),
        )
        .into_any_element()
}

/// Renders a series or directory folder grid card.
pub fn render_folder_card(
    group_key: &str,
    group_name: &str,
    count: usize,
    cover_path: PathBuf,
    card_w: f32,
    cover_h: f32,
    radius: Pixels,
    border_px: f32,
    selection_state: GroupSelectionState,
    colors: UiColors,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    context_menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> AnyElement {
    let inner_radius = (radius - px(border_px)).max(px(0.));
    let is_selected = selection_state != GroupSelectionState::Unselected;

    div()
        .id((ElementId::from("folder"), group_key))
        .w(px(card_w))
        .flex_none()
        .flex()
        .flex_col()
        .rounded(radius)
        .bg(match selection_state {
            GroupSelectionState::FullySelected => with_alpha(colors.accent, 0.12),
            GroupSelectionState::PartiallySelected => with_alpha(colors.accent, 0.08),
            GroupSelectionState::Unselected => with_alpha(colors.surface, 0.85),
        })
        .when(border_px > 0.0 || is_selected, |s| {
            let w = if is_selected {
                border_px.max(1.5)
            } else {
                border_px
            };
            s.border(px(w)).border_color(if is_selected {
                colors.accent
            } else {
                colors.border
            })
        })
        .overflow_hidden()
        .cursor_pointer()
        .hover(|s| s.border_color(colors.accent).shadow_md().bg(colors.surface))
        .on_click(on_click)
        .context_menu(context_menu)
        // Cover container
        .child(
            div()
                .relative()
                .w_full()
                .h(px(cover_h))
                .rounded_t(inner_radius)
                .bg(with_alpha(colors.foreground, 0.04))
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(cover_path)
                        .size_full()
                        .rounded_t(inner_radius)
                        .object_fit(ObjectFit::Cover),
                )
                // Selection tint over cover
                .children(match selection_state {
                    GroupSelectionState::FullySelected => Some(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .bg(with_alpha(colors.accent, 0.15)),
                    ),
                    GroupSelectionState::PartiallySelected => Some(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .bg(with_alpha(colors.accent, 0.08)),
                    ),
                    GroupSelectionState::Unselected => None,
                })
                // Top-left folder badge
                .child(
                    div()
                        .absolute()
                        .top_2()
                        .left_2()
                        .px_2()
                        .py(px(1.5))
                        .rounded(px(4.))
                        .bg(with_alpha(colors.background, 0.8))
                        .border_1()
                        .border_color(with_alpha(colors.border, 0.6))
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Icon::new(IconName::Folder)
                                .size(px(12.))
                                .text_color(colors.accent),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(colors.foreground)
                                .child("SERIES"),
                        ),
                )
                // Top-right volume count badge + selection indicator
                .child(
                    div()
                        .absolute()
                        .top_2()
                        .right_2()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(stat_badge(count, false, colors, radius))
                        .children(match selection_state {
                            GroupSelectionState::FullySelected => Some(
                                div()
                                    .size(px(20.))
                                    .rounded(px(4.))
                                    .bg(colors.accent)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(IconName::Check)
                                            .size(px(12.))
                                            .text_color(colors.accent_foreground),
                                    ),
                            ),
                            GroupSelectionState::PartiallySelected => Some(
                                div()
                                    .size(px(20.))
                                    .rounded(px(4.))
                                    .bg(colors.accent)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(IconName::Minus)
                                            .size(px(12.))
                                            .text_color(colors.accent_foreground),
                                    ),
                            ),
                            GroupSelectionState::Unselected => None,
                        }),
                ),
        )
        // Metadata container below cover
        .child(
            div()
                .p_3()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(colors.foreground)
                        .truncate()
                        .child(group_name.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.muted)
                        .truncate()
                        .child(format!("{count} volumes · Series")),
                ),
        )
        .into_any_element()
}

/// Renders a series or directory folder list row.
pub fn render_folder_list_row(
    group_key: &str,
    group_name: &str,
    count: usize,
    cover_path: PathBuf,
    radius: Pixels,
    border_px: f32,
    selection_state: GroupSelectionState,
    colors: UiColors,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    context_menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> AnyElement {
    let is_selected = selection_state != GroupSelectionState::Unselected;
    ListItem::new((ElementId::from("folder-item"), group_key))
        .h(px(64.))
        .px_3p5()
        .rounded(radius)
        .selected(is_selected)
        .bg(match selection_state {
            GroupSelectionState::FullySelected => with_alpha(colors.accent, 0.12),
            GroupSelectionState::PartiallySelected => with_alpha(colors.accent, 0.08),
            GroupSelectionState::Unselected => with_alpha(colors.surface, 0.85),
        })
        .border(px(if is_selected {
            border_px.max(1.5)
        } else {
            border_px.max(1.0)
        }))
        .border_color(if is_selected {
            colors.accent
        } else {
            colors.border
        })
        .on_click(on_click)
        .context_menu(context_menu)
        .child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .min_w_0()
                        .flex_1()
                        .children(match selection_state {
                            GroupSelectionState::FullySelected => Some(
                                div()
                                    .size(px(18.))
                                    .rounded(px(4.))
                                    .bg(colors.accent)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(IconName::Check)
                                            .size(px(12.))
                                            .text_color(colors.accent_foreground),
                                    ),
                            ),
                            GroupSelectionState::PartiallySelected => Some(
                                div()
                                    .size(px(18.))
                                    .rounded(px(4.))
                                    .bg(colors.accent)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(IconName::Minus)
                                            .size(px(12.))
                                            .text_color(colors.accent_foreground),
                                    ),
                            ),
                            GroupSelectionState::Unselected => None,
                        })
                        .child(
                            div()
                                .w(px(38.))
                                .h(px(52.))
                                .flex_none()
                                .rounded(px(f32::from(radius).min(4.)))
                                .bg(with_alpha(colors.foreground, 0.04))
                                .overflow_hidden()
                                .child(
                                    img(cover_path)
                                        .size_full()
                                        .rounded(px(f32::from(radius).min(4.)))
                                        .object_fit(ObjectFit::Contain),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .min_w_0()
                                .flex_1()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1p5()
                                        .child(
                                            Icon::new(IconName::Folder)
                                                .size(px(14.))
                                                .text_color(colors.accent),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(colors.foreground)
                                                .truncate()
                                                .child(group_name.to_string()),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(colors.muted)
                                        .child(format!("{count} volumes · Series folder")),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2p5()
                        .flex_none()
                        .child(stat_badge(count, false, colors, radius))
                        .child(
                            Icon::new(IconName::ChevronRight)
                                .size(px(16.))
                                .text_color(colors.muted),
                        ),
                ),
        )
        .into_any_element()
}
