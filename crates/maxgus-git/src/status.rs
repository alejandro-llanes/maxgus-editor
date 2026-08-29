//! Reading `git status --porcelain=v2 -z --branch`.
//!
//! Porcelain v2 rather than v1: it carries the branch, the tracking counts and
//! rename information, all of which the status view shows, and its format is
//! documented as stable — v1's `XY` codes are not enough to tell a rename from
//! an add without guessing.
//!
//! `-z` rather than newline-separated, because a path may contain a newline.
//! Git will happily quote such a path in the readable form; parsing the
//! quoting back is a source of bugs that not asking for it avoids entirely.

use std::path::PathBuf;

/// What happened to a file on one side, the index or the working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Change {
    #[default]
    None,
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    /// Both sides changed it and the merge is unresolved.
    Unmerged,
}

impl Change {
    fn from_code(code: u8) -> Change {
        match code {
            b'A' => Change::Added,
            b'M' => Change::Modified,
            b'D' => Change::Deleted,
            b'R' => Change::Renamed,
            b'C' => Change::Copied,
            b'T' => Change::TypeChanged,
            b'U' => Change::Unmerged,
            _ => Change::None,
        }
    }

    pub fn is_change(self) -> bool {
        self != Change::None
    }

    /// The word the status view puts in front of the path.
    pub fn label(self) -> &'static str {
        match self {
            Change::None => "",
            Change::Added => "new file",
            Change::Modified => "modified",
            Change::Deleted => "deleted",
            Change::Renamed => "renamed",
            Change::Copied => "copied",
            Change::TypeChanged => "typechange",
            Change::Unmerged => "unmerged",
        }
    }
}

/// One path git has something to say about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    /// Where a rename or copy came from.
    pub original: Option<PathBuf>,
    /// What is staged for the next commit.
    pub index: Change,
    /// What is changed but not staged.
    pub worktree: Change,
    pub untracked: bool,
    pub ignored: bool,
    pub unmerged: bool,
}

impl Entry {
    fn blank(path: PathBuf) -> Entry {
        Entry {
            path,
            original: None,
            index: Change::None,
            worktree: Change::None,
            untracked: false,
            ignored: false,
            unmerged: false,
        }
    }
}

/// Where the repository is and what is in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// `None` on a detached head.
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// The commit `HEAD` is at, or `None` before the first commit.
    pub head: Option<String>,
    pub entries: Vec<Entry>,
}

impl Status {
    /// True when there is nothing at all to commit.
    pub fn is_clean(&self) -> bool {
        self.entries.iter().all(|entry| entry.ignored)
    }

    pub fn staged(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|entry| entry.index.is_change() && !entry.unmerged)
    }

    pub fn unstaged(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|entry| entry.worktree.is_change() && !entry.untracked && !entry.unmerged)
    }

    pub fn untracked(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|entry| entry.untracked)
    }

    pub fn unmerged(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|entry| entry.unmerged)
    }
}

/// Parses the output of `git status --porcelain=v2 -z --branch`.
pub fn parse(output: &[u8]) -> Status {
    let mut status = Status::default();
    // `-z` terminates every record with a NUL, and a rename's original path
    // is a record of its own immediately after the entry that names it.
    let mut records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());

    while let Some(record) = records.next() {
        let text = String::from_utf8_lossy(record).into_owned();
        let mut fields = text.split(' ');
        match fields.next() {
            Some("#") => header(&mut status, &text),
            // `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`
            Some("1") => {
                if let Some(entry) = ordinary(&text, 8) {
                    status.entries.push(entry);
                }
            }
            // `2 <XY> ... <X><score> <path>` and the original path follows.
            Some("2") => {
                if let Some(mut entry) = ordinary(&text, 9) {
                    entry.original = records
                        .next()
                        .map(|path| PathBuf::from(String::from_utf8_lossy(path).into_owned()));
                    status.entries.push(entry);
                }
            }
            // `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`
            Some("u") => {
                if let Some(path) = field_after(&text, 10) {
                    let mut entry = Entry::blank(PathBuf::from(path));
                    entry.unmerged = true;
                    let codes = text.split(' ').nth(1).unwrap_or("").as_bytes();
                    entry.index = codes
                        .first()
                        .map(|c| Change::from_code(*c))
                        .unwrap_or_default();
                    entry.worktree = codes
                        .get(1)
                        .map(|c| Change::from_code(*c))
                        .unwrap_or_default();
                    status.entries.push(entry);
                }
            }
            Some("?") => {
                if let Some(path) = field_after(&text, 1) {
                    let mut entry = Entry::blank(PathBuf::from(path));
                    entry.untracked = true;
                    status.entries.push(entry);
                }
            }
            Some("!") => {
                if let Some(path) = field_after(&text, 1) {
                    let mut entry = Entry::blank(PathBuf::from(path));
                    entry.ignored = true;
                    status.entries.push(entry);
                }
            }
            _ => {}
        }
    }
    status
}

/// A `1` or `2` record: the two change codes and the path.
fn ordinary(text: &str, path_field: usize) -> Option<Entry> {
    let codes = text.split(' ').nth(1)?.as_bytes();
    let path = field_after(text, path_field)?;
    let mut entry = Entry::blank(PathBuf::from(path));
    entry.index = codes
        .first()
        .map(|c| Change::from_code(*c))
        .unwrap_or_default();
    entry.worktree = codes
        .get(1)
        .map(|c| Change::from_code(*c))
        .unwrap_or_default();
    Some(entry)
}

/// Everything from field `n` to the end, which is how a path that contains
/// spaces stays in one piece.
fn field_after(text: &str, n: usize) -> Option<String> {
    let mut rest = text;
    for _ in 0..n {
        let (_, after) = rest.split_once(' ')?;
        rest = after;
    }
    (!rest.is_empty()).then(|| rest.to_string())
}

fn header(status: &mut Status, text: &str) {
    let mut fields = text.split(' ').skip(1);
    match (fields.next(), fields.next()) {
        (Some("branch.oid"), Some("(initial)")) => status.head = None,
        (Some("branch.oid"), Some(oid)) => status.head = Some(oid.to_string()),
        (Some("branch.head"), Some("(detached)")) => status.branch = None,
        (Some("branch.head"), Some(branch)) => status.branch = Some(branch.to_string()),
        (Some("branch.upstream"), Some(upstream)) => {
            status.upstream = Some(upstream.to_string());
        }
        (Some("branch.ab"), Some(ahead)) => {
            status.ahead = ahead.trim_start_matches('+').parse().unwrap_or(0);
            status.behind = fields
                .next()
                .unwrap_or("-0")
                .trim_start_matches('-')
                .parse()
                .unwrap_or(0);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds `-z` output from records, which is how git actually sends it.
    fn zero_terminated(records: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for record in records {
            out.extend_from_slice(record.as_bytes());
            out.push(0);
        }
        out
    }

    #[test]
    fn the_branch_and_its_tracking_are_read_from_the_header() {
        let output = zero_terminated(&[
            "# branch.oid 5958f5e13418d8b5d31f856238ffd96e9174d1d3",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +3 -2",
        ]);
        let status = parse(&output);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind), (3, 2));
        assert!(status.head.is_some());
        assert!(status.is_clean());
    }

    #[test]
    fn a_repository_with_no_commits_yet_has_no_head() {
        let output = zero_terminated(&["# branch.oid (initial)", "# branch.head main"]);
        let status = parse(&output);
        assert_eq!(status.head, None, "there is nothing to be at yet");
        assert_eq!(status.branch.as_deref(), Some("main"));
    }

    #[test]
    fn a_detached_head_has_a_commit_but_no_branch() {
        let output = zero_terminated(&["# branch.oid abc123", "# branch.head (detached)"]);
        let status = parse(&output);
        assert_eq!(status.branch, None);
        assert_eq!(status.head.as_deref(), Some("abc123"));
    }

    #[test]
    fn the_two_sides_of_a_change_are_told_apart() {
        // `MM` is the case that matters: staged *and* further changed since.
        // Showing it in one place only would hide half of what is going on.
        let output = zero_terminated(&[
            "1 M. N... 100644 100644 100644 aaa bbb staged.rs",
            "1 .M N... 100644 100644 100644 aaa bbb unstaged.rs",
            "1 MM N... 100644 100644 100644 aaa bbb both.rs",
            "1 A. N... 100644 100644 100644 aaa bbb added.rs",
            "1 .D N... 100644 100644 100644 aaa bbb gone.rs",
        ]);
        let status = parse(&output);
        let staged: Vec<_> = status
            .staged()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(staged, ["staged.rs", "both.rs", "added.rs"]);
        let unstaged: Vec<_> = status
            .unstaged()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(unstaged, ["unstaged.rs", "both.rs", "gone.rs"]);
        assert_eq!(status.entries[3].index, Change::Added);
        assert_eq!(status.entries[4].worktree, Change::Deleted);
    }

    #[test]
    fn a_rename_carries_the_name_it_came_from() {
        // The original path is a record of its own, straight after the entry.
        let output = zero_terminated(&[
            "2 R. N... 100644 100644 100644 aaa bbb R100 new/name.rs",
            "old/name.rs",
        ]);
        let status = parse(&output);
        assert_eq!(
            status.entries.len(),
            1,
            "the original path was read as an entry"
        );
        assert_eq!(status.entries[0].path.to_string_lossy(), "new/name.rs");
        assert_eq!(
            status.entries[0]
                .original
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            Some("old/name.rs".to_string())
        );
        assert_eq!(status.entries[0].index, Change::Renamed);
    }

    #[test]
    fn untracked_ignored_and_unmerged_are_each_their_own_thing() {
        let output = zero_terminated(&[
            "? new.rs",
            "! target/debug",
            "u UU N... 100644 100644 100644 100644 aaa bbb ccc conflict.rs",
        ]);
        let status = parse(&output);
        assert_eq!(status.untracked().count(), 1);
        assert_eq!(status.unmerged().count(), 1);
        assert!(status.entries.iter().any(|e| e.ignored));
        // An unresolved merge is not offered as staged or unstaged: it is
        // neither until it is resolved.
        assert_eq!(status.staged().count(), 0);
        assert_eq!(status.unstaged().count(), 0);
    }

    #[test]
    fn a_path_with_spaces_in_it_survives() {
        // The reason for `-z` and for reading the path as everything after
        // the last field rather than as one space-delimited word.
        let output =
            zero_terminated(&["1 .M N... 100644 100644 100644 aaa bbb my notes/two words.md"]);
        let status = parse(&output);
        assert_eq!(
            status.entries[0].path.to_string_lossy(),
            "my notes/two words.md"
        );
    }

    #[test]
    fn a_truncated_record_is_dropped_rather_than_guessed_at() {
        let status = parse(&zero_terminated(&["1 .M N...", "?", "", "# branch.ab"]));
        assert!(status.entries.is_empty(), "half a record became an entry");
    }

    #[test]
    fn a_tree_with_only_ignored_files_in_it_is_clean() {
        // Ignored files are noise. Reporting them as changes would mean a
        // `target` directory made every repository look dirty.
        let status = parse(&zero_terminated(&["! target/debug", "! node_modules"]));
        assert_eq!(status.entries.len(), 2);
        assert!(status.is_clean(), "ignored files made the tree look dirty");
    }
}
