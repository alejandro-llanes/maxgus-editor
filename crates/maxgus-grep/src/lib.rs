//! Searching a project's files, and writing the answers back.
//!
//! Two halves that belong together: finding the lines that match, and editing
//! those lines in place afterwards. The second is what makes a search worth
//! more than a list — a rename across forty files is a search whose results
//! were edited, which is what `wgrep` does for Emacs.
//!
//! What to search is decided by the same rules a `git status` obeys: the
//! `ignore` crate reads `.gitignore` and its friends, so a search does not
//! spend its time in `target/` and does not report a match in a lockfile.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum GrepError {
    #[error("invalid pattern: {0}")]
    Pattern(#[from] regex::Error),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} has changed since it was searched")]
    Stale { path: PathBuf },
}

pub type Result<T> = std::result::Result<T, GrepError>;

/// One matching line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: PathBuf,
    /// Zero-based, as the editor counts lines.
    pub line: usize,
    /// Where in the line the match starts, in characters.
    pub column: usize,
    /// How many characters it covers, for drawing it.
    pub length: usize,
    /// The whole line, without its ending.
    pub text: String,
}

/// What to search for, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    pub pattern: String,
    /// True to read the pattern as a regular expression rather than as text.
    pub regexp: bool,
    /// `None` is smart case: case-insensitive until the pattern has a capital
    /// in it, which is what every search in this editor already means by it.
    pub case_fold: Option<bool>,
    /// Only files whose name matches one of these, when there are any.
    pub globs: Vec<String>,
    /// Stop after this many hits, so a search for `e` cannot fill memory.
    pub limit: usize,
}

impl Search {
    pub fn new(pattern: &str) -> Search {
        Search {
            pattern: pattern.to_string(),
            regexp: true,
            case_fold: None,
            globs: Vec::new(),
            limit: 5_000,
        }
    }

    fn regex(&self) -> Result<regex::Regex> {
        let pattern = match self.regexp {
            true => self.pattern.clone(),
            false => regex::escape(&self.pattern),
        };
        let fold = self
            .case_fold
            .unwrap_or_else(|| !self.pattern.chars().any(char::is_uppercase));
        Ok(regex::RegexBuilder::new(&pattern)
            .case_insensitive(fold)
            .build()?)
    }
}

/// What a search found, and whether it stopped early.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub hits: Vec<Hit>,
    pub files_searched: usize,
    /// True when the limit was reached and there was more to find.
    pub truncated: bool,
}

/// Runs `search` under `root`.
///
/// Blocking, and meant to be: it is called from the executor's thread pool,
/// where a search of a large tree belongs.
pub fn search(root: &Path, search: &Search) -> Result<Found> {
    let regex = search.regex()?;
    let globs = build_globs(&search.globs);
    let mut found = Found::default();
    let walk = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .parents(true)
        // A directory with a `.gitignore` and no `.git` is still a project
        // whose author said what not to look at.
        .require_git(false)
        .build();
    for entry in walk.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        if let Some(globs) = &globs
            && !globs.matched(path, false).is_whitelist()
        {
            continue;
        }
        let Ok(contents) = std::fs::read(path) else {
            continue;
        };
        // A file with a zero byte in the first block is binary, which is the
        // same rule grep uses and for the same reason: reporting a "match" in
        // a compiled object helps nobody.
        if contents.iter().take(8_000).any(|byte| *byte == 0) {
            continue;
        }
        let Ok(text) = String::from_utf8(contents) else {
            continue;
        };
        found.files_searched += 1;
        for (number, line) in text.lines().enumerate() {
            let Some(m) = regex.find(line) else {
                continue;
            };
            if found.hits.len() >= search.limit {
                found.truncated = true;
                return Ok(found);
            }
            found.hits.push(Hit {
                path: path.to_path_buf(),
                line: number,
                column: line[..m.start()].chars().count(),
                length: m.as_str().chars().count(),
                text: line.to_string(),
            });
        }
    }
    Ok(found)
}

fn build_globs(globs: &[String]) -> Option<ignore::overrides::Override> {
    if globs.is_empty() {
        return None;
    }
    let mut builder = ignore::overrides::OverrideBuilder::new("");
    for glob in globs {
        // A glob that will not parse is dropped rather than failing the whole
        // search: the user typed it into a prompt, and half of it working is
        // better than an error and no results.
        let _ = builder.add(glob);
    }
    builder.build().ok()
}

/// One line to be replaced, as an edited results buffer describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub path: PathBuf,
    pub line: usize,
    /// The line as it was when it was searched, so an edit made against a
    /// file that has since changed is refused rather than applied blind.
    pub was: String,
    pub now: String,
}

/// What applying a set of replacements did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    pub files: usize,
    pub lines: usize,
}

/// Rewrites `text` with the replacements for one file, checking each against
/// the line it was made from.
///
/// Separate from the writing so it can be tested without a filesystem, and so
/// a file the editor already has open can be edited in its buffer instead.
pub fn rewrite(text: &str, replacements: &[Replacement]) -> Result<String> {
    let mut lines: Vec<&str> = text.lines().collect();
    for replacement in replacements {
        let Some(line) = lines.get_mut(replacement.line) else {
            return Err(GrepError::Stale {
                path: replacement.path.clone(),
            });
        };
        if *line != replacement.was {
            return Err(GrepError::Stale {
                path: replacement.path.clone(),
            });
        }
        *line = &replacement.now;
    }
    let mut out = lines.join("\n");
    // A file that ended with a newline still does; one that did not, does not.
    if text.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// Applies replacements to the files on disk, grouped by file.
pub fn apply(replacements: &[Replacement]) -> Result<Applied> {
    let mut by_file: BTreeMap<PathBuf, Vec<Replacement>> = BTreeMap::new();
    for replacement in replacements {
        by_file
            .entry(replacement.path.clone())
            .or_default()
            .push(replacement.clone());
    }
    let mut applied = Applied::default();
    for (path, replacements) in by_file {
        let text = std::fs::read_to_string(&path).map_err(|source| GrepError::Io {
            path: path.clone(),
            source,
        })?;
        let rewritten = rewrite(&text, &replacements)?;
        std::fs::write(&path, &rewritten).map_err(|source| GrepError::Io {
            path: path.clone(),
            source,
        })?;
        applied.files += 1;
        applied.lines += replacements.len();
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("maxgus-grep-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();
        std::fs::write(root.join("src/b.rs"), "// alpha again\nfn gamma() {}\n").unwrap();
        std::fs::write(root.join("target/built.rs"), "fn alpha() {}\n").unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(root.join("notes.txt"), "ALPHA in prose\n").unwrap();
        root
    }

    #[test]
    fn a_search_finds_the_lines_that_match() {
        let root = fixture("find");
        let found = search(&root, &Search::new("alpha")).unwrap();
        let mut paths: Vec<String> = found
            .hits
            .iter()
            .map(|h| h.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        paths.sort();
        assert_eq!(paths, ["a.rs", "b.rs", "notes.txt"]);
    }

    #[test]
    fn what_gitignore_excludes_is_not_searched() {
        let root = fixture("ignored");
        let found = search(&root, &Search::new("alpha")).unwrap();
        assert!(
            !found.hits.iter().any(|h| h.path.ends_with("built.rs")),
            "it searched an ignored directory"
        );
    }

    #[test]
    fn smart_case_is_the_default_and_a_capital_turns_it_off() {
        let root = fixture("case");
        let lower = search(&root, &Search::new("alpha")).unwrap();
        assert!(
            lower.hits.iter().any(|h| h.text.contains("ALPHA")),
            "a lowercase pattern should have matched the uppercase line"
        );
        let upper = search(&root, &Search::new("ALPHA")).unwrap();
        assert_eq!(upper.hits.len(), 1, "a capital should have narrowed it");
        assert!(upper.hits[0].text.contains("ALPHA"));
    }

    #[test]
    fn a_glob_narrows_the_search_to_the_files_that_match_it() {
        let root = fixture("globs");
        let mut wanted = Search::new("alpha");
        wanted.globs = vec!["*.rs".into()];
        let found = search(&root, &wanted).unwrap();
        assert!(
            found
                .hits
                .iter()
                .all(|h| h.path.extension().unwrap() == "rs"),
            "a non-Rust file was searched: {:?}",
            found.hits
        );
        assert_eq!(found.hits.len(), 2);
    }

    #[test]
    fn a_hit_says_where_in_the_line_it_is() {
        let root = fixture("column");
        let found = search(&root, &Search::new("beta")).unwrap();
        let hit = &found.hits[0];
        assert_eq!(hit.line, 1, "lines are counted from zero");
        assert_eq!(hit.column, 3);
        assert_eq!(hit.length, 4);
        assert_eq!(hit.text, "fn beta() {}");
    }

    #[test]
    fn a_limit_stops_the_search_and_says_it_did() {
        let root = fixture("limit");
        let mut wanted = Search::new("a");
        wanted.limit = 2;
        let found = search(&root, &wanted).unwrap();
        assert_eq!(found.hits.len(), 2);
        assert!(found.truncated, "it did not say it had stopped early");
    }

    #[test]
    fn a_binary_file_is_left_alone() {
        let root = fixture("binary");
        std::fs::write(root.join("blob.bin"), b"alpha\0alpha").unwrap();
        let found = search(&root, &Search::new("alpha")).unwrap();
        assert!(
            !found.hits.iter().any(|h| h.path.ends_with("blob.bin")),
            "it reported a match inside a binary"
        );
    }

    #[test]
    fn a_pattern_that_will_not_parse_is_an_error_rather_than_a_panic() {
        let root = fixture("bad");
        assert!(search(&root, &Search::new("a(b")).is_err());
    }

    #[test]
    fn a_literal_search_takes_the_pattern_as_written() {
        let root = fixture("literal");
        std::fs::write(root.join("src/c.rs"), "let x = a(b;\n").unwrap();
        let mut wanted = Search::new("a(b");
        wanted.regexp = false;
        let found = search(&root, &wanted).unwrap();
        assert_eq!(found.hits.len(), 1, "the parentheses should be characters");
    }

    // ---- writing the answers back ---------------------------------------

    fn replacement(path: &str, line: usize, was: &str, now: &str) -> Replacement {
        Replacement {
            path: PathBuf::from(path),
            line,
            was: was.into(),
            now: now.into(),
        }
    }

    #[test]
    fn a_rewrite_replaces_the_lines_it_was_given() {
        let text = "one\ntwo\nthree\n";
        let out = rewrite(
            text,
            &[
                replacement("f", 0, "one", "ONE"),
                replacement("f", 2, "three", "THREE"),
            ],
        )
        .unwrap();
        assert_eq!(out, "ONE\ntwo\nTHREE\n");
    }

    #[test]
    fn a_file_that_changed_underneath_is_refused() {
        // The whole point of carrying `was`: the results buffer was made from
        // a file that something else may have written since.
        let text = "one\ntwo\n";
        let error = rewrite(text, &[replacement("f", 0, "ONE", "x")]).unwrap_err();
        assert!(matches!(error, GrepError::Stale { .. }), "{error}");
    }

    #[test]
    fn a_line_that_is_no_longer_there_is_refused() {
        let text = "one\n";
        let error = rewrite(text, &[replacement("f", 9, "one", "x")]).unwrap_err();
        assert!(matches!(error, GrepError::Stale { .. }), "{error}");
    }

    #[test]
    fn a_file_keeps_whether_it_ended_with_a_newline() {
        assert_eq!(
            rewrite("a\nb\n", &[replacement("f", 0, "a", "A")]).unwrap(),
            "A\nb\n"
        );
        assert_eq!(
            rewrite("a\nb", &[replacement("f", 0, "a", "A")]).unwrap(),
            "A\nb"
        );
    }

    #[test]
    fn applying_writes_every_file_it_was_given() {
        let root = fixture("apply");
        let a = root.join("src/a.rs");
        let b = root.join("src/b.rs");
        let applied = apply(&[
            Replacement {
                path: a.clone(),
                line: 0,
                was: "fn alpha() {}".into(),
                now: "fn renamed() {}".into(),
            },
            Replacement {
                path: b.clone(),
                line: 0,
                was: "// alpha again".into(),
                now: "// renamed again".into(),
            },
        ])
        .unwrap();
        assert_eq!(applied, Applied { files: 2, lines: 2 });
        assert!(
            std::fs::read_to_string(&a)
                .unwrap()
                .contains("fn renamed()")
        );
        assert!(
            std::fs::read_to_string(&b)
                .unwrap()
                .contains("// renamed again")
        );
    }

    #[test]
    fn a_refused_file_is_not_half_written() {
        let root = fixture("atomic");
        let a = root.join("src/a.rs");
        let before = std::fs::read_to_string(&a).unwrap();
        let error = apply(&[Replacement {
            path: a.clone(),
            line: 0,
            was: "something else entirely".into(),
            now: "x".into(),
        }])
        .unwrap_err();
        assert!(matches!(error, GrepError::Stale { .. }), "{error}");
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            before,
            "the file was written despite the refusal"
        );
    }
}
