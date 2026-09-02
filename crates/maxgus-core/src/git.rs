//! The git status view: what it shows, and what folds away.
//!
//! Modelled on magit, and for magit's reason: a commit is assembled by looking
//! at the change rather than by remembering it. So the view is one buffer with
//! everything in it — what is untracked, what is changed, what is staged, what
//! is stashed, what has not been pushed — and every level of it folds, down to
//! the individual hunk.
//!
//! As with the side panel, the view is a list of rows over a read-only buffer,
//! so point moves with the ordinary motion commands and every command asks
//! what row point is on. Nothing here runs git or knows how: it is given
//! parsed output and lays out rows.

use maxgus_git::{Commit, FileDiff, Stash, Status};
use std::collections::BTreeSet;

/// The parts of the status view, in the order they appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    /// Conflicts first: nothing else can be done until they are resolved.
    Unmerged,
    Untracked,
    Unstaged,
    Staged,
    Stashes,
    /// Commits here that the upstream has not.
    Unpushed,
    /// Commits the upstream has that here does not.
    Unpulled,
    /// The last few commits, so there is always something to see.
    Recent,
}

pub const SECTIONS: [Section; 8] = [
    Section::Unmerged,
    Section::Untracked,
    Section::Unstaged,
    Section::Staged,
    Section::Stashes,
    Section::Unpushed,
    Section::Unpulled,
    Section::Recent,
];

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Unmerged => "Unmerged",
            Section::Untracked => "Untracked files",
            Section::Unstaged => "Unstaged changes",
            Section::Staged => "Staged changes",
            Section::Stashes => "Stashes",
            Section::Unpushed => "Unpushed to upstream",
            Section::Unpulled => "Unpulled from upstream",
            Section::Recent => "Recent commits",
        }
    }

    /// True when the section holds files that can be staged or unstaged.
    pub fn is_files(self) -> bool {
        matches!(
            self,
            Section::Unmerged | Section::Untracked | Section::Unstaged | Section::Staged
        )
    }
}

/// One line of the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// The `Head:`/`Merge:`/`Push:` lines at the top.
    Header(HeaderLine),
    Blank,
    Section(Section),
    /// A file, by its index within the section.
    File {
        section: Section,
        file: usize,
    },
    /// A hunk of a file.
    Hunk {
        section: Section,
        file: usize,
        hunk: usize,
    },
    /// One line of a hunk.
    Line {
        section: Section,
        file: usize,
        hunk: usize,
        line: usize,
    },
    Stash(usize),
    Commit {
        section: Section,
        commit: usize,
    },
    /// A section that is on but has nothing in it. Only shown for sections
    /// worth reassuring about.
    Empty(Section),
}

impl Row {
    /// How deeply nested the row is, or `None` when it is not a section at
    /// all — a diff line, a blank, or one of the header lines.
    ///
    /// This is what `n` and `p` move by. Stepping through the lines of a hunk
    /// one at a time is what `C-n` is for; `n` is for getting about.
    pub fn level(&self) -> Option<usize> {
        match self {
            Row::Section(_) => Some(0),
            Row::File { .. } | Row::Stash(_) | Row::Commit { .. } => Some(1),
            Row::Hunk { .. } => Some(2),
            Row::Header(_) | Row::Blank | Row::Line { .. } | Row::Empty(_) => None,
        }
    }

    /// The section the row belongs to, when it belongs to one.
    pub fn section(&self) -> Option<Section> {
        match self {
            Row::Section(section)
            | Row::Empty(section)
            | Row::File { section, .. }
            | Row::Hunk { section, .. }
            | Row::Line { section, .. }
            | Row::Commit { section, .. } => Some(*section),
            Row::Stash(_) => Some(Section::Stashes),
            Row::Header(_) | Row::Blank => None,
        }
    }
}

/// One of the lines describing where `HEAD` is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderLine {
    pub label: String,
    pub reference: String,
    pub subject: String,
}

/// Everything the view shows, and what is folded.
#[derive(Debug, Clone, Default)]
pub struct GitView {
    pub status: Status,
    /// Diffs for the working tree and for the index, in section order.
    pub unstaged: Vec<FileDiff>,
    pub staged: Vec<FileDiff>,
    pub stashes: Vec<Stash>,
    pub unpushed: Vec<Commit>,
    pub unpulled: Vec<Commit>,
    pub recent: Vec<Commit>,
    /// The subject of the commit `HEAD` is at.
    pub head_subject: String,
    /// True once a refresh has answered, so an empty view can say whether it
    /// is empty or merely not asked yet.
    pub loaded: bool,

    collapsed: BTreeSet<Section>,
    /// Files whose hunks are shown, by section and path. Collapsed is the
    /// default: a status with twenty changed files should fit on a screen.
    expanded_files: BTreeSet<(Section, String)>,
    collapsed_hunks: BTreeSet<(Section, String, usize)>,
    rows: Vec<Row>,
}

impl GitView {
    pub fn new() -> GitView {
        GitView::default()
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row(&self, line: usize) -> Option<&Row> {
        self.rows.get(line)
    }

    pub fn line_of(&self, row: &Row) -> Option<usize> {
        self.rows.iter().position(|candidate| candidate == row)
    }

    /// The line of `row`, or of the nearest thing to it that is still
    /// shown: the item before it in its section, then the section itself.
    ///
    /// Staging the last unstaged file takes its row away, and point has to
    /// land somewhere. Magit's answer is what was around it, not the top
    /// of the buffer.
    pub fn line_near(&self, row: &Row) -> Option<usize> {
        let mut candidate = row.clone();
        loop {
            if let Some(line) = self.line_of(&candidate) {
                return Some(line);
            }
            candidate = match candidate {
                Row::Line {
                    section,
                    file,
                    hunk,
                    line,
                } if line > 0 => Row::Line {
                    section,
                    file,
                    hunk,
                    line: line - 1,
                },
                Row::Line {
                    section,
                    file,
                    hunk,
                    ..
                } => Row::Hunk {
                    section,
                    file,
                    hunk,
                },
                Row::Hunk {
                    section,
                    file,
                    hunk,
                } if hunk > 0 => Row::Hunk {
                    section,
                    file,
                    hunk: hunk - 1,
                },
                Row::Hunk { section, file, .. } => Row::File { section, file },
                Row::File { section, file } if file > 0 => Row::File {
                    section,
                    file: file - 1,
                },
                Row::File { section, .. } => Row::Section(section),
                Row::Commit { section, commit } if commit > 0 => Row::Commit {
                    section,
                    commit: commit - 1,
                },
                Row::Commit { section, .. } => Row::Section(section),
                Row::Stash(n) if n > 0 => Row::Stash(n - 1),
                Row::Stash(_) => Row::Section(Section::Stashes),
                _ => return None,
            };
        }
    }

    /// The files a section shows.
    pub fn files(&self, section: Section) -> &[FileDiff] {
        match section {
            Section::Unstaged => &self.unstaged,
            Section::Staged => &self.staged,
            _ => &[],
        }
    }

    /// The paths a section shows, which for untracked and unmerged files come
    /// from the status rather than from a diff.
    pub fn paths(&self, section: Section) -> Vec<String> {
        match section {
            Section::Untracked => self
                .status
                .untracked()
                .map(|e| e.path.to_string_lossy().into_owned())
                .collect(),
            Section::Unmerged => self
                .status
                .unmerged()
                .map(|e| e.path.to_string_lossy().into_owned())
                .collect(),
            other => self
                .files(other)
                .iter()
                .map(|file| file.path.clone())
                .collect(),
        }
    }

    pub fn commits(&self, section: Section) -> &[Commit] {
        match section {
            Section::Unpushed => &self.unpushed,
            Section::Unpulled => &self.unpulled,
            Section::Recent => &self.recent,
            _ => &[],
        }
    }

    /// How many things a section holds, for its heading.
    pub fn count(&self, section: Section) -> usize {
        match section {
            Section::Stashes => self.stashes.len(),
            Section::Unpushed | Section::Unpulled | Section::Recent => self.commits(section).len(),
            other => self.paths(other).len(),
        }
    }

    pub fn is_collapsed(&self, section: Section) -> bool {
        self.collapsed.contains(&section)
    }

    pub fn toggle_section(&mut self, section: Section) {
        if !self.collapsed.insert(section) {
            self.collapsed.remove(&section);
        }
    }

    pub fn is_file_expanded(&self, section: Section, path: &str) -> bool {
        self.expanded_files.contains(&(section, path.to_string()))
    }

    pub fn toggle_file(&mut self, section: Section, path: &str) {
        let key = (section, path.to_string());
        if !self.expanded_files.insert(key.clone()) {
            self.expanded_files.remove(&key);
        }
    }

    pub fn is_hunk_collapsed(&self, section: Section, path: &str, hunk: usize) -> bool {
        self.collapsed_hunks
            .contains(&(section, path.to_string(), hunk))
    }

    pub fn toggle_hunk(&mut self, section: Section, path: &str, hunk: usize) {
        let key = (section, path.to_string(), hunk);
        if !self.collapsed_hunks.insert(key.clone()) {
            self.collapsed_hunks.remove(&key);
        }
    }

    /// Folds everything, which is how to see the shape of a large change.
    pub fn collapse_all(&mut self) {
        self.expanded_files.clear();
        self.collapsed.extend(SECTIONS);
    }

    /// Unfolds every section, but not every file: expanding a hundred files
    /// produces a buffer nobody can navigate.
    pub fn expand_all(&mut self) {
        self.collapsed.clear();
    }

    /// Lays the rows out and returns them.
    pub fn lay_out(&mut self) -> Vec<Row> {
        let mut rows = Vec::new();

        // Where `HEAD` is, then where it is going.
        let head = self.status.branch.clone().unwrap_or_else(|| {
            self.status
                .head
                .as_deref()
                .map(short)
                .unwrap_or("(no commits yet)")
                .to_string()
        });
        rows.push(Row::Header(HeaderLine {
            label: "Head:".into(),
            reference: head,
            subject: self.head_subject.clone(),
        }));
        if let Some(upstream) = self.status.upstream.clone() {
            let ahead_behind = match (self.status.ahead, self.status.behind) {
                (0, 0) => String::new(),
                (ahead, 0) => format!("{ahead} ahead"),
                (0, behind) => format!("{behind} behind"),
                (ahead, behind) => format!("{ahead} ahead, {behind} behind"),
            };
            rows.push(Row::Header(HeaderLine {
                label: "Merge:".into(),
                reference: upstream,
                subject: ahead_behind,
            }));
        }
        rows.push(Row::Blank);

        for section in SECTIONS {
            let count = self.count(section);
            // A section with nothing in it is left out entirely. Magit does
            // the same: a status view is a list of what needs attention, and
            // eight empty headings need none.
            if count == 0 {
                continue;
            }
            rows.push(Row::Section(section));
            if self.is_collapsed(section) {
                rows.push(Row::Blank);
                continue;
            }
            match section {
                Section::Stashes => {
                    rows.extend((0..self.stashes.len()).map(Row::Stash));
                }
                Section::Unpushed | Section::Unpulled | Section::Recent => {
                    rows.extend(
                        (0..self.commits(section).len())
                            .map(|commit| Row::Commit { section, commit }),
                    );
                }
                _ => self.lay_out_files(section, &mut rows),
            }
            rows.push(Row::Blank);
        }
        if rows.len() <= 3 && self.loaded {
            rows.push(Row::Empty(Section::Staged));
        }
        self.rows = rows.clone();
        rows
    }

    fn lay_out_files(&self, section: Section, rows: &mut Vec<Row>) {
        let paths = self.paths(section);
        for (index, path) in paths.iter().enumerate() {
            rows.push(Row::File {
                section,
                file: index,
            });
            // Untracked files have no diff to show, and an unmerged one is
            // not a diff either until it is resolved.
            if !section.is_files() || !self.is_file_expanded(section, path) {
                continue;
            }
            let Some(file) = self.files(section).get(index) else {
                continue;
            };
            for (hunk_index, hunk) in file.hunks.iter().enumerate() {
                rows.push(Row::Hunk {
                    section,
                    file: index,
                    hunk: hunk_index,
                });
                if self.is_hunk_collapsed(section, path, hunk_index) {
                    continue;
                }
                rows.extend((0..hunk.lines.len()).map(|line| Row::Line {
                    section,
                    file: index,
                    hunk: hunk_index,
                    line,
                }));
            }
        }
    }
}

/// Seven characters of a hash, which is what git shows by default.
fn short(hash: &str) -> &str {
    &hash[..hash.len().min(7)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
diff --git a/a.rs b/a.rs
index 111..222 100644
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,2 @@
-old
+new
 tail
@@ -10,2 +10,2 @@
-second
+SECOND
 tail
";

    fn view() -> GitView {
        let mut view = GitView::new();
        view.loaded = true;
        view.status = maxgus_git::status::parse(
            b"# branch.oid 5958f5e13418d8b5\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0? new.rs\0",
        );
        view.head_subject = "the last commit".into();
        view.unstaged = maxgus_git::diff::parse(DIFF);
        view.recent = maxgus_git::log::parse_log(
            "h\u{1f}abc1234\u{1f}Someone\u{1f}an hour ago\u{1f}\u{1f}a commit\u{1e}\n",
        );
        view
    }

    #[test]
    fn the_head_lines_say_where_the_branch_is_and_where_it_is_going() {
        let mut view = view();
        let rows = view.lay_out();
        let Row::Header(head) = &rows[0] else {
            panic!("no head line: {:?}", rows[0])
        };
        assert_eq!(head.reference, "main");
        assert_eq!(head.subject, "the last commit");

        let Row::Header(merge) = &rows[1] else {
            panic!("no merge line: {:?}", rows[1])
        };
        assert_eq!(merge.reference, "origin/main");
        assert_eq!(merge.subject, "2 ahead, 1 behind");
    }

    #[test]
    fn a_detached_head_is_named_by_its_commit() {
        let mut view = GitView::new();
        view.status =
            maxgus_git::status::parse(b"# branch.oid 5958f5e13418d8b5\0# branch.head (detached)\0");
        let rows = view.lay_out();
        let Row::Header(head) = &rows[0] else {
            panic!()
        };
        assert_eq!(head.reference, "5958f5e", "seven characters, as git shows");
    }

    #[test]
    fn an_empty_section_is_left_out_entirely() {
        // A status view is a list of what needs attention. Eight headings
        // over nothing need none.
        let mut view = view();
        let rows = view.lay_out();
        assert!(rows.contains(&Row::Section(Section::Untracked)));
        assert!(
            !rows.contains(&Row::Section(Section::Staged)),
            "nothing is staged, so there should be no staged heading"
        );
    }

    #[test]
    fn a_file_shows_its_hunks_only_once_expanded() {
        // Twenty changed files should fit on a screen; expanded by default
        // they would not.
        let mut view = view();
        let rows = view.lay_out();
        assert!(rows.contains(&Row::File {
            section: Section::Unstaged,
            file: 0
        }));
        assert!(
            !rows.iter().any(|row| matches!(row, Row::Hunk { .. })),
            "hunks are shown before the file was opened"
        );

        view.toggle_file(Section::Unstaged, "a.rs");
        let rows = view.lay_out();
        assert_eq!(
            rows.iter()
                .filter(|row| matches!(row, Row::Hunk { .. }))
                .count(),
            2
        );
        // And the lines of each hunk with them.
        assert!(rows.iter().any(|row| matches!(row, Row::Line { .. })));
    }

    #[test]
    fn a_hunk_folds_on_its_own() {
        let mut view = view();
        view.toggle_file(Section::Unstaged, "a.rs");
        view.lay_out();
        let lines = |view: &GitView| {
            view.rows()
                .iter()
                .filter(|row| matches!(row, Row::Line { .. }))
                .count()
        };
        let all = lines(&view);

        view.toggle_hunk(Section::Unstaged, "a.rs", 0);
        view.lay_out();
        assert!(lines(&view) < all, "folding a hunk hid nothing");
        assert_eq!(
            view.rows()
                .iter()
                .filter(|row| matches!(row, Row::Hunk { .. }))
                .count(),
            2,
            "the hunk headings should stay"
        );
    }

    #[test]
    fn a_collapsed_section_keeps_its_heading_and_loses_its_contents() {
        let mut view = view();
        view.toggle_section(Section::Untracked);
        let rows = view.lay_out();
        assert!(rows.contains(&Row::Section(Section::Untracked)));
        assert!(
            !rows.contains(&Row::File {
                section: Section::Untracked,
                file: 0
            }),
            "the section folded but its files are still listed"
        );
    }

    #[test]
    fn counts_are_taken_from_the_right_place_for_each_section() {
        // Untracked files come from the status, changes from a diff, and
        // getting that the wrong way round shows zero of everything.
        let view = view();
        assert_eq!(view.count(Section::Untracked), 1);
        assert_eq!(view.count(Section::Unstaged), 1);
        assert_eq!(view.count(Section::Staged), 0);
        assert_eq!(view.count(Section::Recent), 1);
    }

    #[test]
    fn a_row_knows_which_section_it_is_in() {
        let mut view = view();
        view.toggle_file(Section::Unstaged, "a.rs");
        let rows = view.lay_out();
        for row in &rows {
            match row {
                Row::Header(_) | Row::Blank => assert_eq!(row.section(), None),
                other => assert!(other.section().is_some(), "{other:?} has no section"),
            }
        }
    }

    #[test]
    fn folding_everything_leaves_only_the_headings() {
        let mut view = view();
        view.toggle_file(Section::Unstaged, "a.rs");
        view.collapse_all();
        let rows = view.lay_out();
        assert!(
            !rows
                .iter()
                .any(|row| matches!(row, Row::Line { .. } | Row::File { .. }))
        );

        view.expand_all();
        let rows = view.lay_out();
        assert!(rows.iter().any(|row| matches!(row, Row::File { .. })));
        assert!(
            !rows.iter().any(|row| matches!(row, Row::Line { .. })),
            "unfolding sections should not also unfold every file"
        );
    }

    #[test]
    fn a_clean_repository_says_so_once_it_has_looked() {
        let mut view = GitView::new();
        view.status = maxgus_git::status::parse(b"# branch.head main\0");
        assert!(
            view.lay_out()
                .iter()
                .all(|row| !matches!(row, Row::Empty(_))),
            "not asked yet"
        );

        view.loaded = true;
        assert!(
            view.lay_out()
                .iter()
                .any(|row| matches!(row, Row::Empty(_))),
            "a clean tree should say so rather than showing a blank buffer"
        );
    }
}

// ---- the other buffers --------------------------------------------------

/// A buffer of diffs whose files and hunks fold: the revision view and the
/// diff view, which differ only in what is above the diffs.
#[derive(Debug, Clone, Default)]
pub struct DiffView {
    /// What the buffer is called, for its own first line.
    pub title: String,
    /// Lines above the diff — a commit's author and message — with the face
    /// each is drawn in.
    pub preamble: Vec<(String, &'static str)>,
    pub files: Vec<maxgus_git::FileDiff>,
    collapsed: BTreeSet<String>,
    rows: Vec<DiffRow>,
}

/// One line of a diff buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffRow {
    Title,
    Preamble(usize),
    Blank,
    File(usize),
    Hunk(usize, usize),
    Line(usize, usize, usize),
    Empty,
}

impl DiffView {
    /// A view of some files, with a title and whatever goes above them.
    pub fn new(
        title: impl Into<String>,
        preamble: Vec<(String, &'static str)>,
        files: Vec<maxgus_git::FileDiff>,
    ) -> DiffView {
        DiffView {
            title: title.into(),
            preamble,
            files,
            ..DiffView::default()
        }
    }

    pub fn rows(&self) -> &[DiffRow] {
        &self.rows
    }

    pub fn row(&self, line: usize) -> Option<&DiffRow> {
        self.rows.get(line)
    }

    pub fn line_of(&self, row: &DiffRow) -> Option<usize> {
        self.rows.iter().position(|candidate| candidate == row)
    }

    pub fn is_collapsed(&self, path: &str) -> bool {
        self.collapsed.contains(path)
    }

    pub fn toggle_file(&mut self, path: &str) {
        if !self.collapsed.insert(path.to_string()) {
            self.collapsed.remove(path);
        }
    }

    /// How many lines added and removed across every file.
    pub fn counts(&self) -> (usize, usize) {
        self.files.iter().fold((0, 0), |(a, r), file| {
            let (added, removed) = file.counts();
            (a + added, r + removed)
        })
    }

    pub fn lay_out(&mut self) -> Vec<DiffRow> {
        let mut rows = vec![DiffRow::Title];
        rows.extend((0..self.preamble.len()).map(DiffRow::Preamble));
        rows.push(DiffRow::Blank);
        if self.files.is_empty() {
            rows.push(DiffRow::Empty);
        }
        for (index, file) in self.files.iter().enumerate() {
            rows.push(DiffRow::File(index));
            // Expanded by default here, unlike the status: a diff buffer was
            // opened to read the diff.
            if self.collapsed.contains(&file.path) {
                continue;
            }
            for (hunk_index, hunk) in file.hunks.iter().enumerate() {
                rows.push(DiffRow::Hunk(index, hunk_index));
                rows.extend(
                    (0..hunk.lines.len()).map(|line| DiffRow::Line(index, hunk_index, line)),
                );
            }
        }
        self.rows = rows.clone();
        rows
    }
}

/// A buffer that is a list of lines: the log, the references, and the record
/// of what git was asked to do.
///
/// One type for the three because they differ only in what a line means, and
/// a line that can be acted on carries what to act on with it.
#[derive(Debug, Clone, Default)]
pub struct ListView {
    pub title: String,
    pub lines: Vec<ListLine>,
}

/// One line, its faces, and what acting on it refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListLine {
    /// Runs of text, each with the face it is drawn in.
    pub spans: Vec<(String, &'static str)>,
    /// A commit or a branch this line stands for, for `RET`.
    pub target: Option<String>,
}

impl ListLine {
    pub fn plain(text: impl Into<String>, face: &'static str) -> ListLine {
        ListLine {
            spans: vec![(text.into(), face)],
            target: None,
        }
    }

    pub fn text(&self) -> String {
        self.spans.iter().map(|(text, _)| text.as_str()).collect()
    }
}

impl ListView {
    /// The commits of a log, as lines.
    pub fn from_log(title: impl Into<String>, commits: &[maxgus_git::Commit]) -> ListView {
        let mut lines = Vec::new();
        for commit in commits {
            let mut spans = vec![(format!("{} ", commit.short), "magit-hash")];
            for reference in &commit.refs {
                let face = if reference.starts_with("tag: ") {
                    "magit-tag"
                } else if reference.contains('/') {
                    "magit-branch-remote"
                } else {
                    "magit-branch-local"
                };
                spans.push((format!("{} ", reference.trim_start_matches("tag: ")), face));
            }
            spans.push((commit.subject.clone(), "default"));
            spans.push((format!("  {}", commit.when), "shadow"));
            lines.push(ListLine {
                spans,
                target: Some(commit.hash.clone()),
            });
        }
        if lines.is_empty() {
            lines.push(ListLine::plain("Nothing to show", "shadow"));
        }
        ListView {
            title: title.into(),
            lines,
        }
    }

    /// Branches and tags, grouped by what they actually are.
    pub fn from_refs(references: &[maxgus_git::Reference], head: Option<&str>) -> ListView {
        use maxgus_git::RefKind;
        let mut lines = Vec::new();
        let of = |kind: RefKind| -> Vec<&maxgus_git::Reference> {
            references
                .iter()
                .filter(|reference| reference.kind == kind)
                .collect()
        };
        for (title, group, face) in [
            ("Branches", of(RefKind::Local), "magit-branch-local"),
            ("Remotes", of(RefKind::Remote), "magit-branch-remote"),
            ("Tags", of(RefKind::Tag), "magit-tag"),
        ] {
            if group.is_empty() {
                continue;
            }
            lines.push(ListLine::plain(title, "magit-section-heading"));
            for reference in group {
                // The branch checked out is marked, as `git branch` marks it.
                let marker = if Some(reference.name.as_str()) == head {
                    "* "
                } else {
                    "  "
                };
                lines.push(ListLine {
                    spans: vec![
                        (marker.to_string(), "shadow"),
                        (reference.name.clone(), face),
                    ],
                    target: Some(reference.name.clone()),
                });
            }
            lines.push(ListLine::plain("", "default"));
        }
        if lines.is_empty() {
            lines.push(ListLine::plain("No branches", "shadow"));
        }
        ListView {
            title: "References".into(),
            lines,
        }
    }
}

#[cfg(test)]
mod movement_tests {
    use super::*;

    #[test]
    fn what_counts_as_a_section_is_what_n_and_p_stop_at() {
        // Stepping through the lines of a hunk one at a time is what `C-n` is
        // for; `n` is for getting about, so it does not stop on them.
        assert_eq!(Row::Section(Section::Staged).level(), Some(0));
        assert_eq!(
            Row::File {
                section: Section::Staged,
                file: 0
            }
            .level(),
            Some(1)
        );
        assert_eq!(Row::Stash(0).level(), Some(1));
        assert_eq!(
            Row::Commit {
                section: Section::Recent,
                commit: 0
            }
            .level(),
            Some(1)
        );
        assert_eq!(
            Row::Hunk {
                section: Section::Staged,
                file: 0,
                hunk: 0
            }
            .level(),
            Some(2)
        );
        assert_eq!(
            Row::Line {
                section: Section::Staged,
                file: 0,
                hunk: 0,
                line: 0
            }
            .level(),
            None
        );
        assert_eq!(Row::Blank.level(), None);
    }
}

#[cfg(test)]
mod other_buffer_tests {
    use super::*;

    const DIFF_TEXT: &str = "\
diff --git a/a.rs b/a.rs
index 111..222 100644
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,2 @@
-old
+new
 tail
diff --git a/b.rs b/b.rs
index 333..444 100644
--- a/b.rs
+++ b/b.rs
@@ -9 +9 @@
-x
+y
";

    fn diff_view() -> DiffView {
        DiffView {
            title: "commit abc1234".into(),
            preamble: vec![("Author: Someone".into(), "shadow")],
            files: maxgus_git::diff::parse(DIFF_TEXT),
            ..DiffView::default()
        }
    }

    #[test]
    fn a_diff_buffer_shows_its_files_open() {
        // Unlike the status view: this buffer was opened to read the diff.
        let mut view = diff_view();
        let rows = view.lay_out();
        assert_eq!(rows[0], DiffRow::Title);
        assert_eq!(rows[1], DiffRow::Preamble(0));
        assert!(rows.contains(&DiffRow::File(0)));
        assert!(rows.contains(&DiffRow::Hunk(0, 0)));
        assert!(
            rows.contains(&DiffRow::Line(0, 0, 0)),
            "the lines are not shown"
        );
        assert!(
            rows.contains(&DiffRow::File(1)),
            "the second file is missing"
        );
    }

    #[test]
    fn folding_a_file_in_a_diff_buffer_hides_only_that_file() {
        let mut view = diff_view();
        view.lay_out();
        view.toggle_file("a.rs");
        let rows = view.lay_out();
        assert!(rows.contains(&DiffRow::File(0)), "the heading should stay");
        assert!(
            !rows.contains(&DiffRow::Hunk(0, 0)),
            "the first file did not fold"
        );
        assert!(
            rows.contains(&DiffRow::Hunk(1, 0)),
            "the second file folded too"
        );
    }

    #[test]
    fn an_empty_diff_says_so() {
        let mut view = DiffView {
            title: "nothing".into(),
            ..DiffView::default()
        };
        assert!(view.lay_out().contains(&DiffRow::Empty));
    }

    #[test]
    fn the_whole_diff_is_counted_across_its_files() {
        let view = diff_view();
        assert_eq!(view.counts(), (2, 2));
    }

    #[test]
    fn a_log_line_carries_the_commit_it_stands_for() {
        // `RET` has to know which commit, and a position in a list is not it.
        let commits = maxgus_git::log::parse_log(
            "hash1\u{1f}abc1234\u{1f}Someone\u{1f}an hour ago\u{1f}HEAD -> main, tag: v1\u{1f}a change\u{1e}\n",
        );
        let view = ListView::from_log("Log", &commits);
        assert_eq!(view.lines.len(), 1);
        assert_eq!(view.lines[0].target.as_deref(), Some("hash1"));
        let text = view.lines[0].text();
        assert!(text.contains("abc1234"), "no hash: `{text}`");
        assert!(text.contains("a change"));
        assert!(text.contains("an hour ago"));
        // Refs are drawn in their own faces, which is what tells a tag from a
        // branch at a glance.
        assert!(
            view.lines[0]
                .spans
                .iter()
                .any(|(_, face)| *face == "magit-tag")
        );
        assert!(
            view.lines[0]
                .spans
                .iter()
                .any(|(_, face)| *face == "magit-branch-local")
        );
    }

    #[test]
    fn an_empty_log_says_so_rather_than_showing_nothing() {
        let view = ListView::from_log("Log", &[]);
        assert_eq!(view.lines.len(), 1);
        assert!(view.lines[0].text().contains("Nothing"));
    }

    #[test]
    fn references_are_grouped_by_what_they_are_and_the_current_branch_marked() {
        use maxgus_git::{RefKind, Reference};
        let reference = |name: &str, kind| Reference {
            name: name.into(),
            kind,
        };
        let references = [
            reference("main", RefKind::Local),
            // A local branch with a slash in it. Grouping by the slash would
            // file this under remotes, which is why the kind is carried.
            reference("feature/x", RefKind::Local),
            reference("origin/main", RefKind::Remote),
            reference("v1.0", RefKind::Tag),
        ];
        let view = ListView::from_refs(&references, Some("main"));
        let text: Vec<String> = view.lines.iter().map(|line| line.text()).collect();
        let at = |needle: &str| text.iter().position(|line| line == needle);

        assert!(
            at("* main").is_some(),
            "the current branch is not marked: {text:?}"
        );
        let branches = at("Branches").expect("a branches heading");
        let remotes = at("Remotes").expect("a remotes heading");
        let tags = at("Tags").expect("a tags heading");
        assert!(
            at("  feature/x").expect("the branch") < remotes,
            "a local branch was filed as a remote"
        );
        assert!(at("  origin/main").expect("the remote") > remotes);
        assert!(at("  v1.0").expect("the tag") > tags);
        assert!(branches < remotes && remotes < tags);
    }
}
