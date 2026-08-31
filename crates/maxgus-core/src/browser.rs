//! A file browser you type at: a box over the frame, narrowing as you go.
//!
//! `C-x C-f` asks for a path and completes it, which is the right thing when
//! you know what you want to open. This is for the other case — when you
//! know roughly where it is and would rather look. Typing narrows the
//! listing fuzzily, the arrows walk it, and right and left go in and out of
//! directories, so finding something is a matter of looking at it rather
//! than of spelling it.
//!
//! Deliberately *not* dired. dired is for working on a directory — marking
//! a dozen files and doing one thing to all of them — and its single-letter
//! keys are what makes that quick. Those keys and typing to narrow want the
//! same keyboard, and the answer is two commands rather than one command
//! with a mode in it.
//!
//! The model is here and the drawing is [`crate::render`]'s, so what it
//! shows and what it selects can be checked without a window.

use crate::dired::Entry;
use std::path::{Path, PathBuf};

/// One row of the listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// `..`, which is offered whenever there is a directory above.
    Parent,
    /// An entry, by its index into [`Browser::entries`].
    Entry(usize),
}

/// A directory, what is in it, and what has been typed to narrow it.
#[derive(Debug, Clone, Default)]
pub struct Browser {
    pub directory: PathBuf,
    /// Everything in the directory, directories first and then by name.
    pub entries: Vec<Entry>,
    /// What has been typed. Narrows the listing fuzzily.
    pub filter: String,
    /// Which row the cursor is on, as an index into [`Browser::rows`].
    pub selected: usize,
    /// True while a directory is being read and there is nothing to show
    /// yet, so an empty box can say so rather than looking like an empty
    /// directory.
    pub pending: bool,
    rows: Vec<Row>,
}

impl Browser {
    /// A browser waiting for `directory` to be read.
    pub fn opening(directory: impl Into<PathBuf>) -> Browser {
        Browser {
            directory: directory.into(),
            pending: true,
            ..Browser::default()
        }
    }

    /// What was read. The filter is kept — a listing arriving should not
    /// undo what was typed while it was being read — but the cursor goes
    /// back to the top, because it was pointing into a different directory.
    pub fn listed(&mut self, directory: impl Into<PathBuf>, entries: Vec<Entry>) {
        self.directory = directory.into();
        self.entries = entries;
        // Directories first and then by name, which is the order anyone
        // walking a tree by eye expects. The reader gives no order at all.
        self.entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.pending = false;
        self.selected = 0;
        self.rebuild();
    }

    /// The rows as they are shown, narrowed by whatever has been typed.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The entry a row names, if it names one.
    pub fn entry(&self, row: Row) -> Option<&Entry> {
        match row {
            Row::Parent => None,
            Row::Entry(index) => self.entries.get(index),
        }
    }

    /// The row the cursor is on.
    pub fn current(&self) -> Option<Row> {
        self.rows.get(self.selected).copied()
    }

    /// Where the cursor is pointing: a file to open, or a directory to go
    /// into. `None` when the listing is empty.
    pub fn current_path(&self) -> Option<PathBuf> {
        match self.current()? {
            Row::Parent => self.parent(),
            Row::Entry(index) => Some(self.directory.join(&self.entries.get(index)?.name)),
        }
    }

    /// True when the cursor is on something to go into rather than open.
    pub fn current_is_dir(&self) -> bool {
        match self.current() {
            Some(Row::Parent) => true,
            Some(Row::Entry(index)) => self.entries.get(index).is_some_and(|e| e.is_dir),
            None => false,
        }
    }

    /// The directory above this one, when there is one.
    pub fn parent(&self) -> Option<PathBuf> {
        self.directory.parent().map(Path::to_path_buf)
    }

    /// How many of the entries survive the filter, and how many there are.
    ///
    /// For the count in the corner: a listing narrowed to three of forty is
    /// a different thing from a directory with three files in it.
    pub fn tally(&self) -> (usize, usize) {
        let shown = self
            .rows
            .iter()
            .filter(|row| matches!(row, Row::Entry(_)))
            .count();
        (shown, self.entries.len())
    }

    pub fn type_char(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
        self.rebuild();
    }

    /// Rubs out the last character typed. Says whether there was one — an
    /// empty filter is how the caller knows to go up a directory instead,
    /// which is what backspace does in every file browser.
    pub fn rub_out(&mut self) -> bool {
        if self.filter.pop().is_none() {
            return false;
        }
        self.selected = 0;
        self.rebuild();
        true
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.selected = 0;
        self.rebuild();
    }

    /// Down `n` rows, stopping at the bottom rather than wrapping: a list
    /// that wraps under a held-down arrow is a list you overshoot.
    pub fn next(&mut self, n: usize) {
        if self.rows.is_empty() {
            return;
        }
        self.selected = (self.selected + n).min(self.rows.len() - 1);
    }

    pub fn previous(&mut self, n: usize) {
        self.selected = self.selected.saturating_sub(n);
    }

    pub fn goto_first(&mut self) {
        self.selected = 0;
    }

    pub fn goto_last(&mut self) {
        self.selected = self.rows.len().saturating_sub(1);
    }

    /// Recomputes which rows are shown, in the order they are shown.
    ///
    /// With nothing typed the listing is in its own order, `..` first.
    /// With something typed it is in score order — the best match at the
    /// top, where the cursor already is — and `..` is dropped, because
    /// somebody typing a name is looking for a name.
    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        if self.filter.is_empty() {
            if self.parent().is_some() {
                rows.push(Row::Parent);
            }
            rows.extend((0..self.entries.len()).map(Row::Entry));
        } else {
            let mut scored: Vec<(i32, usize)> = self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    crate::fuzzy::score(&self.filter, &entry.name).map(|score| (score, index))
                })
                .collect();
            // Best first, and by name among equals so the order does not
            // shuffle as unrelated entries come and go.
            scored.sort_by(|a, b| {
                b.0.cmp(&a.0).then_with(|| {
                    self.entries[a.1]
                        .name
                        .to_lowercase()
                        .cmp(&self.entries[b.1].name.to_lowercase())
                })
            });
            rows.extend(scored.into_iter().map(|(_, index)| Row::Entry(index)));
        }
        self.rows = rows;
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.to_string(),
            is_dir,
            link: None,
            size: 100,
            permissions: "rw-r--r--".into(),
            modified: "Aug 30 12:00".into(),
        }
    }

    fn browser() -> Browser {
        let mut browser = Browser::opening("/project/src");
        browser.listed(
            "/project/src",
            vec![
                entry("main.rs", false),
                entry("inner", true),
                entry("lib.rs", false),
                entry("assets", true),
            ],
        );
        browser
    }

    #[test]
    fn a_listing_puts_directories_first_and_then_sorts_by_name() {
        // The reader gives no order at all, and a directory that turns up
        // between two files is a directory nobody finds.
        let browser = browser();
        let names: Vec<&str> = browser.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["assets", "inner", "lib.rs", "main.rs"]);
    }

    #[test]
    fn the_parent_is_offered_first_when_nothing_is_typed() {
        let browser = browser();
        assert_eq!(browser.rows().first(), Some(&Row::Parent));
        assert_eq!(browser.rows().len(), 5, "four entries and `..`");
        assert_eq!(browser.current(), Some(Row::Parent));
        assert_eq!(browser.current_path(), Some(PathBuf::from("/project")));
        assert!(browser.current_is_dir());
    }

    #[test]
    fn the_filesystem_root_is_not_offered_a_parent() {
        let mut browser = Browser::opening("/");
        browser.listed("/", vec![entry("etc", true)]);
        assert_eq!(browser.rows(), [Row::Entry(0)]);
        assert_eq!(browser.parent(), None);
    }

    #[test]
    fn typing_narrows_the_listing_fuzzily() {
        let mut browser = browser();
        for c in "lb".chars() {
            browser.type_char(c);
        }
        let shown: Vec<&str> = browser
            .rows()
            .iter()
            .filter_map(|row| browser.entry(*row))
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(shown, ["lib.rs"], "`lb` should find `lib.rs`");
        assert_eq!(browser.tally(), (1, 4));
    }

    #[test]
    fn typing_drops_the_parent_row() {
        // Somebody typing a name is looking for a name, and `..` is not one.
        let mut browser = browser();
        browser.type_char('a');
        assert!(
            !browser.rows().contains(&Row::Parent),
            "`..` survived a search: {:?}",
            browser.rows()
        );
    }

    #[test]
    fn the_cursor_goes_back_to_the_top_when_the_filter_changes() {
        // Otherwise it is left pointing at whatever happens to be in that
        // position now, which is a different file.
        let mut browser = browser();
        browser.next(3);
        assert_eq!(browser.selected, 3);
        browser.type_char('s');
        assert_eq!(browser.selected, 0);
    }

    #[test]
    fn rubbing_out_says_whether_there_was_anything_to_rub_out() {
        // How the caller knows to go up a directory instead, which is what
        // backspace does in every file browser there is.
        let mut browser = browser();
        assert!(!browser.rub_out(), "nothing was typed");
        browser.type_char('x');
        assert!(browser.rub_out());
        assert_eq!(browser.filter, "");
        assert!(!browser.rub_out());
    }

    #[test]
    fn the_cursor_stops_at_the_ends_rather_than_wrapping() {
        // A list that wraps under a held-down arrow is a list you overshoot.
        let mut browser = browser();
        browser.next(100);
        assert_eq!(browser.selected, browser.rows().len() - 1);
        browser.next(1);
        assert_eq!(browser.selected, browser.rows().len() - 1);
        browser.previous(100);
        assert_eq!(browser.selected, 0);
        browser.previous(1);
        assert_eq!(browser.selected, 0);
    }

    #[test]
    fn what_the_cursor_points_at_is_a_path_in_this_directory() {
        let mut browser = browser();
        browser.next(1);
        assert_eq!(browser.current(), Some(Row::Entry(0)));
        assert_eq!(
            browser.current_path(),
            Some(PathBuf::from("/project/src/assets"))
        );
        assert!(browser.current_is_dir(), "`assets` is a directory");

        browser.next(2);
        assert_eq!(
            browser.current_path(),
            Some(PathBuf::from("/project/src/lib.rs"))
        );
        assert!(!browser.current_is_dir());
    }

    #[test]
    fn a_filter_that_matches_nothing_leaves_nothing_to_open() {
        let mut browser = browser();
        for c in "zzzz".chars() {
            browser.type_char(c);
        }
        assert!(browser.rows().is_empty());
        assert_eq!(browser.current(), None);
        assert_eq!(browser.current_path(), None);
        assert!(!browser.current_is_dir());
        assert_eq!(browser.tally(), (0, 4));
    }

    #[test]
    fn a_new_listing_keeps_what_was_typed_but_starts_at_the_top() {
        // A directory arriving should not undo what was typed while it was
        // being read; the cursor has to move, because it was pointing into
        // a different directory.
        let mut browser = browser();
        browser.type_char('l');
        browser.next(5);
        browser.listed("/other", vec![entry("late.rs", false), entry("x", false)]);
        assert_eq!(browser.filter, "l", "it forgot what was typed");
        assert_eq!(browser.selected, 0);
        assert_eq!(browser.directory, PathBuf::from("/other"));
    }

    #[test]
    fn an_empty_directory_has_only_its_parent_in_it() {
        let mut browser = Browser::opening("/project/empty");
        browser.listed("/project/empty", Vec::new());
        assert_eq!(browser.rows(), [Row::Parent]);
        assert_eq!(browser.tally(), (0, 0));
    }
}
