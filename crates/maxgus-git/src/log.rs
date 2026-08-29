//! Reading `git log` and `git stash list`.
//!
//! Both are asked for with an explicit format built from separators that
//! cannot occur in a commit message — the ASCII unit and record separators —
//! rather than parsed out of git's readable output. A subject line may contain
//! anything at all, including whatever delimiter looked safe.

/// The format string to pass `git log --format=`.
pub const LOG_FORMAT: &str = "%H%x1f%h%x1f%an%x1f%ar%x1f%D%x1f%s%x1e";
/// The format string to pass `git stash list --format=`.
pub const STASH_FORMAT: &str = "%gd%x1f%s%x1e";

/// One commit, as the log view shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub author: String,
    /// Relative, as git writes it: "3 days ago".
    pub when: String,
    /// Branch and tag names pointing here.
    pub refs: Vec<String>,
    pub subject: String,
}

/// One stash entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stash {
    /// `stash@{0}`, which is also how it is named to git.
    pub name: String,
    pub subject: String,
}

/// Parses `git log --format=LOG_FORMAT`.
pub fn parse_log(output: &str) -> Vec<Commit> {
    records(output)
        .filter_map(|record| {
            let mut fields = record.split('\u{1f}');
            let hash = fields.next()?.to_string();
            if hash.is_empty() {
                return None;
            }
            Some(Commit {
                hash,
                short: fields.next().unwrap_or_default().to_string(),
                author: fields.next().unwrap_or_default().to_string(),
                when: fields.next().unwrap_or_default().to_string(),
                refs: fields
                    .next()
                    .unwrap_or_default()
                    .split(", ")
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .collect(),
                subject: fields.next().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// Parses `git stash list --format=STASH_FORMAT`.
pub fn parse_stashes(output: &str) -> Vec<Stash> {
    records(output)
        .filter_map(|record| {
            let (name, subject) = record.split_once('\u{1f}')?;
            (!name.is_empty()).then(|| Stash {
                name: name.to_string(),
                subject: subject.to_string(),
            })
        })
        .collect()
}

/// Splits on the record separator, dropping the newline git puts after each.
fn records(output: &str) -> impl Iterator<Item = &str> {
    output
        .split('\u{1e}')
        .map(|record| record.trim_start_matches(['\n', '\r']))
        .filter(|record| !record.is_empty())
}

/// What a reference is, which its full name says and its short name does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Local,
    Remote,
    Tag,
}

/// A reference, by the short name a person uses and the kind it really is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub name: String,
    pub kind: RefKind,
}

/// Parses `git for-each-ref --format=%(refname)`.
///
/// Full names, not short ones: a local branch may be called `feature/x`, and
/// deciding what a reference is by looking for a slash puts it among the
/// remotes.
pub fn parse_refs(output: &str) -> Vec<Reference> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let (kind, name) = match line {
                _ if line.starts_with("refs/heads/") => {
                    (RefKind::Local, &line["refs/heads/".len()..])
                }
                _ if line.starts_with("refs/remotes/") => {
                    (RefKind::Remote, &line["refs/remotes/".len()..])
                }
                _ if line.starts_with("refs/tags/") => (RefKind::Tag, &line["refs/tags/".len()..]),
                _ => return None,
            };
            // `origin/HEAD` is a symbolic pointer, not somewhere to go.
            (!name.ends_with("/HEAD")).then(|| Reference {
                name: name.to_string(),
                kind,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(fields: &[&str]) -> String {
        format!("{}\u{1e}\n", fields.join("\u{1f}"))
    }

    #[test]
    fn a_commit_is_read_field_by_field() {
        let output = record(&[
            "5958f5e13418d8b5",
            "5958f5e",
            "Alejandro Llanes",
            "3 days ago",
            "HEAD -> main, origin/main, tag: v0.1.0",
            "Count the tests correctly",
        ]);
        let commits = parse_log(&output);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].short, "5958f5e");
        assert_eq!(commits[0].author, "Alejandro Llanes");
        assert_eq!(commits[0].when, "3 days ago");
        assert_eq!(
            commits[0].refs,
            ["HEAD -> main", "origin/main", "tag: v0.1.0"]
        );
        assert_eq!(commits[0].subject, "Count the tests correctly");
    }

    #[test]
    fn a_subject_containing_anything_at_all_survives() {
        // The reason for the unit separator: a commit message may contain
        // every delimiter that looked safe, including newlines and tabs.
        let awkward = "fix: handle a | b, and \"quotes\"\ttoo";
        let output = record(&["h", "s", "a", "now", "", awkward]);
        assert_eq!(parse_log(&output)[0].subject, awkward);
    }

    #[test]
    fn a_commit_with_no_refs_has_none_rather_than_one_empty_one() {
        let output = record(&["h", "s", "a", "now", "", "subject"]);
        assert!(parse_log(&output)[0].refs.is_empty());
    }

    #[test]
    fn several_commits_come_back_in_order() {
        let output = format!(
            "{}{}",
            record(&["h1", "s1", "a", "now", "", "newest"]),
            record(&["h2", "s2", "a", "then", "", "older"])
        );
        let commits = parse_log(&output);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].subject, "newest");
        assert_eq!(commits[1].subject, "older");
    }

    #[test]
    fn stashes_are_read_with_the_names_git_knows_them_by() {
        // The name is what a pop or a drop is addressed to, so it has to be
        // git's own rather than a position in the list.
        let output = format!(
            "{}{}",
            record(&["stash@{0}", "WIP on main: 5958f5e Count the tests"]),
            record(&["stash@{1}", "On main: an experiment"])
        );
        let stashes = parse_stashes(&output);
        assert_eq!(stashes.len(), 2);
        assert_eq!(stashes[0].name, "stash@{0}");
        assert_eq!(stashes[1].subject, "On main: an experiment");
    }

    #[test]
    fn empty_output_is_no_commits_rather_than_one_blank_one() {
        assert!(parse_log("").is_empty());
        assert!(parse_log("\n").is_empty());
        assert!(parse_stashes("").is_empty());
        assert!(parse_refs("\n\n").is_empty());
    }

    #[test]
    fn a_reference_is_classified_by_its_full_name() {
        // `feature/x` is a local branch with a slash in it. Deciding by the
        // slash would file it under remotes.
        let refs = parse_refs(
            "refs/heads/main\nrefs/heads/feature/x\nrefs/remotes/origin/main\nrefs/tags/v1.0\n",
        );
        assert_eq!(refs.len(), 4);
        assert_eq!(
            refs[1],
            Reference {
                name: "feature/x".into(),
                kind: RefKind::Local
            }
        );
        assert_eq!(
            refs[2],
            Reference {
                name: "origin/main".into(),
                kind: RefKind::Remote
            }
        );
        assert_eq!(
            refs[3],
            Reference {
                name: "v1.0".into(),
                kind: RefKind::Tag
            }
        );
    }

    #[test]
    fn the_remotes_symbolic_head_is_not_somewhere_to_go() {
        let refs = parse_refs("refs/remotes/origin/HEAD\nrefs/remotes/origin/main\n");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "origin/main");
    }

    #[test]
    fn anything_that_is_not_a_reference_is_dropped() {
        assert!(parse_refs("refs/stash\nnonsense\n\n").is_empty());
    }
}
