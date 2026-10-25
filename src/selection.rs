//! Which books are selected in the library. Pure logic (no gpui) so the click rules are tested.
//!
//! Rules, in the style of a file manager:
//! * Ctrl+click toggles one book.
//! * Shift+click selects the range from the last clicked book (the anchor) to this one.
//! * Once anything is selected the library is in *selection mode*: a plain click toggles a book
//!   instead of opening it, so you can keep clicking to build up a selection.

use std::collections::HashSet;

#[derive(Debug, Default, Clone)]
pub struct Selection {
    ids: HashSet<String>,
    /// The book a Shift+click range starts from.
    anchor: Option<String>,
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    pub fn clear(&mut self) {
        self.ids.clear();
        self.anchor = None;
    }

    pub fn remove(&mut self, id: &str) {
        self.ids.remove(id);
        if self.anchor.as_deref() == Some(id) {
            self.anchor = None;
        }
    }

    /// Selected ids in library order (so bulk actions and messages are deterministic).
    pub fn in_order(&self, order: &[String]) -> Vec<String> {
        order
            .iter()
            .filter(|id| self.ids.contains(*id))
            .cloned()
            .collect()
    }

    pub fn toggle(&mut self, id: &str) {
        if !self.ids.remove(id) {
            self.ids.insert(id.to_string());
        }
        self.anchor = Some(id.to_string());
    }

    pub fn select_all(&mut self, order: &[String]) {
        self.ids = order.iter().cloned().collect();
        self.anchor = order.first().cloned();
    }

    /// Select everything between the anchor and `id` (inclusive). Without an anchor (or if the
    /// anchor is gone) this just selects `id`. `extend` keeps the existing selection and adds the
    /// range; otherwise the range replaces it. The anchor stays put, like in a file manager.
    pub fn select_range(&mut self, order: &[String], id: &str, extend: bool) {
        let target = order.iter().position(|candidate| candidate == id);
        let anchor = self
            .anchor
            .as_deref()
            .and_then(|a| order.iter().position(|candidate| candidate == a));

        if !extend {
            self.ids.clear();
        }
        match (anchor, target) {
            (Some(a), Some(t)) => {
                let (low, high) = (a.min(t), a.max(t));
                self.ids.extend(order[low..=high].iter().cloned());
            }
            (None, Some(_)) => {
                self.ids.insert(id.to_string());
                self.anchor = Some(id.to_string());
            }
            _ => {}
        }
    }

    /// A click on a book card. Returns `true` if the click was handled as selection, or `false`
    /// if it is a plain click on an unselected library and should open the book instead.
    pub fn click(&mut self, order: &[String], id: &str, ctrl: bool, shift: bool) -> bool {
        if shift {
            self.select_range(order, id, ctrl);
            true
        } else if ctrl || !self.is_empty() {
            self.toggle(id);
            true
        } else {
            false
        }
    }

    /// Forget ids that are no longer in the library.
    pub fn retain_existing(&mut self, existing: &HashSet<String>) {
        self.ids.retain(|id| existing.contains(id));
        if self.anchor.as_ref().is_some_and(|a| !existing.contains(a)) {
            self.anchor = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order() -> Vec<String> {
        ["a", "b", "c", "d", "e"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn selected(selection: &Selection) -> Vec<String> {
        selection.in_order(&order())
    }

    #[test]
    fn plain_click_opens_until_something_is_selected() {
        let mut selection = Selection::new();
        assert!(
            !selection.click(&order(), "b", false, false),
            "plain click opens"
        );
        assert!(selection.is_empty());

        assert!(
            selection.click(&order(), "b", true, false),
            "ctrl+click selects"
        );
        assert_eq!(selected(&selection), vec!["b"]);

        // selection mode: plain clicks now toggle
        assert!(selection.click(&order(), "d", false, false));
        assert_eq!(selected(&selection), vec!["b", "d"]);
        assert!(selection.click(&order(), "b", false, false));
        assert_eq!(selected(&selection), vec!["d"]);
        assert!(selection.click(&order(), "d", false, false));
        assert!(
            selection.is_empty(),
            "deselecting the last book leaves selection mode"
        );
        assert!(
            !selection.click(&order(), "a", false, false),
            "plain click opens again"
        );
    }

    #[test]
    fn shift_click_selects_a_range_in_either_direction() {
        let mut selection = Selection::new();
        selection.toggle("b");
        selection.select_range(&order(), "d", false);
        assert_eq!(selected(&selection), vec!["b", "c", "d"]);

        // the anchor stays at b, so shift-clicking above it flips the range
        selection.select_range(&order(), "a", false);
        assert_eq!(selected(&selection), vec!["a", "b"]);
    }

    #[test]
    fn shift_click_without_an_anchor_selects_just_that_book() {
        let mut selection = Selection::new();
        assert!(selection.click(&order(), "c", false, true));
        assert_eq!(selected(&selection), vec!["c"]);
        selection.select_range(&order(), "e", false);
        assert_eq!(selected(&selection), vec!["c", "d", "e"]);
    }

    #[test]
    fn ctrl_shift_adds_a_range_to_the_existing_selection() {
        let mut selection = Selection::new();
        selection.toggle("a");
        selection.toggle("c");
        selection.click(&order(), "e", true, true);
        assert_eq!(selected(&selection), vec!["a", "c", "d", "e"]);
    }

    #[test]
    fn select_all_and_clear() {
        let mut selection = Selection::new();
        selection.select_all(&order());
        assert_eq!(selection.len(), 5);
        assert!(selection.contains("e"));
        selection.clear();
        assert!(selection.is_empty());
        selection.select_range(&order(), "c", false);
        assert_eq!(
            selected(&selection),
            vec!["c"],
            "the anchor was cleared too"
        );
    }

    #[test]
    fn removed_books_drop_out_of_the_selection() {
        let mut selection = Selection::new();
        selection.select_all(&order());
        let existing: HashSet<String> = ["a", "c"].iter().map(|s| s.to_string()).collect();
        selection.retain_existing(&existing);
        assert_eq!(selected(&selection), vec!["a", "c"]);

        selection.toggle("c");
        selection.retain_existing(&HashSet::new());
        assert!(selection.is_empty());
    }

    #[test]
    fn in_order_follows_the_library_not_the_click_order() {
        let mut selection = Selection::new();
        selection.toggle("d");
        selection.toggle("a");
        assert_eq!(selected(&selection), vec!["a", "d"]);
    }

    #[test]
    fn unknown_ids_are_ignored_by_ranges() {
        let mut selection = Selection::new();
        selection.toggle("b");
        selection.select_range(&order(), "zzz", false);
        assert!(
            selection.is_empty(),
            "a range to a book that does not exist selects nothing"
        );
    }
}
