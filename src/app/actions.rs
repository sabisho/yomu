#![forbid(unsafe_code)]

//! Menu, dialog, confirmation, and book action orchestration.

use std::path::PathBuf;
use std::time::Instant;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::description_list::{DescriptionItem, DescriptionList};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, AppContext, Context, Entity, Focusable, SharedString, Window, px};

use yomu::library::Book;
use yomu::menu::{self, BookAction, Entry};

use crate::app::state::Yomu;
use crate::ui::library_view::directory_groups;
use crate::ui::overlays::{ConfirmAction, ConfirmDialog, Overlay, PropertiesDialog, SeriesDialog};

impl Yomu {
    pub(crate) fn build_book_context_menu(
        &mut self,
        id: &str,
        mut menu: PopupMenu,
        yomu: Entity<Self>,
    ) -> PopupMenu {
        if self.is_quitting {
            return menu;
        }

        self.last_context_menu_book = Some(id.to_string());
        let selected_ids = self.selected_ids();
        let (entries, target_ids) =
            if selected_ids.contains(&id.to_string()) && selected_ids.len() > 1 {
                (menu::selection_menu(selected_ids.len()), selected_ids)
            } else {
                self.selection.clear();
                let Some(book) = self.library.get(id) else {
                    return menu;
                };
                (
                    menu::book_menu(book, book.path.exists()),
                    vec![id.to_string()],
                )
            };

        menu = menu.min_w(px(200.));
        for entry in entries {
            match entry {
                Entry::Separator => {
                    menu = menu.separator();
                }
                Entry::Item(item) => {
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
                    let action = item.action;
                    let book_id = id.to_string();
                    let target_ids = target_ids.clone();
                    let yomu = yomu.clone();
                    menu = menu.item(
                        PopupMenuItem::new(item.label)
                            .icon(icon)
                            .disabled(!item.enabled)
                            .on_click(move |_, window, cx| {
                                yomu.update(cx, |this, cx| {
                                    this.run_book_action(action, &book_id, &target_ids, window, cx);
                                });
                            }),
                    );
                }
            }
        }
        menu
    }

    pub(crate) fn build_group_context_menu(
        &mut self,
        group_key: &str,
        group_name: &str,
        mut menu: PopupMenu,
        yomu: Entity<Self>,
    ) -> PopupMenu {
        if self.is_quitting {
            return menu;
        }
        self.selection.clear();
        let book_ids: Vec<String> = directory_groups(self.library.books())
            .into_iter()
            .find(|g| g.key == group_key)
            .map(|g| g.books.iter().map(|b| b.id.clone()).collect())
            .unwrap_or_default();

        let entries = menu::group_menu(group_name, book_ids.len());
        menu = menu.min_w(px(200.));
        for entry in entries {
            match entry {
                Entry::Separator => {
                    menu = menu.separator();
                }
                Entry::Item(item) => {
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
                    let action = item.action;
                    let target_ids = book_ids.clone();
                    let group_id = format!("group:{}", group_key);
                    let yomu = yomu.clone();
                    menu = menu.item(
                        PopupMenuItem::new(item.label)
                            .icon(icon)
                            .disabled(!item.enabled)
                            .on_click(move |_, window, cx| {
                                yomu.update(cx, |this, cx| {
                                    this.run_book_action(
                                        action,
                                        &group_id,
                                        &target_ids,
                                        window,
                                        cx,
                                    );
                                });
                            }),
                    );
                }
            }
        }
        menu
    }

    pub(crate) fn open_series_dialog(
        &mut self,
        book_ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting || book_ids.is_empty() {
            return;
        }
        let books: Vec<&Book> = book_ids
            .iter()
            .filter_map(|id| self.library.get(id))
            .collect();
        let existing = books
            .first()
            .and_then(|first| first.series.clone())
            .filter(|name| {
                books.iter().all(|book| {
                    book.series
                        .as_deref()
                        .is_some_and(|other| other.eq_ignore_ascii_case(name))
                })
            });
        let common_parent = books
            .first()
            .and_then(|book| book.path.parent().map(PathBuf::from))
            .filter(|parent| {
                books
                    .iter()
                    .all(|book| book.path.parent() == Some(parent.as_path()))
            })
            .and_then(|parent| {
                parent
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            });
        let value = existing.or(common_parent).unwrap_or_default();
        self.dismiss();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value)
                .placeholder("Type a series name…")
        });
        let focus_handle = input.read(cx).focus_handle(cx);
        window.focus(&focus_handle, cx);
        let yomu = cx.entity().clone();
        cx.subscribe(&input, move |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.save_series_dialog(cx);
                let _ = this.window_handle.update(cx, |_, window, cx| {
                    window.close_dialog(cx);
                });
            }
        })
        .detach();
        self.overlay = Some(Overlay::Series(SeriesDialog {
            book_ids: book_ids.clone(),
            input: input.clone(),
        }));
        cx.notify();

        let title = if book_ids.len() == 1 {
            "Set Series"
        } else {
            "Set Series for Selected Books"
        };
        let input_clone = input.clone();
        let initial_focus = std::cell::Cell::new(true);

        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                input_clone.read(cx).focus_handle(cx).focus(window, cx);
            }
            let yomu_close = yomu.clone();
            let yomu_save = yomu.clone();
            dialog
                .title(title)
                .close_button(true)
                .centered()
                .w(px(460.))
                .on_close(move |_, _window, cx| {
                    yomu_close.update(cx, |this, cx| {
                        if matches!(this.overlay, Some(Overlay::Series(_))) {
                            this.dismiss();
                            cx.notify();
                        }
                    });
                })
                .child(
                    v_flex()
                        .gap_3()
                        .py_2()
                        .child(
                            gpui_kit::div()
                                .text_xs()
                                .text_color(yomu.read(cx).colors.muted)
                                .child("Assign a series name to group these volumes."),
                        )
                        .child(
                            Input::new(&input_clone)
                                .id("series-name-input")
                                .cleanable(true),
                        ),
                )
                .footer(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(Button::new("series-cancel").label("Cancel").on_click(
                            move |_, window, cx| {
                                window.close_dialog(cx);
                            },
                        ))
                        .child(Button::new("series-save").primary().label("Save").on_click(
                            move |_, window, cx| {
                                yomu_save.update(cx, |this, cx| {
                                    this.save_series_dialog(cx);
                                });
                                window.close_dialog(cx);
                            },
                        )),
                )
        });
    }

    pub(crate) fn save_series_dialog(&mut self, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        let Some(Overlay::Series(dialog)) = self.overlay.take() else {
            return;
        };
        self.overlay_closed_at = Some(Instant::now());
        let count = dialog.book_ids.len();
        let value = dialog.input.read(cx).value().to_string();
        let series = (!value.trim().is_empty()).then_some(value);

        self.library.set_series(&dialog.book_ids, series.clone());
        self.prune_open_group();
        self.save_library(cx);
        let message = match (&series, count) {
            (Some(name), 1) => format!("Added to series “{name}”"),
            (Some(name), _) => format!("Assigned {count} books to “{name}”"),
            (None, 1) => "Removed from series".to_string(),
            (None, _) => format!("Removed {count} books from their series"),
        };
        self.show_toast(message, cx);
        cx.notify();
    }

    /// Run a menu action, asking first if it would throw away progress.
    pub(crate) fn run_book_action(
        &mut self,
        action: BookAction,
        id: &str,
        target_ids: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting {
            return;
        }
        match action {
            BookAction::Open => {
                if let Some(group_key) = id.strip_prefix("group:") {
                    self.enter_group(group_key.to_string(), cx);
                    self.dismiss();
                    return;
                }
            }
            BookAction::SetSeries => {
                let ids = if !target_ids.is_empty() {
                    target_ids.to_vec()
                } else if id.starts_with("group:")
                    || (self.selection.contains(id) && self.selection.len() > 1)
                {
                    self.selected_ids()
                } else {
                    vec![id.to_string()]
                };
                self.open_series_dialog(ids, window, cx);
                return;
            }
            BookAction::MarkRead => {
                let ids = if !target_ids.is_empty() {
                    target_ids.to_vec()
                } else {
                    self.selected_ids()
                };
                if ids.len() > 1 || id.starts_with("group:") {
                    for bid in &ids {
                        if let Some(book) = self.library.get_mut(bid) {
                            book.mark_read();
                        }
                    }
                    self.save_library(cx);
                    for bid in &ids {
                        self.selection.remove(bid);
                    }
                    self.dismiss();
                    self.show_toast(format!("Marked {} books as read", ids.len()), cx);
                    return;
                }
            }
            BookAction::MarkUnread => {
                let ids = if !target_ids.is_empty() {
                    target_ids.to_vec()
                } else {
                    self.selected_ids()
                };
                if ids.len() > 1 || id.starts_with("group:") {
                    for bid in &ids {
                        if let Some(book) = self.library.get_mut(bid) {
                            book.reset_progress();
                        }
                    }
                    self.save_library(cx);
                    for bid in &ids {
                        self.selection.remove(bid);
                    }
                    self.dismiss();
                    self.show_toast(format!("Marked {} books as unread", ids.len()), cx);
                    return;
                }
            }
            BookAction::RemoveSelected => {
                if id.starts_with("group:") || target_ids.len() > 1 {
                    self.request_remove_books(target_ids, window, cx);
                    return;
                }
                self.request_remove_selected(window, cx);
                return;
            }
            BookAction::ClearSelection => {
                self.dismiss();
                self.selection.clear();
                cx.notify();
                return;
            }
            _ => {}
        }
        let Some(book) = self.library.get(id).cloned() else {
            self.dismiss();
            cx.notify();
            return;
        };
        if let Some(confirmation) = menu::confirmation(action, &book) {
            let confirm_dialog = ConfirmDialog {
                title: confirmation.title.into(),
                message: confirmation.message.into(),
                confirm_label: confirmation.confirm_label,
                action: ConfirmAction::Book(action, id.to_string()),
            };
            self.open_confirm_alert(confirm_dialog, window, cx);
            return;
        }
        self.perform_book_action(action, id, window, cx);
    }

    pub(crate) fn perform_book_action(
        &mut self,
        action: BookAction,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting {
            return;
        }
        self.dismiss();
        let Some(title) = self.library.get(id).map(|book| book.title.clone()) else {
            cx.notify();
            return;
        };
        match action {
            BookAction::Open => self.open_book(id, window, cx),
            BookAction::StartOver => {
                if let Some(book) = self.library.get_mut(id) {
                    book.start_over();
                }
                self.save_library(cx);
                self.open_book(id, window, cx);
            }
            BookAction::MarkRead => {
                if let Some(book) = self.library.get_mut(id) {
                    book.mark_read();
                }
                self.save_library(cx);
                self.show_toast(format!("Marked “{title}” as read"), cx);
            }
            BookAction::MarkUnread | BookAction::ResetProgress => {
                if let Some(book) = self.library.get_mut(id) {
                    book.reset_progress();
                }
                self.save_library(cx);
                let message = if action == BookAction::ResetProgress {
                    format!("Progress reset for “{title}”")
                } else {
                    format!("Marked “{title}” as unread")
                };
                self.show_toast(message, cx);
            }
            BookAction::RevealFile => match self.library.get(id).map(|book| book.path.clone()) {
                Some(path) if path.exists() => cx.reveal_path(&path),
                _ => self.show_toast(format!("“{title}” is missing from disk"), cx),
            },
            BookAction::Properties => {
                self.open_properties_dialog(id, window, cx);
            }
            BookAction::SetSeries => {}
            BookAction::Remove => self.remove_book(id, cx),
            // Handled in `run_book_action`; they never reach here.
            BookAction::RemoveSelected | BookAction::ClearSelection => {}
        }
        cx.notify();
    }

    pub(crate) fn open_properties_dialog(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting {
            return;
        }
        let Some(book) = self.library.get(id).cloned() else {
            return;
        };
        self.dismiss();
        let title: SharedString = book.title.clone().into();
        let rows = menu::properties(&book);
        self.overlay = Some(Overlay::Properties(PropertiesDialog {
            title: title.clone(),
            rows: rows.clone(),
        }));
        cx.notify();

        let yomu = cx.entity().clone();
        let content_rows = rows.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let yomu_close = yomu.clone();
            dialog
                .title(title.clone())
                .close_button(true)
                .centered()
                .w(px(520.))
                .on_close(move |_, _window, cx| {
                    yomu_close.update(cx, |this, cx| {
                        if matches!(this.overlay, Some(Overlay::Properties(_))) {
                            this.dismiss();
                            cx.notify();
                        }
                    });
                })
                .child(
                    DescriptionList::new()
                        .columns(1)
                        .bordered(false)
                        .label_width(px(100.))
                        .children(
                            content_rows
                                .iter()
                                .cloned()
                                .map(|(label, value)| DescriptionItem::new(label).value(value)),
                        ),
                )
                .footer(h_flex().justify_end().child(
                    Button::new("properties-close").label("Close").on_click(
                        move |_, window, cx| {
                            window.close_dialog(cx);
                        },
                    ),
                ))
        });
    }

    pub(crate) fn show_confirm_alert_on_window(
        window: &mut Window,
        cx: &mut App,
        dialog: &ConfirmDialog,
        yomu: Entity<Self>,
    ) {
        let title = dialog.title.clone();
        let message = dialog.message.clone();
        let confirm_label = dialog.confirm_label;
        let action = dialog.action.clone();
        let is_destructive = matches!(
            action,
            ConfirmAction::ClearAllData
                | ConfirmAction::ForceQuit
                | ConfirmAction::RemoveBooks(_)
                | ConfirmAction::RemoveSelected
                | ConfirmAction::ResetSettings
                | ConfirmAction::Book(BookAction::Remove | BookAction::RemoveSelected, _)
        );

        window.open_alert_dialog(cx, move |alert, _window, _cx| {
            let yomu_ok = yomu.clone();
            let yomu_cancel = yomu.clone();
            let action_cancel = action.clone();
            alert
                .confirm()
                .centered()
                .title(title.clone())
                .description(message.clone())
                .ok_text(confirm_label)
                .ok_variant(if is_destructive {
                    ButtonVariant::Danger
                } else {
                    ButtonVariant::Primary
                })
                .on_ok(move |_event, window, cx| {
                    yomu_ok.update(cx, |this, cx| {
                        this.confirm_accept(window, cx);
                    });
                    true
                })
                .on_cancel(move |_event, _window, cx| {
                    yomu_cancel.update(cx, |this, cx| {
                        let back_to_settings = matches!(
                            action_cancel,
                            ConfirmAction::ResetSettings
                                | ConfirmAction::ClearThumbnailCache
                                | ConfirmAction::ClearAllData
                        );
                        this.dismiss();
                        if back_to_settings {
                            this.overlay = Some(Overlay::Settings);
                        }
                        cx.notify();
                    });
                    true
                })
        });
    }

    pub(crate) fn open_confirm_alert(
        &mut self,
        dialog: ConfirmDialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting && !matches!(dialog.action, ConfirmAction::ForceQuit) {
            return;
        }
        self.overlay = Some(Overlay::Confirm(dialog.clone()));
        cx.notify();
        let entity = cx.entity().clone();
        Self::show_confirm_alert_on_window(window, cx, &dialog, entity);
    }

    pub(crate) fn prompt_confirm(&mut self, dialog: ConfirmDialog, cx: &mut Context<Self>) {
        if self.is_quitting && !matches!(dialog.action, ConfirmAction::ForceQuit) {
            return;
        }
        self.overlay = Some(Overlay::Confirm(dialog.clone()));
        cx.notify();
        let entity = cx.entity().clone();
        let _ = self.window_handle.update(cx, |_, window, cx| {
            Self::show_confirm_alert_on_window(window, cx, &dialog, entity);
        });
    }

    pub(crate) fn confirm_accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Overlay::Confirm(dialog)) = self.overlay.take() else {
            return;
        };
        if self.is_quitting && !matches!(dialog.action, ConfirmAction::ForceQuit) {
            return;
        }
        self.overlay_closed_at = Some(Instant::now());
        let back_to_settings = matches!(
            dialog.action,
            ConfirmAction::ResetSettings
                | ConfirmAction::ClearThumbnailCache
                | ConfirmAction::ClearAllData
        );
        match dialog.action {
            ConfirmAction::Book(action, id) => self.perform_book_action(action, &id, window, cx),
            ConfirmAction::RemoveBooks(ids) => self.remove_books(&ids, cx),
            ConfirmAction::RemoveSelected => self.remove_selected(cx),
            ConfirmAction::ResetSettings => self.reset_settings(cx),
            ConfirmAction::ClearThumbnailCache => self.clear_thumbnail_cache(cx),
            ConfirmAction::ClearAllData => self.clear_all_data(cx),
            ConfirmAction::ForceQuit => {
                self.overlay = None;
                cx.quit();
            }
        }
        if back_to_settings {
            self.overlay = Some(Overlay::Settings);
        }
    }

    pub(crate) fn remove_book(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        let Some(title) = self.library.get(id).map(|b| b.title.clone()) else {
            return;
        };
        self.library.remove(id, &self.dirs);
        self.prune_selection();
        self.prune_open_group();
        self.save_library(cx);
        self.show_toast(format!("Removed “{title}” from the library"), cx);
    }

    /// Remove books from the library, asking first only if progress or marks would be lost.
    pub(crate) fn request_remove_books(
        &mut self,
        ids: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ids.is_empty() {
            return;
        }
        let confirmation = {
            let books: Vec<&Book> = ids.iter().filter_map(|id| self.library.get(id)).collect();
            menu::bulk_remove_confirmation(&books)
        };
        match confirmation {
            Some(confirmation) => {
                let confirm_dialog = ConfirmDialog {
                    title: confirmation.title.into(),
                    message: confirmation.message.into(),
                    confirm_label: confirmation.confirm_label,
                    action: ConfirmAction::RemoveBooks(ids.to_vec()),
                };
                self.open_confirm_alert(confirm_dialog, window, cx);
            }
            None => self.remove_books(ids, cx),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn prompt_remove_books(&mut self, ids: &[String], cx: &mut Context<Self>) {
        let ids = ids.to_vec();
        let entity = cx.entity().clone();
        let _ = self.window_handle.update(cx, move |_, window, cx| {
            entity.update(cx, |this, cx| {
                this.request_remove_books(&ids, window, cx);
            });
        });
    }

    pub(crate) fn request_remove_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
        self.request_remove_books(&ids, window, cx);
    }

    #[allow(dead_code)]
    pub(crate) fn prompt_remove_selected(&mut self, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
        self.prompt_remove_books(&ids, cx);
    }

    pub(crate) fn remove_books(&mut self, ids: &[String], cx: &mut Context<Self>) {
        if self.is_quitting {
            return;
        }
        self.dismiss();
        for id in ids {
            self.library.remove(id, &self.dirs);
            self.selection.remove(id);
        }
        self.prune_open_group();
        self.save_library(cx);
        let message = if ids.len() == 1 {
            "Removed 1 book from the library".to_string()
        } else {
            format!("Removed {} books from the library", ids.len())
        };
        self.show_toast(message, cx);
    }

    pub(crate) fn remove_selected(&mut self, cx: &mut Context<Self>) {
        let ids = self.selected_ids();
        self.remove_books(&ids, cx);
    }
}
