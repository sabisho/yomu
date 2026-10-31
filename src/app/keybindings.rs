#![forbid(unsafe_code)]

//! Keyboard shortcuts, Vim motions, and action dispatch.

use gpui_kit::{Context, KeyDownEvent, Window};

use yomu::layout::{self, Arrive, FitMode, Viewport};
use yomu::menu::{self, Entry};
use yomu::vim::{self, Action, Amount, Dir};

use crate::app::state::Yomu;
use crate::ui::overlays::{Overlay, SettingsTab, command_suggestions};
use crate::ui::reader::viewport_of;
use crate::ui::theme::BUILT_IN_THEMES;

impl Yomu {
    pub(crate) fn in_cmdline(&self) -> bool {
        if let Some(reader) = &self.reader {
            reader.vim.in_cmdline()
        } else {
            self.library_vim.in_cmdline()
        }
    }

    pub(crate) fn cmdline_text(&self) -> Option<&str> {
        if let Some(reader) = &self.reader {
            reader.vim.cmdline_text()
        } else {
            self.library_vim.cmdline_text()
        }
    }

    pub(crate) fn replace_cmdline(&mut self, text: String) {
        if let Some(reader) = self.reader.as_mut() {
            reader.vim.replace_cmdline(text);
        } else {
            self.library_vim.replace_cmdline(text);
        }
    }

    pub(crate) fn cancel_cmdline(&mut self, cx: &mut Context<Self>) {
        if let Some(reader) = self.reader.as_mut() {
            reader.vim.cancel_cmdline();
        } else {
            self.library_vim.cancel_cmdline();
        }
        self.command_selection = 0;
        self.restore_theme_preview(cx);
        cx.notify();
    }

    pub(crate) fn sync_theme_preview(&mut self, draft: &str, cx: &mut Context<Self>) {
        let trimmed = draft.trim_start();
        let words = trimmed.split_whitespace();
        let first_word = words.into_iter().next().unwrap_or_default();
        if matches!(first_word, "theme" | "colorscheme" | "colo" | "colors") {
            if self.theme_before_preview.is_none() {
                self.theme_before_preview = Some(self.library.settings().theme_name.clone());
            }
            let suggestions = command_suggestions(draft, &self.custom_themes);
            if let Some((_, label, _)) = suggestions.get(self.command_selection) {
                self.preview_theme(label, cx);
            }
        } else {
            self.restore_theme_preview(cx);
        }
    }

    pub(crate) fn restore_theme_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(prev) = self.theme_before_preview.take()
            && self.library.settings().theme_name != prev
        {
            self.preview_theme(&prev, cx);
        }
    }

    pub(crate) fn on_overlay_key(
        &mut self,
        token: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting {
            return;
        }
        if matches!(token, "enter" | "space")
            && let Some(Overlay::LibraryMenu(menu)) = self.overlay.as_ref()
        {
            let selected = menu.selected;
            self.run_library_menu_action(selected, window, cx);
            return;
        }
        match self.overlay.as_mut() {
            Some(Overlay::Menu(menu)) => match token {
                "escape" => self.close_overlay(cx),
                "down" | "j" | "tab" | "C-n" => {
                    menu.selected = menu::step(&menu.entries, menu.selected, true);
                    cx.notify();
                }
                "up" | "k" | "S-tab" | "C-p" => {
                    menu.selected = menu::step(&menu.entries, menu.selected, false);
                    cx.notify();
                }
                "home" => {
                    menu.selected = menu::step(&menu.entries, None, true);
                    cx.notify();
                }
                "end" => {
                    menu.selected = menu::step(&menu.entries, None, false);
                    cx.notify();
                }
                "enter" | "space" => {
                    let picked = menu.selected.and_then(|ix| match &menu.entries[ix] {
                        Entry::Item(item) if item.enabled => Some(item.action),
                        _ => None,
                    });
                    let id = menu.book_id.clone();
                    let target_ids = menu.target_ids.clone();
                    if let Some(action) = picked {
                        self.run_book_action(action, &id, &target_ids, window, cx);
                    }
                }
                _ => {}
            },
            Some(Overlay::LibraryMenu(menu)) => match token {
                "escape" => self.close_overlay(cx),
                "down" | "j" | "tab" | "C-n" => {
                    menu.selected = (menu.selected + 1) % 5;
                    cx.notify();
                }
                "up" | "k" | "S-tab" | "C-p" => {
                    menu.selected = (menu.selected + 4) % 5;
                    cx.notify();
                }
                "home" => {
                    menu.selected = 0;
                    cx.notify();
                }
                "end" => {
                    menu.selected = 4;
                    cx.notify();
                }
                _ => {}
            },
            Some(Overlay::Confirm(_)) => match token {
                "enter" | "y" => self.confirm_accept(window, cx),
                "escape" | "n" | "q" => self.close_overlay(cx),
                _ => {}
            },
            Some(Overlay::Properties(_)) => {
                if matches!(token, "escape" | "enter" | "space" | "q") {
                    self.close_overlay(cx);
                }
            }
            Some(Overlay::Series(_)) => {}
            Some(Overlay::Collection(_)) => {
                if matches!(token, "escape") {
                    self.close_overlay(cx);
                }
            }
            Some(Overlay::Settings) => {
                if matches!(token, "escape" | "enter" | "q") {
                    self.close_overlay(cx);
                }
            }
            None => {}
        }
    }

    pub(crate) fn on_library_key(
        &mut self,
        token: &str,
        _key_char: Option<&str>,
        _control: bool,
        _alt: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting {
            return;
        }
        if matches!(token, "C-f" | "/" | "slash" | "C-k") {
            self.activate_search(window, cx);
            return;
        }
        if self.search_active {
            match token {
                "escape" => {
                    if self.search_query.is_empty() {
                        self.deactivate_search(cx);
                    } else {
                        self.search_query.clear();
                        if let Some(input) = &self.search_input {
                            input.update(cx, |state, cx| {
                                state.set_value("", window, cx);
                            });
                        }
                        cx.notify();
                    }
                }
                "enter" => {
                    self.deactivate_search(cx);
                }
                _ => {}
            }
            return;
        }
        match token {
            "C-a" => {
                let order = self.current_view_order();
                self.selection.select_all(&order);
                cx.notify();
            }
            "escape" | "backspace" => {
                if !self.selection.is_empty() {
                    self.selection.clear();
                    cx.notify();
                } else if self.open_group.is_some() {
                    self.exit_group(cx);
                }
            }
            "delete" => self.request_remove_selected(cx),
            _ => {}
        }
    }

    pub(crate) fn on_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting {
            return;
        }
        let keystroke = &event.keystroke;
        let mods = &keystroke.modifiers;
        if mods.platform && !cfg!(target_os = "macos") {
            return;
        }
        let primary_ctrl = mods.control || (cfg!(target_os = "macos") && mods.platform);
        let token = vim::key_token(
            &keystroke.key,
            keystroke.key_char.as_deref(),
            primary_ctrl,
            mods.alt,
            mods.shift,
        );

        // An open menu or dialog gets every key; nothing leaks through.
        if self.overlay.is_some() {
            if matches!(self.overlay, Some(Overlay::Series(_))) {
                if token == "escape" {
                    self.close_overlay(cx);
                }
                return;
            }
            self.on_overlay_key(&token, window, cx);
            return;
        }

        // Active command line prompt (works in both reader and library)
        if self.in_cmdline() {
            let draft = self.cmdline_text().map(str::to_owned).unwrap_or_default();
            if matches!(token.as_str(), "up" | "down" | "tab" | "S-tab") {
                let suggestions = command_suggestions(&draft, &self.custom_themes);
                if !suggestions.is_empty() {
                    let count = suggestions.len();
                    let backwards = token == "up" || token == "S-tab";
                    self.command_selection = if backwards {
                        self.command_selection.wrapping_add(count).wrapping_sub(1) % count
                    } else {
                        (self.command_selection + 1) % count
                    };
                    if token == "tab" || token == "S-tab" {
                        self.replace_cmdline(suggestions[self.command_selection].0.clone());
                        self.command_selection = 0;
                    }
                    self.sync_theme_preview(&draft, cx);
                }
                cx.notify();
                return;
            }

            if token == "escape" {
                self.cancel_cmdline(cx);
                return;
            }

            if token == "enter" {
                let suggestions = command_suggestions(&draft, &self.custom_themes);
                if !suggestions.is_empty() && self.command_selection < suggestions.len() {
                    self.replace_cmdline(suggestions[self.command_selection].0.clone());
                }
            }

            let vp = viewport_of(window);
            let actions = if let Some(reader) = self.reader.as_mut() {
                reader.vim.feed(&token)
            } else {
                self.library_vim.feed(&token)
            };
            self.command_selection = 0;
            if let Some(new_draft) = self.cmdline_text().map(str::to_owned) {
                self.sync_theme_preview(&new_draft, cx);
            } else {
                let trimmed = draft.trim_start();
                let words = trimmed.split_whitespace();
                let first_word = words.into_iter().next().unwrap_or_default();
                if !matches!(first_word, "theme" | "colorscheme" | "colo" | "colors") {
                    self.restore_theme_preview(cx);
                }
            }
            cx.notify();
            for action in actions {
                self.apply(action, vp, window, cx);
            }
            return;
        }

        let vim_enabled = self.is_vim_enabled();
        let Some(reader) = self.reader.as_mut() else {
            if vim_enabled && token == ":" && !self.search_active {
                self.library_vim.feed(":");
                self.command_selection = 0;
                cx.notify();
                return;
            }
            self.on_library_key(
                &token,
                keystroke.key_char.as_deref(),
                mods.control,
                mods.alt,
                window,
                cx,
            );
            return;
        };

        if reader.show_help && matches!(token.as_str(), "escape" | "?" | "q") {
            reader.show_help = false;
            cx.notify();
            return;
        }

        let vp = viewport_of(window);
        if vim_enabled {
            let actions = reader.vim.feed(&token);
            self.command_selection = 0;
            cx.notify();
            for action in actions {
                self.apply(action, vp, window, cx);
            }
        } else {
            let center = (vp.width / 2.0, vp.height / 2.0);
            let anchor = self.cursor_pos.unwrap_or(center);
            match token.as_str() {
                "escape" => {
                    self.close_book(window, cx);
                }
                "?" => {
                    reader.show_help = true;
                    cx.notify();
                }
                "right" | "space" => {
                    self.turn_by(true, 1, Arrive::Start, cx);
                }
                "left" | "S-space" => {
                    self.turn_by(false, 1, Arrive::Start, cx);
                }
                "down" => {
                    self.scroll(Dir::Down, Amount::Line, 1, vp, cx);
                }
                "up" => {
                    self.scroll(Dir::Up, Amount::Line, 1, vp, cx);
                }
                "pagedown" => {
                    self.scroll(Dir::Down, Amount::Page, 1, vp, cx);
                }
                "pageup" => {
                    self.scroll(Dir::Up, Amount::Page, 1, vp, cx);
                }
                "home" => {
                    self.jump_to(0, cx);
                }
                "end" => {
                    self.jump_to(usize::MAX, cx);
                }
                "+" | "=" | "C-=" | "C-plus" => {
                    self.zoom_by(layout::ZOOM_STEP, anchor, vp, cx);
                }
                "-" | "C-minus" => {
                    self.zoom_by(layout::ZOOM_STEP.recip(), anchor, vp, cx);
                }
                "0" | "C-0" => {
                    self.zoom_to(1.0, anchor, vp, cx);
                }
                "v" => {
                    let next = !self.library.settings().continuous;
                    self.set_continuous(Some(next), cx);
                }
                "w" => {
                    let next = !self.library.settings().webcomics;
                    self.set_webcomics(Some(next), cx);
                }
                "f" => {
                    let next = self
                        .reader
                        .as_ref()
                        .map(|r| r.fit.next())
                        .unwrap_or(FitMode::Page);
                    self.set_fit(next, cx);
                }
                _ => {}
            }
        }
    }

    pub(crate) fn apply(
        &mut self,
        action: Action,
        vp: Viewport,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting && !matches!(action, Action::QuitApp) {
            return;
        }
        let center = (vp.width / 2.0, vp.height / 2.0);
        let anchor = self.cursor_pos.unwrap_or(center);
        match action {
            Action::Scroll { dir, amount, count } => self.scroll(dir, amount, count, vp, cx),
            Action::NextPage(n) => self.turn_by(true, n, Arrive::Start, cx),
            Action::PrevPage(n) => self.turn_by(false, n, Arrive::Start, cx),
            Action::GotoPage(n) => self.jump_to(n.saturating_sub(1), cx),
            Action::FirstPage => self.jump_to(0, cx),
            Action::LastPage => self.jump_to(usize::MAX, cx),
            Action::ZoomIn(n) => {
                self.zoom_by(layout::ZOOM_STEP.powi(n.min(64) as i32), anchor, vp, cx)
            }
            Action::ZoomOut(n) => {
                self.zoom_by(layout::ZOOM_STEP.powi(-(n.min(64) as i32)), anchor, vp, cx)
            }
            Action::ZoomReset => self.zoom_to(1.0, anchor, vp, cx),
            Action::ZoomSet(zoom) => self.zoom_to(zoom, anchor, vp, cx),
            Action::Fit(mode) => self.set_fit(mode, cx),
            Action::CycleFit => {
                if let Some(next) = self.reader.as_ref().map(|r| r.fit.next()) {
                    self.set_fit(next, cx);
                } else {
                    let next = self.library.settings().fit.next();
                    self.set_fit(next, cx);
                }
            }
            Action::ScrollTop => self.scroll_vertical_to(0.0, vp, cx),
            Action::ScrollCenter => self.scroll_vertical_to(0.5, vp, cx),
            Action::ScrollBottom => self.scroll_vertical_to(1.0, vp, cx),
            Action::SetMark(key) => {
                let Some((id, page)) = self.reader.as_ref().map(|r| (r.book_id.clone(), r.page))
                else {
                    return;
                };
                if let Some(book) = self.library.get_mut(&id) {
                    book.marks.insert(key.to_string(), page);
                }
                self.save_library(cx);
                self.show_hud(format!("Mark {key} set on page {}", page + 1), cx);
            }
            Action::JumpMark(key) => {
                let Some(id) = self.reader.as_ref().map(|r| r.book_id.clone()) else {
                    return;
                };
                let target = self
                    .library
                    .get(&id)
                    .and_then(|book| book.marks.get(&key.to_string()).copied());
                match target {
                    Some(page) => self.jump_to(page, cx),
                    None => self.show_hud(format!("No mark {key}"), cx),
                }
            }
            Action::JumpBack => {
                let Some(reader) = self.reader.as_mut() else {
                    return;
                };
                match reader.back_page {
                    Some(previous) => {
                        reader.back_page = Some(reader.page);
                        self.goto_page(previous, Arrive::Start, cx);
                    }
                    None => self.show_hud("No previous position", cx),
                }
            }
            Action::SetAnimations(value) => self.set_animations(value, cx),
            Action::SetContinuous(value) => self.set_continuous(value, cx),
            Action::SetWebcomics(value) => self.set_webcomics(value, cx),
            Action::SetRememberZoom(value) => self.set_remember_zoom(value, cx),
            Action::ToggleInfo => {
                if let Some(reader) = self.reader.as_mut() {
                    reader.show_info = !reader.show_info;
                    cx.notify();
                }
            }
            Action::ToggleHelp => {
                if let Some(reader) = self.reader.as_mut() {
                    reader.show_help = !reader.show_help;
                    cx.notify();
                } else {
                    self.settings_tab = SettingsTab::Shortcuts;
                    self.open_settings(cx);
                }
            }
            Action::ToggleFullscreen => window.toggle_fullscreen(),
            Action::OpenSettings => self.open_settings_window(window, cx),
            Action::CloseBook => {
                if self.reader.is_some() {
                    self.close_book(window, cx);
                } else if self.open_group.is_some() {
                    self.exit_group(cx);
                } else if !self.selection.is_empty() {
                    self.selection.clear();
                    cx.notify();
                }
            }
            Action::QuitApp => {
                self.request_quit(cx);
            }
            Action::SetTheme(name) => {
                let name = name.trim();
                if name.is_empty() {
                    self.cycle_theme(1, cx);
                    self.show_toast(format!("Theme: {}", self.library.settings().theme_name), cx);
                } else {
                    let mut matched = self
                        .custom_themes
                        .keys()
                        .find(|k| k.eq_ignore_ascii_case(name))
                        .cloned();
                    if matched.is_none() {
                        matched = BUILT_IN_THEMES
                            .keys()
                            .find(|k| k.eq_ignore_ascii_case(name))
                            .cloned();
                    }
                    if matched.is_none() {
                        let lower = name.to_lowercase();
                        matched = self
                            .custom_themes
                            .keys()
                            .find(|k| k.to_lowercase().contains(&lower))
                            .or_else(|| {
                                BUILT_IN_THEMES
                                    .keys()
                                    .find(|k| k.to_lowercase().contains(&lower))
                            })
                            .cloned();
                    }
                    if let Some(theme_name) = matched {
                        self.apply_theme(&theme_name, cx);
                        self.show_toast(format!("Theme: {theme_name}"), cx);
                    } else {
                        self.show_toast(format!("Theme “{name}” is unavailable"), cx);
                    }
                }
                self.theme_before_preview = None;
            }
            Action::Message(text) => self.show_hud(text, cx),
        }
    }
}
