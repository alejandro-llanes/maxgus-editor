//! A directory as a buffer you can edit the contents of.
//!
//! The tree is for browsing a project; this is for working on a directory —
//! marking a dozen files and deleting, copying, renaming or running something
//! over all of them at once. That is what dired is for in Emacs, and the
//! marks are why: an operation you can aim at a set is worth more than one
//! you aim at a file.

use std::path::{Path, PathBuf};

/// One directory entry, as the listing shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    /// Where a symbolic link points, when it is one.
    pub link: Option<String>,
    pub size: u64,
    /// `rwxr-xr-x`, or empty where the platform has nothing to say.
    pub permissions: String,
    /// As a person reads it: `Aug 29 15:03`.
    pub modified: String,
}

/// What a line is marked with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    None,
    /// `*`: chosen, for whatever is done next.
    Marked,
    /// `D`: to be deleted when `x` is pressed.
    Deleted,
}

impl Mark {
    pub fn glyph(self) -> char {
        match self {
            Mark::None => ' ',
            Mark::Marked => '*',
            Mark::Deleted => 'D',
        }
    }
}

/// One line of the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// The directory's own name and what is in it.
    Title,
    Blank,
    /// `..`, which every directory but the root has.
    Parent,
    /// An entry, by its index.
    Entry(usize),
}

/// A directory, its entries, and what is marked.
#[derive(Debug, Clone, Default)]
pub struct DiredView {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    marks: Vec<Mark>,
    rows: Vec<Row>,
}

impl DiredView {
    pub fn new(path: PathBuf, mut entries: Vec<Entry>) -> DiredView {
        // Directories first, then by name: the order `ls --group-directories`
        // gives, and the one that makes a deep tree readable.
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        let marks = vec![Mark::None; entries.len()];
        let mut view = DiredView {
            path,
            entries,
            marks,
            rows: Vec::new(),
        };
        view.lay_out();
        view
    }

    /// Keeps the marks that still name something, over a refresh.
    pub fn refreshed(&self, entries: Vec<Entry>) -> DiredView {
        let marked: Vec<&str> = self
            .marks
            .iter()
            .enumerate()
            .filter(|(_, mark)| **mark != Mark::None)
            .filter_map(|(index, _)| self.entries.get(index).map(|e| e.name.as_str()))
            .collect();
        let mut fresh = DiredView::new(self.path.clone(), entries);
        for (index, entry) in fresh.entries.iter().enumerate() {
            if marked.contains(&entry.name.as_str()) {
                fresh.marks[index] = Mark::Marked;
            }
        }
        fresh
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row(&self, line: usize) -> Option<&Row> {
        self.rows.get(line)
    }

    /// The entry a line is about.
    pub fn entry(&self, line: usize) -> Option<&Entry> {
        match self.rows.get(line)? {
            Row::Entry(index) => self.entries.get(*index),
            _ => None,
        }
    }

    /// The full path of the entry a line is about, or the parent directory.
    pub fn target(&self, line: usize) -> Option<PathBuf> {
        match self.rows.get(line)? {
            Row::Parent => self.path.parent().map(Path::to_path_buf),
            Row::Entry(index) => Some(self.path.join(&self.entries.get(*index)?.name)),
            _ => None,
        }
    }

    pub fn mark_at(&self, line: usize) -> Mark {
        match self.rows.get(line) {
            Some(Row::Entry(index)) => self.marks.get(*index).copied().unwrap_or(Mark::None),
            _ => Mark::None,
        }
    }

    pub fn set_mark(&mut self, line: usize, mark: Mark) -> bool {
        match self.rows.get(line) {
            Some(Row::Entry(index)) => match self.marks.get_mut(*index) {
                Some(slot) => {
                    *slot = mark;
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    pub fn mark_all(&mut self, mark: Mark) {
        self.marks.iter_mut().for_each(|slot| *slot = mark);
    }

    /// Turns marked into unmarked and unmarked into marked, leaving flagged
    /// lines alone: `t` in dired means the choice, not the flags.
    pub fn toggle_marks(&mut self) {
        for slot in &mut self.marks {
            *slot = match *slot {
                Mark::None => Mark::Marked,
                Mark::Marked => Mark::None,
                Mark::Deleted => Mark::Deleted,
            };
        }
    }

    /// The paths carrying `mark`.
    pub fn with_mark(&self, mark: Mark) -> Vec<PathBuf> {
        self.marks
            .iter()
            .enumerate()
            .filter(|(_, slot)| **slot == mark)
            .filter_map(|(index, _)| self.entries.get(index))
            .map(|entry| self.path.join(&entry.name))
            .collect()
    }

    /// What an operation should act on: everything marked, or the line point
    /// is on when nothing is. Dired's rule, and the reason marking is
    /// optional rather than a mode.
    pub fn acting_on(&self, line: usize) -> Vec<PathBuf> {
        let marked = self.with_mark(Mark::Marked);
        match marked.is_empty() {
            true => self.target(line).into_iter().collect(),
            false => marked,
        }
    }

    /// Where point starts: the first real entry, not `..`.
    ///
    /// Dired opens on something you can act on. `..` is a way out rather than
    /// a thing in the directory, and starting on it makes the first `m` or
    /// `d` do nothing.
    pub fn first_entry_line(&self) -> usize {
        self.rows
            .iter()
            .position(|row| matches!(row, Row::Entry(_)))
            .or_else(|| self.rows.iter().position(|row| matches!(row, Row::Parent)))
            .unwrap_or(0)
    }

    /// The next or previous line that names something.
    pub fn step(&self, from: usize, forward: bool) -> Option<usize> {
        let lines: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, Row::Entry(_) | Row::Parent))
            .map(|(line, _)| line)
            .collect();
        match forward {
            true => lines.into_iter().find(|line| *line > from),
            false => lines.into_iter().rfind(|line| *line < from),
        }
    }

    /// The line an entry is drawn on, for keeping point over a refresh.
    pub fn line_of_name(&self, name: &str) -> Option<usize> {
        self.rows.iter().position(|row| match row {
            Row::Entry(index) => self.entries.get(*index).is_some_and(|e| e.name == name),
            _ => false,
        })
    }

    fn lay_out(&mut self) {
        let mut rows = vec![Row::Title, Row::Blank];
        if self.path.parent().is_some() {
            rows.push(Row::Parent);
        }
        for index in 0..self.entries.len() {
            rows.push(Row::Entry(index));
        }
        self.rows = rows;
    }

    /// The text of one row.
    pub fn row_text(&self, row: &Row) -> String {
        match row {
            Row::Title => {
                let (dirs, files): (Vec<_>, Vec<_>) =
                    self.entries.iter().partition(|entry| entry.is_dir);
                let bytes: u64 = files.iter().map(|entry| entry.size).sum();
                format!(
                    "{}  —  {} file(s), {} director(ies), {}",
                    self.path.display(),
                    files.len(),
                    dirs.len(),
                    human_size(bytes)
                )
            }
            Row::Blank => String::new(),
            Row::Parent => "  ..".to_string(),
            Row::Entry(index) => {
                let Some(entry) = self.entries.get(*index) else {
                    return String::new();
                };
                let mark = self.marks.get(*index).copied().unwrap_or(Mark::None);
                let size = match entry.is_dir {
                    true => "     -".to_string(),
                    false => format!("{:>6}", human_size(entry.size)),
                };
                let name = match (&entry.link, entry.is_dir) {
                    (Some(target), _) => format!("{} -> {target}", entry.name),
                    (None, true) => format!("{}/", entry.name),
                    (None, false) => entry.name.clone(),
                };
                format!(
                    "{} {:<10} {size} {:<12} {name}",
                    mark.glyph(),
                    entry.permissions,
                    entry.modified
                )
            }
        }
    }

    pub fn text(&self) -> String {
        self.rows
            .iter()
            .map(|row| format!("{}\n", self.row_text(row)))
            .collect()
    }
}

/// A size as a person says it.
fn human_size(bytes: u64) -> String {
    const K: u64 = 1024;
    match bytes {
        _ if bytes < K => format!("{bytes}"),
        _ if bytes < K * K => format!("{:.1}k", bytes as f64 / K as f64),
        _ if bytes < K * K * K => format!("{:.1}M", bytes as f64 / (K * K) as f64),
        _ => format!("{:.1}G", bytes as f64 / (K * K * K) as f64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool, size: u64) -> Entry {
        Entry {
            name: name.into(),
            is_dir,
            link: None,
            size,
            permissions: if is_dir { "drwxr-xr-x" } else { "-rw-r--r--" }.into(),
            modified: "Aug 29 15:03".into(),
        }
    }

    fn view() -> DiredView {
        DiredView::new(
            PathBuf::from("/project/src"),
            vec![
                entry("zeta.rs", false, 100),
                entry("alpha.rs", false, 2048),
                entry("nested", true, 0),
            ],
        )
    }

    #[test]
    fn directories_come_first_and_then_names_in_order() {
        let view = view();
        let names: Vec<&str> = view.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["nested", "alpha.rs", "zeta.rs"]);
    }

    #[test]
    fn the_listing_reads_as_a_directory() {
        let text = view().text();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].contains("/project/src"), "no path: {:?}", lines[0]);
        assert!(lines[0].contains("2 file(s)"), "no count: {:?}", lines[0]);
        assert_eq!(lines[2], "  ..", "no way up");
        assert!(lines[3].contains("nested/"), "got {:?}", lines[3]);
        assert!(lines[4].contains("2.0k"), "no size: {:?}", lines[4]);
    }

    #[test]
    fn a_line_names_the_file_it_is_about() {
        let view = view();
        assert_eq!(view.target(4), Some(PathBuf::from("/project/src/alpha.rs")));
        assert_eq!(view.target(2), Some(PathBuf::from("/project")), "`..`");
        assert_eq!(view.target(0), None, "the title is not a file");
    }

    #[test]
    fn marks_go_on_entries_and_nowhere_else() {
        let mut view = view();
        assert!(view.set_mark(4, Mark::Marked));
        assert!(!view.set_mark(0, Mark::Marked), "the title took a mark");
        assert!(!view.set_mark(2, Mark::Marked), "`..` took a mark");
        assert_eq!(view.mark_at(4), Mark::Marked);
        assert!(
            view.text().contains("* -rw-r--r--"),
            "the mark is not drawn"
        );
    }

    #[test]
    fn an_operation_acts_on_the_marks_or_on_the_line() {
        let mut view = view();
        // Nothing marked: the line point is on.
        assert_eq!(
            view.acting_on(4),
            vec![PathBuf::from("/project/src/alpha.rs")]
        );
        view.set_mark(3, Mark::Marked);
        view.set_mark(5, Mark::Marked);
        let acting = view.acting_on(4);
        assert_eq!(acting.len(), 2, "the marks were ignored");
        assert!(acting.contains(&PathBuf::from("/project/src/nested")));
        assert!(acting.contains(&PathBuf::from("/project/src/zeta.rs")));
    }

    #[test]
    fn toggling_swaps_the_choice_and_leaves_the_flags() {
        let mut view = view();
        view.set_mark(3, Mark::Marked);
        view.set_mark(4, Mark::Deleted);
        view.toggle_marks();
        assert_eq!(view.mark_at(3), Mark::None, "a mark was not cleared");
        assert_eq!(view.mark_at(4), Mark::Deleted, "a flag was cleared");
        assert_eq!(
            view.mark_at(5),
            Mark::Marked,
            "an unmarked line was not marked"
        );
    }

    #[test]
    fn stepping_skips_the_title_and_the_blank() {
        let view = view();
        assert_eq!(
            view.first_entry_line(),
            3,
            "it should start on a real entry"
        );
        assert_eq!(view.step(2, true), Some(3));
        assert_eq!(view.step(5, true), None);
        assert_eq!(view.step(3, false), Some(2));
    }

    #[test]
    fn a_refresh_keeps_the_marks_that_still_name_something() {
        let mut view = view();
        view.set_mark(4, Mark::Marked); // alpha.rs
        let fresh = view.refreshed(vec![
            entry("alpha.rs", false, 3000),
            entry("new.rs", false, 10),
        ]);
        let marked = fresh.with_mark(Mark::Marked);
        assert_eq!(marked, vec![PathBuf::from("/project/src/alpha.rs")]);
        assert_eq!(fresh.entries.len(), 2, "the listing did not refresh");
    }

    #[test]
    fn a_refresh_drops_a_mark_on_something_that_is_gone() {
        let mut view = view();
        view.set_mark(5, Mark::Marked); // zeta.rs
        let fresh = view.refreshed(vec![entry("alpha.rs", false, 1)]);
        assert!(fresh.with_mark(Mark::Marked).is_empty());
    }

    #[test]
    fn a_symbolic_link_says_where_it_goes() {
        let view = DiredView::new(
            PathBuf::from("/p"),
            vec![Entry {
                link: Some("../elsewhere".into()),
                ..entry("here", false, 0)
            }],
        );
        assert!(view.text().contains("here -> ../elsewhere"));
    }

    #[test]
    fn a_directory_at_the_root_has_no_way_up() {
        let view = DiredView::new(PathBuf::from("/"), vec![entry("etc", true, 0)]);
        assert!(!view.rows().contains(&Row::Parent));
        assert_eq!(view.first_entry_line(), 2);
    }

    #[test]
    fn an_empty_directory_starts_on_the_way_out_of_it() {
        let view = DiredView::new(PathBuf::from("/p/empty"), Vec::new());
        assert!(matches!(
            view.row(view.first_entry_line()),
            Some(Row::Parent)
        ));
    }

    #[test]
    fn a_name_can_be_found_again_after_a_refresh() {
        let view = view();
        assert_eq!(view.line_of_name("alpha.rs"), Some(4));
        assert_eq!(view.line_of_name("gone.rs"), None);
    }
}
