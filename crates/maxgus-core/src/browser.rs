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
//! The same box answers the other question a path prompt asks — *which
//! directory* — for the tree's `r a` and `C-x t d`. Typing a directory in
//! full is the slowest way to name one you could point at, so those prompts
//! open this instead. See [`Purpose`].
//!
//! Walking is the wrong way to reach somewhere that is not under where you
//! started, so that box can also be handed the whole of a directory tree at
//! once — every directory under `$HOME`, listed by its path relative to it —
//! and narrowed by typing across all of them. See [`Browser::found`].
//!
//! The model is here and the drawing is [`crate::render`]'s, so what it
//! shows and what it selects can be checked without a window.

use crate::dired::Entry;
use std::path::{Path, PathBuf};

/// One row of the listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// `.`, the directory being looked at. Only when the answer is a
    /// directory, where it is the one row that is otherwise unreachable:
    /// every other row names something *in* here.
    Here,
    /// `..`, which is offered whenever there is a directory above.
    Parent,
    /// An entry, by its index into [`Browser::entries`].
    Entry(usize),
}

/// What the box is being used to answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Purpose {
    /// A file to open. Everything in the directory is listed, and `RET` on
    /// a directory goes into it, there being nothing else it could mean.
    #[default]
    Open,
    /// A directory to hand back to the command that asked.
    ///
    /// Only directories are listed — a file is not an answer, and rows that
    /// cannot be chosen are rows to arrow past. That frees `RET` to *choose*
    /// rather than to descend, which the right arrow already does, so
    /// picking one is: arrow to it, `RET`.
    Directory,
}

/// A directory, what is in it, and what has been typed to narrow it.
#[derive(Debug, Clone, Default)]
pub struct Browser {
    pub directory: PathBuf,
    /// What is being asked, and so what `RET` does.
    pub purpose: Purpose,
    /// The question, for the box to show. Empty when it is just a file
    /// being opened, where the box itself is the question.
    pub prompt: String,
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
    /// True when the rows came from a walk rather than from one directory.
    ///
    /// The names are then paths relative to [`Browser::directory`], which is
    /// the root that was walked — so `current_path` joins them and needs to
    /// know nothing about it.
    pub searched: bool,
    /// True when the walk stopped at its limit rather than at the end, so
    /// the box can say the list is not all of them.
    pub capped: bool,
    rows: Vec<Row>,
}

impl Browser {
    /// A browser waiting for `directory` to be read, to open a file from.
    pub fn opening(directory: impl Into<PathBuf>) -> Browser {
        Browser {
            directory: directory.into(),
            pending: true,
            ..Browser::default()
        }
    }

    /// The same box, asking `prompt` and answering with a directory.
    pub fn choosing(directory: impl Into<PathBuf>, prompt: impl Into<String>) -> Browser {
        Browser {
            directory: directory.into(),
            purpose: Purpose::Directory,
            prompt: prompt.into(),
            pending: true,
            ..Browser::default()
        }
    }

    /// True when only directories are listed and `RET` chooses one.
    pub fn is_choosing(&self) -> bool {
        self.purpose == Purpose::Directory
    }

    /// What was read. The filter is kept — a listing arriving should not
    /// undo what was typed while it was being read — but the cursor goes
    /// back to the top, because it was pointing into a different directory.
    pub fn listed(&mut self, directory: impl Into<PathBuf>, entries: Vec<Entry>) {
        self.directory = directory.into();
        self.entries = entries;
        // Dropped here rather than when the rows are built, so the tally,
        // the filter and everything downstream count what can be chosen
        // rather than what happens to be in the directory. `3/40` with 37
        // of them files nobody can pick is a count of the wrong thing.
        if self.is_choosing() {
            self.entries.retain(|entry| entry.is_dir);
        }
        // Directories first and then by name, which is the order anyone
        // walking a tree by eye expects. The reader gives no order at all.
        self.entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.pending = false;
        self.searched = false;
        self.capped = false;
        self.selected = 0;
        self.rebuild();
    }

    /// Puts the box into the state a walk is about to fill, so it can say
    /// what it is doing while the walk runs.
    pub fn searching(&mut self, root: impl Into<PathBuf>) {
        self.directory = root.into();
        self.entries.clear();
        self.filter.clear();
        self.selected = 0;
        self.pending = true;
        self.searched = true;
        self.capped = false;
        self.rebuild();
    }

    /// What a walk turned up: every directory under `root`, by its path
    /// relative to it.
    ///
    /// Relative because that is the part worth typing at. Under a home
    /// directory every answer starts with the same dozen characters, and a
    /// fuzzy match against them matches everything.
    pub fn found(&mut self, root: impl Into<PathBuf>, paths: Vec<String>, capped: bool) {
        self.directory = root.into();
        self.entries = paths
            .into_iter()
            .map(|name| Entry {
                name,
                is_dir: true,
                link: None,
                size: 0,
                permissions: String::new(),
                modified: String::new(),
            })
            .collect();
        self.pending = false;
        self.searched = true;
        self.capped = capped;
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
            Row::Here | Row::Parent => None,
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
            Row::Here => Some(self.directory.clone()),
            Row::Parent => self.parent(),
            Row::Entry(index) => Some(self.directory.join(&self.entries.get(index)?.name)),
        }
    }

    /// True when the cursor is on something to go into rather than open.
    pub fn current_is_dir(&self) -> bool {
        match self.current() {
            Some(Row::Here | Row::Parent) => true,
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

    /// What has been typed, when it is a path rather than a search.
    ///
    /// A filename cannot contain `/`, so a filter with one in it is nobody
    /// searching — it is somebody typing or pasting a path, and narrowing a
    /// listing to nothing is not what they meant by it. `~` says the same.
    ///
    /// Only where a directory is being chosen. For a file there is already
    /// a command that takes a path and completes it, `C-x C-f`, and this
    /// box exists for the other case; a directory has no such command, so
    /// the box has to carry both ways of naming one.
    pub fn typed_path(&self) -> Option<&str> {
        // Not while the rows are themselves paths: `src/main` is then the
        // most ordinary thing to type, and taking it literally would answer
        // with a directory relative to nowhere in particular.
        if !self.is_choosing() || self.searched {
            return None;
        }
        let typed = self.filter.trim();
        match typed.contains('/') || typed.starts_with('~') {
            true => Some(typed),
            false => None,
        }
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
    /// With nothing typed the listing is in its own order, `.` and `..`
    /// first. With something typed it is in score order — the best match at
    /// the top, where the cursor already is — and neither is offered,
    /// because somebody typing a name is looking for a name.
    ///
    /// When a directory is what is wanted, `.` leads: the box opens on the
    /// directory the command started from, so `RET` straight away answers
    /// with it, which is the answer often enough to be worth being the
    /// default. Walking somewhere else first is what the arrows are for.
    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        if self.filter.is_empty() {
            // A walk is a list of answers and nothing else. `.` would be the
            // root that was walked, which is not what anybody widened the
            // search to find, and `..` would climb out of it.
            if self.is_choosing() && !self.searched {
                rows.push(Row::Here);
            }
            if self.parent().is_some() && !self.searched {
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

    // ---- choosing a directory rather than opening a file ----------

    fn choosing() -> Browser {
        let mut browser = Browser::choosing("/project/src", "Add to tree");
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
    fn only_directories_are_listed_when_a_directory_is_what_is_wanted() {
        // A file is not an answer, and rows that cannot be chosen are rows
        // to arrow past. Dropped from the entries rather than from the
        // rows, so the tally counts what can be chosen: `1/4`, with three
        // of them files nobody can pick, is a count of the wrong thing.
        let browser = choosing();
        let names: Vec<&str> = browser.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["assets", "inner"]);
        assert_eq!(browser.tally(), (2, 2));
    }

    #[test]
    fn the_directory_being_looked_at_leads_and_answers_with_itself() {
        // The box opens where the command started, and that is the answer
        // often enough to be the default. It is also the one row nothing
        // else can reach: every other row names something *inside* here.
        let browser = choosing();
        assert_eq!(browser.rows()[0], Row::Here);
        assert_eq!(browser.rows()[1], Row::Parent);
        assert_eq!(browser.current(), Some(Row::Here));
        assert_eq!(browser.current_path(), Some(PathBuf::from("/project/src")));
        assert!(browser.current_is_dir());
    }

    #[test]
    fn opening_a_file_offers_no_here_row() {
        // There is nothing it could mean: `RET` on it would open a
        // directory, and going into one is what the right arrow is for.
        let browser = browser();
        assert!(!browser.rows().contains(&Row::Here));
        assert!(!browser.is_choosing());
    }

    #[test]
    fn typing_drops_both_the_here_and_the_parent_rows() {
        let mut browser = choosing();
        browser.type_char('n');
        assert_eq!(
            browser
                .rows()
                .iter()
                .filter_map(|row| browser.entry(*row))
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["inner"]
        );
        assert!(!browser.rows().contains(&Row::Here));
        assert!(!browser.rows().contains(&Row::Parent));
    }

    #[test]
    fn a_filter_with_a_slash_in_it_is_a_path_rather_than_a_search() {
        // A filename cannot contain `/`, so somebody who typed one is
        // pasting a path, and narrowing a listing to nothing is not what
        // they meant by it.
        let mut browser = choosing();
        for c in "/other/place".chars() {
            browser.type_char(c);
        }
        assert!(browser.rows().is_empty(), "it matched something");
        assert_eq!(browser.typed_path(), Some("/other/place"));

        browser.clear_filter();
        for c in "~/src".chars() {
            browser.type_char(c);
        }
        assert_eq!(browser.typed_path(), Some("~/src"), "`~` names a path too");
    }

    #[test]
    fn a_plain_name_is_a_search_rather_than_a_path() {
        let mut browser = choosing();
        browser.type_char('i');
        assert_eq!(browser.typed_path(), None);
    }

    #[test]
    fn a_path_typed_at_a_box_opening_a_file_is_just_a_search() {
        // `C-x C-f` is the command that takes a path and completes it, and
        // this box exists for the other case. A directory has no such
        // command, which is why the other box carries both.
        let mut browser = browser();
        for c in "/etc".chars() {
            browser.type_char(c);
        }
        assert_eq!(browser.typed_path(), None);
    }

    #[test]
    fn an_empty_directory_has_only_its_parent_in_it() {
        let mut browser = Browser::opening("/project/empty");
        browser.listed("/project/empty", Vec::new());
        assert_eq!(browser.rows(), [Row::Parent]);
        assert_eq!(browser.tally(), (0, 0));
    }
}
