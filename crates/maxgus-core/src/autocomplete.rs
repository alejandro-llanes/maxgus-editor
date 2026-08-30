//! Suggestions while typing.
//!
//! `company-mode` for Emacs, and what every other editor calls autocomplete:
//! type a few characters, the language server is asked what could follow,
//! and a list appears beside the cursor. Narrowing it is typing more; taking
//! one is `RET`.
//!
//! Everything here is a plain value with no editor in it, so what happens to
//! the list as someone types can be tested without a screen: the list, what
//! is selected, what a prefix leaves, and what accepting would insert.

use maxgus_text::BufferId;

/// One thing a language server offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// What to show.
    pub label: String,
    /// What to type. Usually the label, but a snippet-ish server sends
    /// something else, and a server that sends nothing means the label.
    pub insert: String,
    /// `function`, `variable`, `field` — the word for what it is, or empty.
    pub kind: &'static str,
    /// A type or signature to show beside it, or empty.
    pub detail: String,
}

impl Item {
    pub fn new(label: impl Into<String>) -> Item {
        let label = label.into();
        Item {
            insert: label.clone(),
            label,
            kind: "",
            detail: String::new(),
        }
    }
}

/// The list of suggestions currently on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Autocomplete {
    /// The buffer it belongs to. Suggestions for one buffer must not be
    /// accepted into another.
    pub buffer: BufferId,
    /// Where the word being completed starts. Accepting replaces everything
    /// from here to point, which is what makes a partly-typed word turn
    /// into the whole one rather than gain a second copy of its tail.
    pub start: usize,
    /// Everything the server offered.
    items: Vec<Item>,
    /// Which of them the typed prefix leaves, best first.
    matching: Vec<usize>,
    selected: usize,
    /// The first row on show, so a long list scrolls with the selection.
    top: usize,
}

/// How many rows the list shows at once.
pub const ROWS: usize = 8;

impl Autocomplete {
    /// A list for `items`, narrowed by whatever has been typed already.
    pub fn new(buffer: BufferId, start: usize, prefix: &str, items: Vec<Item>) -> Autocomplete {
        let mut list = Autocomplete {
            buffer,
            start,
            items,
            matching: Vec::new(),
            selected: 0,
            top: 0,
        };
        list.narrow(prefix);
        list
    }

    /// True when nothing the server offered survives what has been typed,
    /// which is when the list should go away rather than sit there empty.
    pub fn is_empty(&self) -> bool {
        self.matching.is_empty()
    }

    pub fn len(&self) -> usize {
        self.matching.len()
    }

    /// Everything still matching, in the order shown.
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.matching.iter().filter_map(|n| self.items.get(*n))
    }

    pub fn selected(&self) -> Option<&Item> {
        self.items.get(*self.matching.get(self.selected)?)
    }

    /// Which row of the list is selected, counting from the first shown.
    pub fn selected_row(&self) -> usize {
        self.selected.saturating_sub(self.top)
    }

    /// The rows on show, and the index of the first.
    pub fn visible(&self) -> (usize, impl Iterator<Item = &Item>) {
        (
            self.top,
            self.matching
                .iter()
                .skip(self.top)
                .take(ROWS)
                .filter_map(|n| self.items.get(*n)),
        )
    }

    /// Keeps what still matches `prefix`.
    ///
    /// Fuzzy, the way `M-x` is: `sbf` finds `switch_buffer`. A prefix that
    /// matches nothing leaves the list empty, and the caller closes it.
    pub fn narrow(&mut self, prefix: &str) {
        let selected = self.selected().cloned();
        let mut scored: Vec<(i32, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(n, item)| {
                crate::fuzzy::score(prefix, &item.label).map(|score| (score, n))
            })
            .collect();
        // Best first, and alphabetically among equals so the order does not
        // shuffle as the score changes.
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| self.items[a.1].label.cmp(&self.items[b.1].label))
        });
        self.matching = scored.into_iter().map(|(_, n)| n).collect();
        // Keep pointing at whatever was selected, when it survived.
        self.selected = selected
            .and_then(|was| self.items().position(|item| *item == was))
            .unwrap_or(0);
        self.scroll_to_selection();
    }

    pub fn next(&mut self) {
        if self.matching.is_empty() {
            return;
        }
        // Wrapping, because a list that stops at the end makes someone look
        // to see whether they are at the end.
        self.selected = (self.selected + 1) % self.matching.len();
        self.scroll_to_selection();
    }

    pub fn previous(&mut self) {
        if self.matching.is_empty() {
            return;
        }
        self.selected = match self.selected {
            0 => self.matching.len() - 1,
            n => n - 1,
        };
        self.scroll_to_selection();
    }

    fn scroll_to_selection(&mut self) {
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + ROWS {
            self.top = self.selected + 1 - ROWS;
        }
        // A list that shrank under a window scrolled past its end.
        let last = self.matching.len().saturating_sub(ROWS);
        self.top = self.top.min(last);
    }
}

/// Where the word before `point` begins.
///
/// What is being completed, and what accepting replaces. A `.` or `::` ends
/// it, which is what makes `foo.ba` complete `ba` rather than `foo.ba`.
pub fn word_start(text: &str, point: usize) -> usize {
    let mut start = point.min(text.chars().count());
    let chars: Vec<char> = text.chars().collect();
    while start > 0 {
        let ch = chars[start - 1];
        if ch.is_alphanumeric() || ch == '_' {
            start -= 1;
        } else {
            break;
        }
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(labels: &[&str]) -> Vec<Item> {
        labels.iter().map(|l| Item::new(*l)).collect()
    }

    fn list(prefix: &str, labels: &[&str]) -> Autocomplete {
        Autocomplete::new(BufferId(1), 0, prefix, items(labels))
    }

    #[test]
    fn typing_narrows_the_list() {
        let mut a = list("", &["push", "push_str", "pop", "len"]);
        assert_eq!(a.len(), 4);
        a.narrow("pu");
        let shown: Vec<&str> = a.items().map(|i| i.label.as_str()).collect();
        assert_eq!(shown, ["push", "push_str"], "got {shown:?}");
        a.narrow("pus_s");
        assert_eq!(
            a.items().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            ["push_str"],
            "the match is a subsequence, not a prefix"
        );
    }

    #[test]
    fn a_prefix_that_matches_nothing_empties_it() {
        let mut a = list("", &["push", "pop"]);
        a.narrow("zzz");
        assert!(a.is_empty());
        assert!(a.selected().is_none(), "an empty list selects nothing");
    }

    #[test]
    fn the_selection_survives_the_list_narrowing_under_it() {
        // Typing another letter should not throw away what was highlighted,
        // as long as it is still there.
        let mut a = list("p", &["push", "push_str", "pop"]);
        a.next();
        let chosen = a.selected().cloned().expect("something");
        a.narrow("pu");
        assert_eq!(
            a.selected().map(|i| i.label.clone()),
            Some(chosen.label),
            "the highlight moved to something else"
        );
    }

    #[test]
    fn a_selection_that_did_not_survive_falls_back_to_the_first() {
        let mut a = list("p", &["push", "pop"]);
        a.narrow("po");
        assert_eq!(a.selected().map(|i| i.label.as_str()), Some("pop"));
        // `push` is gone; the highlight cannot stay on it.
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn moving_wraps_at_both_ends() {
        let mut a = list("", &["one", "two", "three"]);
        a.previous();
        assert_eq!(
            a.selected().map(|i| i.label.as_str()),
            Some("two"),
            "the last"
        );
        a.next();
        assert_eq!(a.selected().map(|i| i.label.as_str()), Some("one"));
    }

    #[test]
    fn a_long_list_scrolls_with_the_selection() {
        let labels: Vec<String> = (0..30).map(|n| format!("item{n:02}")).collect();
        let mut a = Autocomplete::new(
            BufferId(1),
            0,
            "",
            labels.iter().map(|l| Item::new(l.clone())).collect(),
        );
        let (top, _) = a.visible();
        assert_eq!(top, 0);
        for _ in 0..ROWS {
            a.next();
        }
        let (top, shown) = a.visible();
        assert!(top > 0, "the window did not follow the selection");
        assert_eq!(shown.count(), ROWS, "a full window is still full");
        assert!(
            a.selected_row() < ROWS,
            "the selection is off the bottom of its own window"
        );
        // And back round the top.
        a.previous();
        for _ in 0..ROWS + 2 {
            a.previous();
        }
        assert!(a.selected_row() < ROWS);
    }

    #[test]
    fn the_window_never_scrolls_past_the_end_of_a_list_that_shrank() {
        let labels: Vec<String> = (0..30).map(|n| format!("item{n:02}")).collect();
        let mut a = Autocomplete::new(
            BufferId(1),
            0,
            "",
            labels.iter().map(|l| Item::new(l.clone())).collect(),
        );
        for _ in 0..25 {
            a.next();
        }
        a.narrow("item01");
        let (top, shown) = a.visible();
        assert_eq!(top, 0, "a one-line list starts at the top");
        assert_eq!(shown.count(), 1);
    }

    #[test]
    fn the_word_being_completed_is_what_accepting_replaces() {
        assert_eq!(word_start("let x = foo", 11), 8, "`foo`");
        assert_eq!(word_start("obj.fie", 7), 4, "a dot ends the word");
        assert_eq!(word_start("a::bee", 6), 3, "so does a colon");
        assert_eq!(word_start("with_under", 10), 0, "an underscore does not");
        assert_eq!(word_start("    ", 4), 4, "nothing to complete");
        assert_eq!(word_start("", 0), 0);
    }
}
