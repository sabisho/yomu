#![forbid(unsafe_code)]

//! Keyboard shortcuts, Vim motions, and action dispatch.

use gpui_kit::{Context, KeyDownEvent, Window};

use yomu::layout::{self, Arrive, FitMode, Viewport};
use yomu::vim::{self, Action, Amount, Dir};

use crate::app::state::Yomu;
use crate::ui::overlays::{Overlay, SettingsTab};
use crate::ui::reader::viewport_of;
use crate::ui::theme::BUILT_IN_THEMES;

impl Yomu {
    pub(crate) fn on_overlay_key(
        &mut self,
        token: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_quitting {
            return;
        }
        match self.overlay.as_mut() {
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
                if matches!(token, "escape" | "enter" | "q" | "C-,") {
                    self.close_overlay_window(window, cx);
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
            "delete" => self.request_remove_selected(window, cx),
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

        let vim_enabled = self.is_vim_enabled();
        let Some(reader) = self.reader.as_mut() else {
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
                "space" => {
                    self.turn_by(true, 1, Arrive::Start, cx);
                }
                "S-space" => {
                    self.turn_by(false, 1, Arrive::Start, cx);
                }
                "right" => {
                    let is_rtl = self
                        .reader
                        .as_ref()
                        .map(|r| r.reading_direction.is_rtl())
                        .unwrap_or_else(|| self.library.settings().reading_direction.is_rtl());
                    self.turn_by(!is_rtl, 1, Arrive::Start, cx);
                }
                "left" => {
                    let is_rtl = self
                        .reader
                        .as_ref()
                        .map(|r| r.reading_direction.is_rtl())
                        .unwrap_or_else(|| self.library.settings().reading_direction.is_rtl());
                    self.turn_by(is_rtl, 1, Arrive::Start, cx);
                }
                "d" => {
                    let next = self
                        .reader
                        .as_ref()
                        .map(|r| r.page_layout.toggle())
                        .unwrap_or_else(|| self.library.settings().page_layout.toggle());
                    self.set_page_layout(next, cx);
                }
                "s" => {
                    if let Some(reader) = self.reader.as_ref() {
                        let cur = reader.current_spread_override();
                        let next = match cur {
                            yomu::library::PageSpreadOverride::Auto => {
                                yomu::library::PageSpreadOverride::ForceSpread
                            }
                            yomu::library::PageSpreadOverride::ForceSpread => {
                                yomu::library::PageSpreadOverride::ForceSingle
                            }
                            yomu::library::PageSpreadOverride::ForceSingle => {
                                yomu::library::PageSpreadOverride::Auto
                            }
                        };
                        self.set_page_spread_override(next, cx);
                    }
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
            Action::SetDoublePage(value) => {
                let next = match value {
                    Some(true) => yomu::layout::PageLayout::Double,
                    Some(false) => yomu::layout::PageLayout::Single,
                    None => {
                        let cur = self
                            .reader
                            .as_ref()
                            .map(|r| r.page_layout)
                            .unwrap_or_else(|| self.library.settings().page_layout);
                        cur.toggle()
                    }
                };
                self.set_page_layout(next, cx);
            }
            Action::CycleSpreadOverride => {
                if let Some(reader) = self.reader.as_ref() {
                    let cur = reader.current_spread_override();
                    let next = match cur {
                        yomu::library::PageSpreadOverride::Auto => {
                            yomu::library::PageSpreadOverride::ForceSpread
                        }
                        yomu::library::PageSpreadOverride::ForceSpread => {
                            yomu::library::PageSpreadOverride::ForceSingle
                        }
                        yomu::library::PageSpreadOverride::ForceSingle => {
                            yomu::library::PageSpreadOverride::Auto
                        }
                    };
                    self.set_page_spread_override(next, cx);
                }
            }
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
                    self.open_settings_window(window, cx);
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
            }
            Action::Message(text) => self.show_hud(text, cx),
        }
    }
}
