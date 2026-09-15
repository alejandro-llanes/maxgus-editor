//! Git status decoration.
//!
//! Statuses come from one `git status --porcelain` invocation per refresh,
//! parsed into a path-to-status map. Running git is optional: outside a
//! repository, or without git installed, the tree simply shows no indicators.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The status treemacs shows next to a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GitStatus {
    /// Both sides modified, or otherwise unmerged.
    Conflict,
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Ignored,
}

impl GitStatus {
    /// The face the indicator is drawn in.
    pub fn face(self) -> &'static str {
        match self {
            GitStatus::Conflict => "tree-git-conflict",
            GitStatus::Added => "tree-git-added",
            GitStatus::Modified | GitStatus::Renamed => "tree-git-modified",
            GitStatus::Deleted => "tree-git-deleted",
            GitStatus::Untracked => "tree-git-untracked",
            GitStatus::Ignored => "tree-git-ignored",
        }
    }

    /// The single-character indicator.
    pub fn indicator(self) -> char {
        match self {
            GitStatus::Conflict => '!',
            GitStatus::Added => '+',
            GitStatus::Modified => 'M',
            GitStatus::Deleted => '-',
            GitStatus::Renamed => 'R',
            GitStatus::Untracked => '?',
            GitStatus::Ignored => '~',
        }
    }

    /// Parses one porcelain-v1 status field, the two characters that begin
    /// each line of `git status --porcelain`.
    pub fn from_porcelain(code: &str) -> Option<GitStatus> {
        let mut chars = code.chars();
        let index = chars.next()?;
        let worktree = chars.next().unwrap_or(' ');
        Some(match (index, worktree) {
            ('?', '?') => GitStatus::Untracked,
            ('!', '!') => GitStatus::Ignored,
            // Any `U`, or the AA/DD pairs, mean an unmerged path.
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => GitStatus::Conflict,
            ('R', _) | (_, 'R') => GitStatus::Renamed,
            ('A', _) => GitStatus::Added,
            ('D', _) | (_, 'D') => GitStatus::Deleted,
            ('M', _) | (_, 'M') | ('T', _) | (_, 'T') => GitStatus::Modified,
            _ => return None,
        })
    }

    /// The status a directory shows, given its descendants': the most
    /// attention-worthy one wins.
    pub fn rollup(statuses: impl IntoIterator<Item = GitStatus>) -> Option<GitStatus> {
        // `Ord` is declared in priority order, so the minimum is the winner.
        statuses.into_iter().min()
    }
}

/// Parses the output of `git status --porcelain -z`-style plain output into a
/// map from repository-relative path to status.
///
/// `root` is the repository root, so paths come out absolute.
pub fn parse_porcelain(root: &Path, output: &str) -> HashMap<PathBuf, GitStatus> {
    let mut map = HashMap::new();
    for line in output.lines() {
        // Format: `XY <path>`, with renames as `XY <old> -> <new>`.
        if line.len() < 4 {
            continue;
        }
        let (code, rest) = line.split_at(2);
        let Some(status) = GitStatus::from_porcelain(code) else {
            continue;
        };
        let path = rest.trim_start();
        // For a rename, decorate the destination.
        let path = path.rsplit(" -> ").next().unwrap_or(path);
        let path = path.trim_matches('"');
        if path.is_empty() {
            continue;
        }
        map.insert(root.join(path), status);
    }
    map
}

/// Parses `git status --porcelain -z`, the form [`git_status`] asks for.
///
/// Entries end in a NUL rather than a newline, and their paths are the bytes
/// on disk: never quoted, never escaped. The plain form writes `ñandú.txt` as
/// `"\303\261and\303\272.txt"`, which names nothing, so a file with an accent
/// in its name never got its mark.
pub fn parse_porcelain_z(root: &Path, output: &[u8]) -> HashMap<PathBuf, GitStatus> {
    let mut map = HashMap::new();
    let mut entries = output.split(|byte| *byte == 0);
    while let Some(entry) = entries.next() {
        if entry.len() < 4 || entry[2] != b' ' {
            continue;
        }
        // A rename or a copy is followed by the path it came from, which is
        // no longer anything on disk to decorate.
        if matches!(entry[0], b'R' | b'C') {
            entries.next();
        }
        let Some(status) = GitStatus::from_porcelain(&String::from_utf8_lossy(&entry[..2])) else {
            continue;
        };
        // An untracked or ignored directory is reported whole, with a slash.
        let path = entry[3..].strip_suffix(b"/").unwrap_or(&entry[3..]);
        if path.is_empty() {
            continue;
        }
        map.insert(root.join(path_from_bytes(path)), status);
    }
    map
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

/// Runs `git status` in `root` and returns the decorated paths.
///
/// Returns an empty map when `root` is not a repository or git is unavailable:
/// the tree is still perfectly usable without indicators.
pub async fn git_status(root: &Path, include_ignored: bool) -> HashMap<PathBuf, GitStatus> {
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .arg("status")
        .arg("--porcelain")
        .arg("-z")
        .arg("--no-renames")
        .arg("--untracked-files=normal");
    if include_ignored {
        // `matching` reports an ignored directory once rather than every
        // file inside it, which is what keeps a `node_modules` affordable.
        command.arg("--ignored=matching");
    }
    // Never let a hung git block the editor's event loop.
    command.kill_on_drop(true);
    let Ok(output) = command.output().await else {
        return HashMap::new();
    };
    if !output.status.success() {
        return HashMap::new();
    }
    // Resolve the true repository root so paths line up.
    let repo_root = repository_root(root)
        .await
        .unwrap_or_else(|| root.to_path_buf());
    parse_porcelain_z(&repo_root, &output.stdout)
}

/// The branch `path` is on, if it is in a repository at all.
///
/// A detached head has no branch name; git says `HEAD` there, which is not
/// one, so it is reported as none rather than shown as a branch called HEAD.
pub async fn branch(path: &Path) -> Option<String> {
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD");
    // Never let a hung git block the editor.
    command.kill_on_drop(true);
    let output = command.output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let name = text.trim();
    (!name.is_empty() && name != "HEAD").then(|| name.to_string())
}

/// The repository root containing `path`, if any.
pub async fn repository_root(path: &Path) -> Option<PathBuf> {
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .kill_on_drop(true);
    let output = command.output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_codes_map_to_statuses() {
        assert_eq!(GitStatus::from_porcelain("??"), Some(GitStatus::Untracked));
        assert_eq!(GitStatus::from_porcelain("!!"), Some(GitStatus::Ignored));
        assert_eq!(GitStatus::from_porcelain(" M"), Some(GitStatus::Modified));
        assert_eq!(GitStatus::from_porcelain("M "), Some(GitStatus::Modified));
        assert_eq!(GitStatus::from_porcelain("A "), Some(GitStatus::Added));
        assert_eq!(GitStatus::from_porcelain(" D"), Some(GitStatus::Deleted));
        assert_eq!(GitStatus::from_porcelain("R "), Some(GitStatus::Renamed));
        assert_eq!(
            GitStatus::from_porcelain("  "),
            None,
            "a clean file has no status"
        );
    }

    #[test]
    fn unmerged_codes_are_conflicts() {
        for code in ["UU", "AA", "DD", "AU", "UD"] {
            assert_eq!(
                GitStatus::from_porcelain(code),
                Some(GitStatus::Conflict),
                "`{code}` should be a conflict"
            );
        }
    }

    #[test]
    fn each_status_has_a_face_and_an_indicator() {
        let all = [
            GitStatus::Conflict,
            GitStatus::Added,
            GitStatus::Modified,
            GitStatus::Deleted,
            GitStatus::Renamed,
            GitStatus::Untracked,
            GitStatus::Ignored,
        ];
        let mut indicators: Vec<char> = all.iter().map(|s| s.indicator()).collect();
        let before = indicators.len();
        indicators.sort_unstable();
        indicators.dedup();
        assert_eq!(
            indicators.len(),
            before,
            "indicators must be distinguishable"
        );
        for s in all {
            assert!(s.face().starts_with("tree-git-"));
        }
    }

    #[test]
    fn a_directory_rolls_up_the_most_important_status() {
        assert_eq!(
            GitStatus::rollup([
                GitStatus::Ignored,
                GitStatus::Modified,
                GitStatus::Untracked
            ]),
            Some(GitStatus::Modified)
        );
        assert_eq!(
            GitStatus::rollup([GitStatus::Modified, GitStatus::Conflict]),
            Some(GitStatus::Conflict)
        );
        assert_eq!(GitStatus::rollup([]), None);
    }

    #[test]
    fn porcelain_output_parses_into_absolute_paths() {
        let root = Path::new("/repo");
        let out = " M src/main.rs\n?? notes.txt\nA  src/new.rs\n";
        let map = parse_porcelain(root, out);
        assert_eq!(
            map.get(Path::new("/repo/src/main.rs")),
            Some(&GitStatus::Modified)
        );
        assert_eq!(
            map.get(Path::new("/repo/notes.txt")),
            Some(&GitStatus::Untracked)
        );
        assert_eq!(
            map.get(Path::new("/repo/src/new.rs")),
            Some(&GitStatus::Added)
        );
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn renames_decorate_the_destination() {
        let map = parse_porcelain(Path::new("/repo"), "R  old.rs -> new.rs\n");
        assert_eq!(
            map.get(Path::new("/repo/new.rs")),
            Some(&GitStatus::Renamed)
        );
        assert!(!map.contains_key(Path::new("/repo/old.rs")));
    }

    #[test]
    fn quoted_paths_are_unquoted() {
        let map = parse_porcelain(Path::new("/repo"), "?? \"with space.txt\"\n");
        assert_eq!(
            map.get(Path::new("/repo/with space.txt")),
            Some(&GitStatus::Untracked)
        );
    }

    #[test]
    fn nul_separated_output_keeps_names_as_they_are_on_disk() {
        let out = b" M src/main.rs\0?? \xc3\xb1and\xc3\xba.txt\0?? new dir/\0!! target/\0";
        let map = parse_porcelain_z(Path::new("/repo"), out);
        assert_eq!(
            map.get(Path::new("/repo/src/main.rs")),
            Some(&GitStatus::Modified)
        );
        assert_eq!(
            map.get(Path::new("/repo/ñandú.txt")),
            Some(&GitStatus::Untracked),
            "an accented name is not quoted in this form, so it must be found"
        );
        assert_eq!(
            map.get(Path::new("/repo/new dir")),
            Some(&GitStatus::Untracked),
            "a directory's slash is not part of its name"
        );
        assert_eq!(
            map.get(Path::new("/repo/target")),
            Some(&GitStatus::Ignored)
        );
        assert_eq!(map.len(), 4);
    }

    #[test]
    fn a_rename_in_the_nul_form_skips_the_path_it_came_from() {
        let map = parse_porcelain_z(Path::new("/repo"), b"R  new.rs\0old.rs\0 M kept.rs\0");
        assert_eq!(
            map.get(Path::new("/repo/new.rs")),
            Some(&GitStatus::Renamed)
        );
        assert!(!map.contains_key(Path::new("/repo/old.rs")));
        assert_eq!(
            map.get(Path::new("/repo/kept.rs")),
            Some(&GitStatus::Modified)
        );
    }

    #[tokio::test]
    async fn a_real_repository_reports_accents_and_ignored_files() {
        let dir = std::env::temp_dir().join("maxgus-tree-git-real");
        tokio::fs::remove_dir_all(&dir).await.ok();
        tokio::fs::create_dir_all(dir.join("build")).await.unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
        };
        if git(&["init", "-q"]).is_err() {
            // No git on this machine: nothing to check against.
            return;
        }
        tokio::fs::write(dir.join(".gitignore"), "build/\n")
            .await
            .unwrap();
        tokio::fs::write(dir.join("ñandú.txt"), "hola")
            .await
            .unwrap();
        tokio::fs::write(dir.join("build/out.o"), "").await.unwrap();

        let map = git_status(&dir, true).await;
        let root = repository_root(&dir).await.unwrap();
        assert_eq!(
            map.get(&root.join("ñandú.txt")),
            Some(&GitStatus::Untracked)
        );
        assert_eq!(map.get(&root.join("build")), Some(&GitStatus::Ignored));
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let map = parse_porcelain(Path::new("/repo"), "x\n\n   \nZZ weird.rs\n M ok.rs\n");
        assert_eq!(map.len(), 1);
        assert!(map.contains_key(Path::new("/repo/ok.rs")));
    }

    #[tokio::test]
    async fn a_directory_outside_a_repository_yields_no_statuses() {
        let dir = std::env::temp_dir().join("maxgus-tree-git-test-not-a-repo");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let map = git_status(&dir, false).await;
        assert!(map.is_empty());
        assert!(repository_root(&dir).await.is_none());
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn a_missing_directory_is_handled_gracefully() {
        let missing = Path::new("/nonexistent-path-for-maxgus-tests");
        assert!(git_status(missing, false).await.is_empty());
        assert!(repository_root(missing).await.is_none());
    }
}
