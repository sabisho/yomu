//! Vim-style key handling for the reader: counts (`5j`), prefixes (`gg`, `zw`, `ma`, `'a`).
//! It turns key tokens into [`Action`]s and knows nothing about the GUI.

use crate::layout::FitMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Amount {
    Line,
    HalfPage,
    Page,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Scroll the page; turns the page when it cannot scroll any further.
    Scroll {
        dir: Dir,
        amount: Amount,
        count: usize,
    },
    NextPage(usize),
    PrevPage(usize),
    /// 1-based page number.
    GotoPage(usize),
    FirstPage,
    LastPage,
    ZoomIn(usize),
    ZoomOut(usize),
    ZoomReset,
    /// Multiplier relative to the current fit size (`1.5` = 150%).
    ZoomSet(f32),
    Fit(FitMode),
    CycleFit,
    ScrollTop,
    ScrollBottom,
    ScrollCenter,
    SetMark(char),
    JumpMark(char),
    JumpBack,
    /// `None` toggles continuous reading mode.
    SetContinuous(Option<bool>),
    /// `None` toggles gapless webcomics mode.
    SetWebcomics(Option<bool>),
    /// `None` toggles whether custom zoom is retained across books and sessions.
    SetRememberZoom(Option<bool>),
    /// `None` toggles double-page spread mode.
    SetDoublePage(Option<bool>),
    /// Cycles spread override for current page.
    CycleSpreadOverride,
    ToggleInfo,
    ToggleHelp,
    ToggleFullscreen,
    OpenSettings,
    CloseBook,
    QuitApp,
    /// Set or cycle the UI theme. An empty string cycles to the next theme.
    SetTheme(String),
    /// Something to tell the user (for example a bad command).
    Message(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prefix {
    G,
    Z,
    Mark,
    Jump,
}

#[derive(Debug, Default)]
pub struct Vim {
    count: Option<usize>,
    prefix: Option<Prefix>,
}

fn scroll(dir: Dir, amount: Amount, count: usize) -> Vec<Action> {
    vec![Action::Scroll { dir, amount, count }]
}

fn digit(token: &str) -> Option<usize> {
    let mut chars = token.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => c.to_digit(10).map(|d| d as usize),
        _ => None,
    }
}

fn single_char(token: &str) -> Option<char> {
    let mut chars = token.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

impl Vim {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Text for the small status line shown while something is half-typed (`12`, `g`).
    pub fn status(&self) -> Option<String> {
        if self.count.is_none() && self.prefix.is_none() {
            return None;
        }
        let mut text = self.count.map(|c| c.to_string()).unwrap_or_default();
        let suffix = match self.prefix {
            Some(Prefix::G) => Some('g'),
            Some(Prefix::Z) => Some('z'),
            Some(Prefix::Mark) => Some('m'),
            Some(Prefix::Jump) => Some('\''),
            None => None,
        };
        if let Some(c) = suffix {
            text.push(c);
        }
        Some(text)
    }

    /// Feed one key (see [`key_token`]) and get back what should happen.
    pub fn feed(&mut self, token: &str) -> Vec<Action> {
        if token == "escape" {
            // Escape first cancels anything half-typed; on a clean slate it leaves the book.
            return if self.count.is_some() || self.prefix.is_some() {
                self.reset();
                vec![]
            } else {
                vec![Action::CloseBook]
            };
        }

        if let Some(prefix) = self.prefix.take() {
            let count = self.count.take();
            return Self::feed_prefixed(prefix, count, token);
        }

        if let Some(d) = digit(token) {
            // `0` on its own is "reset zoom"; after other digits it extends the count (`10j`).
            if d != 0 || self.count.is_some() {
                let current = self.count.unwrap_or(0);
                self.count = Some(current.saturating_mul(10).saturating_add(d).min(99_999));
                return vec![];
            }
        }

        let count = self.count.take();
        let n = count.unwrap_or(1);

        match token {
            "j" | "down" | "C-e" => scroll(Dir::Down, Amount::Line, n),
            "k" | "up" | "C-y" => scroll(Dir::Up, Amount::Line, n),
            "h" | "left" => scroll(Dir::Left, Amount::Line, n),
            "l" | "right" => scroll(Dir::Right, Amount::Line, n),
            "C-d" => scroll(Dir::Down, Amount::HalfPage, n),
            "C-u" => scroll(Dir::Up, Amount::HalfPage, n),
            "space" | "C-f" => scroll(Dir::Down, Amount::Page, n),
            "S-space" | "C-b" => scroll(Dir::Up, Amount::Page, n),
            "J" | "pagedown" => vec![Action::NextPage(n)],
            "K" | "pageup" => vec![Action::PrevPage(n)],
            "g" => {
                self.prefix = Some(Prefix::G);
                self.count = count;
                vec![]
            }
            "G" => vec![match count {
                Some(page) => Action::GotoPage(page),
                None => Action::LastPage,
            }],
            "home" => vec![Action::FirstPage],
            "end" => vec![Action::LastPage],
            "z" => {
                self.prefix = Some(Prefix::Z);
                self.count = count;
                vec![]
            }
            "+" | "=" | "C-=" | "C-+" => vec![Action::ZoomIn(n)],
            "-" | "C--" => vec![Action::ZoomOut(n)],
            "0" | "C-0" => vec![Action::ZoomReset],
            "a" => vec![Action::Fit(FitMode::Page)],
            "s" => vec![Action::Fit(FitMode::Width)],
            "tab" => vec![Action::CycleFit],
            "m" => {
                self.prefix = Some(Prefix::Mark);
                vec![]
            }
            "'" | "`" => {
                self.prefix = Some(Prefix::Jump);
                vec![]
            }
            "v" => vec![Action::SetContinuous(None)],
            "w" => vec![Action::SetWebcomics(None)],
            "d" => vec![Action::SetDoublePage(None)],
            "i" => vec![Action::ToggleInfo],
            "?" => vec![Action::ToggleHelp],
            "f" => vec![Action::ToggleFullscreen],
            "q" => vec![Action::CloseBook],
            _ => vec![],
        }
    }

    fn feed_prefixed(prefix: Prefix, count: Option<usize>, token: &str) -> Vec<Action> {
        match (prefix, token) {
            (Prefix::G, "g") => vec![match count {
                Some(page) => Action::GotoPage(page),
                None => Action::FirstPage,
            }],
            (Prefix::Z, "i") => vec![Action::ZoomIn(count.unwrap_or(1))],
            (Prefix::Z, "o") => vec![Action::ZoomOut(count.unwrap_or(1))],
            (Prefix::Z, "0") => vec![Action::ZoomReset],
            (Prefix::Z, "p" | "a") => vec![Action::Fit(FitMode::Page)],
            (Prefix::Z, "w") => vec![Action::Fit(FitMode::Width)],
            (Prefix::Z, "h") => vec![Action::Fit(FitMode::Height)],
            (Prefix::Z, "1") => vec![Action::Fit(FitMode::Original)],
            (Prefix::Z, "t") => vec![Action::ScrollTop],
            (Prefix::Z, "b") => vec![Action::ScrollBottom],
            (Prefix::Z, "z") => vec![Action::ScrollCenter],
            (Prefix::Z, "d") => vec![Action::SetDoublePage(None)],
            (Prefix::Z, "s") => vec![Action::CycleSpreadOverride],
            (Prefix::Mark, key) => match single_char(key) {
                Some(c) if c.is_alphabetic() => vec![Action::SetMark(c)],
                _ => vec![],
            },
            (Prefix::Jump, "'" | "`") => vec![Action::JumpBack],
            (Prefix::Jump, key) => match single_char(key) {
                Some(c) if c.is_alphabetic() => vec![Action::JumpMark(c)],
                _ => vec![],
            },
            _ => vec![],
        }
    }
}

/// Turn a key press into the token [`Vim::feed`] understands: `j`, `G`, `:`, `C-d`, `space`,
/// `S-space`, `escape`, `left`, and so on.
pub fn key_token(key: &str, key_char: Option<&str>, ctrl: bool, alt: bool, shift: bool) -> String {
    let named = key.chars().count() > 1;
    let base = if named || ctrl || alt {
        key.to_string()
    } else {
        match key_char {
            Some(text) if !text.is_empty() => text.to_string(),
            _ if shift => key.to_uppercase(),
            _ => key.to_string(),
        }
    };

    let mut token = String::new();
    if ctrl {
        token.push_str("C-");
    }
    if alt {
        token.push_str("A-");
    }
    if shift && (named || ctrl) {
        token.push_str("S-");
    }
    token.push_str(&base);
    token
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(vim: &mut Vim, tokens: &[&str]) -> Vec<Action> {
        tokens.iter().flat_map(|t| vim.feed(t)).collect()
    }

    #[test]
    fn counts_multiply_motions() {
        let mut vim = Vim::new();
        assert_eq!(
            feed_all(&mut vim, &["5", "j"]),
            vec![Action::Scroll {
                dir: Dir::Down,
                amount: Amount::Line,
                count: 5
            }]
        );
        assert_eq!(
            feed_all(&mut vim, &["1", "0", "k"]),
            vec![Action::Scroll {
                dir: Dir::Up,
                amount: Amount::Line,
                count: 10
            }]
        );
    }

    #[test]
    fn zero_alone_resets_zoom_but_extends_a_count() {
        let mut vim = Vim::new();
        assert_eq!(feed_all(&mut vim, &["0"]), vec![Action::ZoomReset]);
        assert_eq!(
            feed_all(&mut vim, &["2", "0", "J"]),
            vec![Action::NextPage(20)]
        );
    }

    #[test]
    fn gg_and_g() {
        let mut vim = Vim::new();
        assert_eq!(feed_all(&mut vim, &["g", "g"]), vec![Action::FirstPage]);
        assert_eq!(feed_all(&mut vim, &["G"]), vec![Action::LastPage]);
        assert_eq!(
            feed_all(&mut vim, &["1", "2", "G"]),
            vec![Action::GotoPage(12)]
        );
        assert_eq!(
            feed_all(&mut vim, &["7", "g", "g"]),
            vec![Action::GotoPage(7)]
        );
    }

    #[test]
    fn view_commands_under_z() {
        let mut vim = Vim::new();
        assert_eq!(
            feed_all(&mut vim, &["z", "w"]),
            vec![Action::Fit(FitMode::Width)]
        );
        assert_eq!(
            feed_all(&mut vim, &["z", "h"]),
            vec![Action::Fit(FitMode::Height)]
        );
        assert_eq!(
            feed_all(&mut vim, &["z", "p"]),
            vec![Action::Fit(FitMode::Page)]
        );
        assert_eq!(
            feed_all(&mut vim, &["z", "1"]),
            vec![Action::Fit(FitMode::Original)]
        );
        assert_eq!(feed_all(&mut vim, &["z", "i"]), vec![Action::ZoomIn(1)]);
        assert_eq!(
            feed_all(&mut vim, &["3", "z", "o"]),
            vec![Action::ZoomOut(3)]
        );
        assert_eq!(feed_all(&mut vim, &["a"]), vec![Action::Fit(FitMode::Page)]);
        assert_eq!(
            feed_all(&mut vim, &["s"]),
            vec![Action::Fit(FitMode::Width)]
        );
        assert_eq!(feed_all(&mut vim, &["+"]), vec![Action::ZoomIn(1)]);
        assert_eq!(feed_all(&mut vim, &["-"]), vec![Action::ZoomOut(1)]);
        assert_eq!(
            feed_all(&mut vim, &["d"]),
            vec![Action::SetDoublePage(None)]
        );
        assert_eq!(
            feed_all(&mut vim, &["z", "d"]),
            vec![Action::SetDoublePage(None)]
        );
        assert_eq!(
            feed_all(&mut vim, &["z", "s"]),
            vec![Action::CycleSpreadOverride]
        );
    }

    #[test]
    fn marks_and_jump_back() {
        let mut vim = Vim::new();
        assert_eq!(feed_all(&mut vim, &["m", "a"]), vec![Action::SetMark('a')]);
        assert_eq!(feed_all(&mut vim, &["'", "a"]), vec![Action::JumpMark('a')]);
        assert_eq!(feed_all(&mut vim, &["'", "'"]), vec![Action::JumpBack]);
        assert!(feed_all(&mut vim, &["m", "5"]).is_empty());
    }

    #[test]
    fn escape_cancels_pending_then_closes() {
        let mut vim = Vim::new();
        assert!(feed_all(&mut vim, &["4", "escape"]).is_empty());
        assert!(vim.status().is_none());
        assert!(feed_all(&mut vim, &["g", "escape"]).is_empty());
        assert_eq!(feed_all(&mut vim, &["escape"]), vec![Action::CloseBook]);
    }

    #[test]
    fn status_shows_pending_input() {
        let mut vim = Vim::new();
        vim.feed("1");
        vim.feed("2");
        assert_eq!(vim.status().as_deref(), Some("12"));
        vim.feed("g");
        assert_eq!(vim.status().as_deref(), Some("12g"));
    }

    #[test]
    fn key_tokens() {
        assert_eq!(key_token("j", Some("j"), false, false, false), "j");
        assert_eq!(key_token("g", Some("G"), false, false, true), "G");
        assert_eq!(key_token(";", Some(":"), false, false, true), ":");
        assert_eq!(key_token("=", Some("+"), false, false, true), "+");
        assert_eq!(key_token("d", Some("d"), true, false, false), "C-d");
        assert_eq!(key_token("space", Some(" "), false, false, true), "S-space");
        assert_eq!(key_token("escape", None, false, false, false), "escape");
        assert_eq!(key_token("left", None, false, false, false), "left");
        // no key_char reported: fall back to the shifted key
        assert_eq!(key_token("g", None, false, false, true), "G");
    }
}
