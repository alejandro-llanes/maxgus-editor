//! Reading a diff, and writing a patch that applies one piece of it.
//!
//! Staging a hunk means handing git a patch containing only that hunk. The
//! header lines are kept exactly as git wrote them and copied through
//! untouched rather than rebuilt: they carry the blob hashes, the file modes
//! and the rename information, and a patch reconstructed from a parse is a
//! patch that can differ from what was read.
//!
//! Nothing here runs git. It is given output and returns text, so every case
//! that matters — a new file, a deletion, a rename, a file with no trailing
//! newline — is a test rather than a repository.

/// What one line of a hunk is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file`, which belongs to the line above it and
    /// must travel with it or the patch changes the file's last byte.
    NoNewline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// The text without its leading marker.
    pub text: String,
}

impl DiffLine {
    /// The line as it appears in a patch, marker and all.
    pub fn to_patch_line(&self) -> String {
        match self.kind {
            LineKind::Context => format!(" {}", self.text),
            LineKind::Added => format!("+{}", self.text),
            LineKind::Removed => format!("-{}", self.text),
            LineKind::NoNewline => "\\ No newline at end of file".to_string(),
        }
    }
}

/// One `@@` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The `@@ -1,3 +1,4 @@ fn main` line, verbatim.
    pub header: String,
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    /// How many lines the hunk adds and removes, for the summary.
    pub fn counts(&self) -> (usize, usize) {
        let added = self
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Added)
            .count();
        let removed = self
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Removed)
            .count();
        (added, removed)
    }
}

/// Everything git said about one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// The path as it is now.
    pub path: String,
    /// Where a rename came from.
    pub old_path: Option<String>,
    /// Every line before the first `@@`, kept exactly as git wrote it.
    pub header: Vec<String>,
    pub hunks: Vec<Hunk>,
    pub binary: bool,
}

impl FileDiff {
    pub fn counts(&self) -> (usize, usize) {
        self.hunks.iter().fold((0, 0), |(a, r), hunk| {
            let (added, removed) = hunk.counts();
            (a + added, r + removed)
        })
    }
}

/// Parses `git diff` output into one entry per file.
pub fn parse(output: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let (old_path, path) = paths(rest);
            files.push(FileDiff {
                path,
                old_path,
                header: vec![line.to_string()],
                hunks: Vec::new(),
                binary: false,
            });
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };

        if line.starts_with("@@") {
            if let Some(hunk) = hunk_header(line) {
                file.hunks.push(hunk);
            }
            continue;
        }
        if file.hunks.is_empty() {
            if line.starts_with("Binary files") || line.starts_with("GIT binary patch") {
                file.binary = true;
            }
            // The `---`/`+++` lines are the ones to believe. `diff --git`
            // carries whatever prefixes git was configured with — modern git
            // uses `i/` and `w/` for a worktree diff, not `a/` and `b/` —
            // and splitting that line on a guessed prefix gets it wrong.
            if let Some(old) = line.strip_prefix("--- ") {
                file.old_path = strip_prefix(old);
            }
            if let Some(new) = line.strip_prefix("+++ ")
                && let Some(path) = strip_prefix(new)
            {
                file.path = path;
            }
            // A rename says both names outright, which settles it.
            if let Some(from) = line.strip_prefix("rename from ") {
                file.old_path = Some(from.to_string());
            }
            if let Some(to) = line.strip_prefix("rename to ") {
                file.path = to.to_string();
            }
            file.header.push(line.to_string());
            continue;
        }

        let Some(hunk) = file.hunks.last_mut() else {
            continue;
        };
        let kind = match line.as_bytes().first() {
            Some(b' ') => LineKind::Context,
            Some(b'+') => LineKind::Added,
            Some(b'-') => LineKind::Removed,
            Some(b'\\') => LineKind::NoNewline,
            // An empty line inside a hunk is a context line whose trailing
            // space git dropped. Treating it as the end of the hunk loses it.
            None => LineKind::Context,
            _ => continue,
        };
        let text = match kind {
            LineKind::NoNewline => String::new(),
            _ => line.get(1..).unwrap_or_default().to_string(),
        };
        hunk.lines.push(DiffLine { kind, text });
    }
    // `--- a/x` sets the old path for every file, not only renamed ones, so
    // a path equal to the new one means no rename after all.
    for file in &mut files {
        if file.old_path.as_deref() == Some(file.path.as_str()) {
            file.old_path = None;
        }
    }
    files
}

/// The two paths on a `diff --git` line, as a first guess.
///
/// Only a guess: the prefix is whatever git chose, and a path may contain the
/// separator being split on. The `---`/`+++` lines that follow overwrite both,
/// and a rename's own lines overwrite them again.
fn paths(rest: &str) -> (Option<String>, String) {
    let halves: Vec<&str> = rest.split(' ').collect();
    let new = halves
        .last()
        .and_then(|half| strip_prefix(half))
        .unwrap_or_default();
    let old = halves.first().and_then(|half| strip_prefix(half));
    (old.filter(|old| *old != new), new)
}

/// Takes git's one-letter path prefix off, and answers `None` for `/dev/null`.
///
/// The prefix is a single component of one character: `a/` and `b/` by
/// default, `i/`, `w/`, `c/` and `o/` when git is being mnemonic about it.
fn strip_prefix(path: &str) -> Option<String> {
    let path = path.trim_end_matches('\t');
    if path == "/dev/null" {
        return None;
    }
    match path.split_once('/') {
        Some((prefix, rest)) if prefix.chars().count() == 1 => Some(rest.to_string()),
        _ => Some(path.to_string()),
    }
}

/// `@@ -1,3 +1,4 @@ context`.
fn hunk_header(line: &str) -> Option<Hunk> {
    let inner = line.strip_prefix("@@ ")?.split(" @@").next()?;
    let (old, new) = inner.split_once(' ')?;
    let (old_start, old_lines) = range(old.strip_prefix('-')?);
    let (new_start, new_lines) = range(new.strip_prefix('+')?);
    Some(Hunk {
        header: line.to_string(),
        old_start,
        old_lines,
        new_start,
        new_lines,
        lines: Vec::new(),
    })
}

/// `12,3` or `12`, where a missing count means one line.
fn range(text: &str) -> (usize, usize) {
    match text.split_once(',') {
        Some((start, count)) => (start.parse().unwrap_or(0), count.parse().unwrap_or(0)),
        None => (text.parse().unwrap_or(0), 1),
    }
}

/// A patch containing exactly one hunk of one file.
///
/// Given to `git apply --cached` it stages that hunk; with `--reverse` it
/// unstages one, and without `--cached` it discards one. The same text serves
/// all three, which is why nothing here needs to know which is meant.
pub fn hunk_patch(file: &FileDiff, hunk: &Hunk) -> String {
    let mut patch = String::new();
    for line in &file.header {
        patch.push_str(line);
        patch.push('\n');
    }
    patch.push_str(&hunk.header);
    patch.push('\n');
    for line in &hunk.lines {
        patch.push_str(&line.to_patch_line());
        patch.push('\n');
    }
    patch
}

/// A patch containing every hunk of one file.
pub fn file_patch(file: &FileDiff) -> String {
    let mut patch = String::new();
    for line in &file.header {
        patch.push_str(line);
        patch.push('\n');
    }
    for hunk in &file.hunks {
        patch.push_str(&hunk.header);
        patch.push('\n');
        for line in &hunk.lines {
            patch.push_str(&line.to_patch_line());
            patch.push('\n');
        }
    }
    patch
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE_FILE: &str = "\
diff --git a/src/main.rs b/src/main.rs
index 83db48f..bf269f4 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@ fn main
 fn main() {
-    println!(\"old\");
+    println!(\"new\");
+    println!(\"and more\");
 }
";

    #[test]
    fn a_diff_is_split_into_files_hunks_and_lines() {
        let files = parse(ONE_FILE);
        assert_eq!(files.len(), 1);
        let file = &files[0];
        assert_eq!(file.path, "src/main.rs");
        assert_eq!(file.old_path, None, "not a rename");
        assert_eq!(file.hunks.len(), 1);

        let hunk = &file.hunks[0];
        assert_eq!((hunk.old_start, hunk.old_lines), (1, 3));
        assert_eq!((hunk.new_start, hunk.new_lines), (1, 4));
        assert_eq!(hunk.counts(), (2, 1), "two added, one removed");
        assert_eq!(hunk.lines[0].kind, LineKind::Context);
        assert_eq!(hunk.lines[1].kind, LineKind::Removed);
        assert_eq!(hunk.lines[1].text, "    println!(\"old\");");
    }

    #[test]
    fn the_header_is_kept_exactly_as_git_wrote_it() {
        // It carries the blob hashes and the mode. Rebuilding it from a parse
        // is how a patch comes to differ from what was read.
        let file = &parse(ONE_FILE)[0];
        assert_eq!(file.header[0], "diff --git a/src/main.rs b/src/main.rs");
        assert_eq!(file.header[1], "index 83db48f..bf269f4 100644");
        assert_eq!(file.header.len(), 4, "the ---/+++ lines belong to it too");
    }

    #[test]
    fn a_hunk_patch_is_the_header_and_that_hunk_alone() {
        let file = &parse(ONE_FILE)[0];
        let patch = hunk_patch(file, &file.hunks[0]);
        assert!(patch.starts_with("diff --git a/src/main.rs b/src/main.rs\n"));
        assert!(patch.contains("@@ -1,3 +1,4 @@ fn main\n"));
        assert!(patch.contains("-    println!(\"old\");\n"));
        assert!(patch.ends_with(" }\n"), "a patch has to end with a newline");
    }

    #[test]
    fn one_hunk_of_several_is_taken_on_its_own() {
        // The whole point of staging by hunk: the other hunk must not appear.
        let two = "\
diff --git a/a.rs b/a.rs
index 111..222 100644
--- a/a.rs
+++ b/a.rs
@@ -1,2 +1,2 @@
-first old
+first new
 tail
@@ -10,2 +10,2 @@
-second old
+second new
 tail
";
        let file = &parse(two)[0];
        assert_eq!(file.hunks.len(), 2);
        let patch = hunk_patch(file, &file.hunks[1]);
        assert!(patch.contains("second new"));
        assert!(
            !patch.contains("first new"),
            "the other hunk came too:\n{patch}"
        );
        assert_eq!(
            patch.matches("@@").count(),
            2,
            "one hunk header, twice on the line"
        );
    }

    #[test]
    fn several_files_are_kept_apart() {
        let both = format!(
            "{ONE_FILE}{}",
            "\
diff --git a/other.rs b/other.rs
index aaa..bbb 100644
--- a/other.rs
+++ b/other.rs
@@ -5 +5 @@
-x
+y
"
        );
        let files = parse(&both);
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].path, "other.rs");
        // A range with no comma is one line.
        assert_eq!(
            (files[1].hunks[0].old_start, files[1].hunks[0].old_lines),
            (5, 1)
        );
    }

    #[test]
    fn a_new_file_and_a_deleted_one_keep_their_dev_null() {
        let new = "\
diff --git a/new.rs b/new.rs
new file mode 100644
index 0000000..e69de29
--- /dev/null
+++ b/new.rs
@@ -0,0 +1,2 @@
+one
+two
";
        let file = &parse(new)[0];
        assert!(file.header.iter().any(|l| l == "--- /dev/null"));
        assert!(file.header.iter().any(|l| l == "new file mode 100644"));
        let patch = hunk_patch(file, &file.hunks[0]);
        assert!(
            patch.contains("--- /dev/null"),
            "a new file must still come from nowhere"
        );
    }

    #[test]
    fn a_rename_takes_its_real_paths_from_the_rename_lines() {
        // `diff --git` uses whatever prefixes git was configured with; the
        // `rename from`/`to` lines are unambiguous.
        let renamed = "\
diff --git a/old/name.rs b/new/name.rs
similarity index 95%
rename from old/name.rs
rename to new/name.rs
index aaa..bbb 100644
--- a/old/name.rs
+++ b/new/name.rs
@@ -1 +1 @@
-a
+b
";
        let file = &parse(renamed)[0];
        assert_eq!(file.path, "new/name.rs");
        assert_eq!(file.old_path.as_deref(), Some("old/name.rs"));
    }

    #[test]
    fn a_path_with_spaces_is_taken_from_the_lines_that_can_be_trusted() {
        // The `diff --git` line cannot be split on a space when the path has
        // one in it, and cannot be split on a prefix that is not known in
        // advance. The `---`/`+++` lines have one path each and no ambiguity.
        let spaced = "\
diff --git a/my notes/two words.md b/my notes/two words.md
index aaa..bbb 100644
--- a/my notes/two words.md
+++ b/my notes/two words.md
@@ -1 +1 @@
-a
+b
";
        let file = &parse(spaced)[0];
        assert_eq!(file.path, "my notes/two words.md");
        assert_eq!(file.old_path, None, "the same path is not a rename");
    }

    #[test]
    fn a_mnemonic_prefix_is_stripped_like_any_other() {
        // Git writes `i/` and `w/` rather than `a/` and `b/` when
        // `diff.mnemonicPrefix` is on, which it is by default for some.
        let mnemonic = "\
diff --git i/src/a.rs w/src/a.rs
index aaa..bbb 100644
--- i/src/a.rs
+++ w/src/a.rs
@@ -1 +1 @@
-a
+b
";
        assert_eq!(parse(mnemonic)[0].path, "src/a.rs");
    }

    #[test]
    fn a_binary_file_is_marked_and_has_no_hunks() {
        let binary = "\
diff --git a/logo.png b/logo.png
index aaa..bbb 100644
Binary files a/logo.png and b/logo.png differ
";
        let file = &parse(binary)[0];
        assert!(file.binary, "a binary change was offered as text");
        assert!(file.hunks.is_empty());
    }

    #[test]
    fn a_missing_final_newline_travels_with_its_line() {
        // Dropping the marker changes the file's last byte, which is a real
        // edit nobody asked for.
        let no_newline = "\
diff --git a/a.txt b/a.txt
index aaa..bbb 100644
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-old
\\ No newline at end of file
+new
\\ No newline at end of file
";
        let file = &parse(no_newline)[0];
        let kinds: Vec<_> = file.hunks[0].lines.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            [
                LineKind::Removed,
                LineKind::NoNewline,
                LineKind::Added,
                LineKind::NoNewline
            ]
        );
        let patch = hunk_patch(file, &file.hunks[0]);
        assert_eq!(patch.matches("\\ No newline at end of file").count(), 2);
    }

    #[test]
    fn an_empty_context_line_is_not_the_end_of_the_hunk() {
        // Git writes a blank context line as an empty line, with the leading
        // space stripped by whatever carried it. Reading that as the end of
        // the hunk silently truncates the patch.
        // Built by hand rather than as a literal: a `\\` line continuation
        // eats the leading whitespace of the next line, which would strip the
        // very space this test is about.
        let with_blank = [
            "diff --git a/a.rs b/a.rs",
            "index aaa..bbb 100644",
            "--- a/a.rs",
            "+++ b/a.rs",
            "@@ -1,4 +1,4 @@",
            " one",
            "",
            "-three",
            "+THREE",
        ]
        .join("\n");
        let with_blank = with_blank.as_str();
        let file = &parse(with_blank)[0];
        assert_eq!(file.hunks[0].lines.len(), 4, "the hunk was cut short");
        assert_eq!(file.hunks[0].lines[1].kind, LineKind::Context);
    }

    #[test]
    fn a_whole_file_patch_carries_every_hunk() {
        let two = "\
diff --git a/a.rs b/a.rs
index 111..222 100644
--- a/a.rs
+++ b/a.rs
@@ -1 +1 @@
-a
+A
@@ -9 +9 @@
-b
+B
";
        let file = &parse(two)[0];
        let patch = file_patch(file);
        assert!(patch.contains("+A") && patch.contains("+B"));
        assert_eq!(patch.matches("@@ -").count(), 2);
    }

    #[test]
    fn nothing_at_all_parses_to_nothing() {
        assert!(parse("").is_empty());
        assert!(parse("not a diff\njust words\n").is_empty());
    }
}
