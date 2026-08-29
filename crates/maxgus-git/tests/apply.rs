//! Patches this crate writes, applied by real git.
//!
//! Everything else about the diff code is tested against text. These tests
//! build a repository, make a change, read git's own diff, write a patch for
//! one hunk of it and hand it back to `git apply`. That is the only check that
//! the patch is *valid* rather than merely plausible — and staging the wrong
//! hunk, or a patch git rejects, is how an editor loses somebody's work.

use std::path::Path;
use std::process::{Command, Stdio};

/// A throwaway repository.
struct Repo(std::path::PathBuf);

impl Repo {
    fn new(tag: &str) -> Repo {
        let directory = std::env::temp_dir().join(format!("maxgus-git-{tag}"));
        std::fs::remove_dir_all(&directory).ok();
        std::fs::create_dir_all(&directory).expect("a directory");
        let repo = Repo(directory);
        repo.git(&["init", "--initial-branch=main"]);
        repo.git(&["config", "user.email", "test@example.invalid"]);
        repo.git(&["config", "user.name", "Test"]);
        repo
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap_or_else(|e| panic!("running git {args:?}: {e}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Feeds a patch to `git apply`, returning what git said if it refused.
    fn apply(&self, patch: &str, args: &[&str]) -> Result<(), String> {
        let mut child = Command::new("git")
            .arg("apply")
            .args(args)
            .arg("-")
            .current_dir(&self.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("git apply");
        use std::io::Write as _;
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(patch.as_bytes())
            .expect("write");
        let output = child.wait_with_output().expect("wait");
        if output.status.success() {
            return Ok(());
        }
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }

    fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.0.join(name), contents).expect("writing a file");
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// Twenty numbered lines. Two edits are only two hunks when there are more
/// than six lines between them — git's three lines of context either side
/// would otherwise run together into one.
fn numbered() -> String {
    (1..=20).map(|n| format!("line {n}\n")).collect()
}

#[test]
fn one_hunk_of_two_is_staged_and_the_other_is_left_alone() {
    // The signature magit operation. Staging the whole file instead would be
    // the obvious wrong answer, and the diff afterwards is what tells them
    // apart.
    let repo = Repo::new("stage-hunk");
    repo.write("file.txt", &numbered());
    repo.git(&["add", "."]);
    repo.git(&["commit", "-m", "first"]);

    let mut edited = numbered();
    edited = edited
        .replace("line 2\n", "LINE TWO\n")
        .replace("line 18\n", "LINE EIGHTEEN\n");
    repo.write("file.txt", &edited);

    let diff = repo.git(&["diff"]);
    let files = maxgus_git::diff::parse(&diff);
    assert_eq!(files.len(), 1);
    assert_eq!(
        files[0].hunks.len(),
        2,
        "the edits should be two hunks:\n{diff}"
    );

    // Stage the second one only.
    let patch = maxgus_git::diff::hunk_patch(&files[0], &files[0].hunks[1]);
    repo.apply(&patch, &["--cached"])
        .unwrap_or_else(|e| panic!("git refused the patch: {e}\n{patch}"));

    let staged = repo.git(&["diff", "--cached"]);
    assert!(
        staged.contains("LINE EIGHTEEN"),
        "the chosen hunk was not staged:\n{staged}"
    );
    assert!(
        !staged.contains("LINE TWO"),
        "the other hunk was staged too:\n{staged}"
    );

    let unstaged = repo.git(&["diff"]);
    assert!(
        unstaged.contains("LINE TWO"),
        "the other hunk vanished:\n{unstaged}"
    );
    assert!(
        !unstaged.contains("LINE EIGHTEEN"),
        "the staged hunk is still unstaged:\n{unstaged}"
    );
}

#[test]
fn a_staged_hunk_is_unstaged_by_the_same_patch_reversed() {
    // One patch, three uses: staged with `--cached`, unstaged with
    // `--cached --reverse`, discarded with `--reverse` alone.
    let repo = Repo::new("unstage-hunk");
    repo.write("file.txt", &numbered());
    repo.git(&["add", "."]);
    repo.git(&["commit", "-m", "first"]);
    repo.write("file.txt", &numbered().replace("line 5\n", "LINE FIVE\n"));
    repo.git(&["add", "."]);

    let staged = repo.git(&["diff", "--cached"]);
    let files = maxgus_git::diff::parse(&staged);
    let patch = maxgus_git::diff::hunk_patch(&files[0], &files[0].hunks[0]);

    repo.apply(&patch, &["--cached", "--reverse"])
        .unwrap_or_else(|e| panic!("git refused the reversed patch: {e}\n{patch}"));
    assert!(
        repo.git(&["diff", "--cached"]).is_empty(),
        "the hunk is still staged"
    );
    // And the change itself is still in the working tree, merely unstaged.
    assert!(
        repo.git(&["diff"]).contains("LINE FIVE"),
        "unstaging threw the change away"
    );
}

#[test]
fn a_whole_new_file_is_staged_from_its_patch() {
    let repo = Repo::new("stage-new");
    repo.write("first.txt", "seed\n");
    repo.git(&["add", "."]);
    repo.git(&["commit", "-m", "first"]);
    repo.write("added.txt", "one\ntwo\n");
    repo.git(&["add", "-N", "added.txt"]);

    let diff = repo.git(&["diff"]);
    let files = maxgus_git::diff::parse(&diff);
    let file = files
        .iter()
        .find(|f| f.path == "added.txt")
        .expect("the new file");
    let patch = maxgus_git::diff::file_patch(file);

    repo.apply(&patch, &["--cached"])
        .unwrap_or_else(|e| panic!("git refused a new file's patch: {e}\n{patch}"));
    assert!(repo.git(&["diff", "--cached"]).contains("added.txt"));
}

#[test]
fn a_file_with_no_trailing_newline_is_staged_without_gaining_one() {
    // The `\ No newline at end of file` marker. Dropping it makes the patch
    // add a byte nobody asked for, and git usually refuses it outright.
    let repo = Repo::new("no-newline");
    repo.write("file.txt", "one\ntwo");
    repo.git(&["add", "."]);
    repo.git(&["commit", "-m", "first"]);
    repo.write("file.txt", "one\nTWO");

    let diff = repo.git(&["diff"]);
    assert!(
        diff.contains("\\ No newline"),
        "the fixture is not testing what it means to"
    );
    let files = maxgus_git::diff::parse(&diff);
    let patch = maxgus_git::diff::hunk_patch(&files[0], &files[0].hunks[0]);
    repo.apply(&patch, &["--cached"])
        .unwrap_or_else(|e| panic!("git refused it: {e}\n{patch}"));

    let staged = repo.git(&["show", ":file.txt"]);
    assert_eq!(staged, "one\nTWO", "a newline was added on the way through");
}

#[test]
fn a_rename_is_read_back_with_both_of_its_names() {
    let repo = Repo::new("rename");
    repo.write("old.txt", &numbered());
    repo.git(&["add", "."]);
    repo.git(&["commit", "-m", "first"]);
    repo.git(&["mv", "old.txt", "new.txt"]);

    let diff = repo.git(&["diff", "--cached"]);
    let files = maxgus_git::diff::parse(&diff);
    let file = &files[0];
    assert_eq!(file.path, "new.txt");
    assert_eq!(file.old_path.as_deref(), Some("old.txt"));
}

#[test]
fn the_status_parser_reads_what_git_actually_writes() {
    // The unit tests feed it records by hand. This one feeds it git.
    let repo = Repo::new("status");
    repo.write("committed.txt", "a\n");
    repo.git(&["add", "."]);
    repo.git(&["commit", "-m", "first"]);
    repo.write("committed.txt", "b\n");
    repo.write("untracked.txt", "c\n");
    repo.write("staged.txt", "d\n");
    repo.git(&["add", "staged.txt"]);

    let output = Command::new("git")
        .args(["status", "--porcelain=v2", "-z", "--branch"])
        .current_dir(repo.path())
        .output()
        .expect("git status");
    let status = maxgus_git::status::parse(&output.stdout);

    assert_eq!(status.branch.as_deref(), Some("main"));
    assert!(status.head.is_some());
    assert!(!status.is_clean());
    let names = |mut it: Vec<String>| {
        it.sort();
        it
    };
    assert_eq!(
        names(
            status
                .staged()
                .map(|e| e.path.to_string_lossy().into_owned())
                .collect()
        ),
        ["staged.txt"]
    );
    assert_eq!(
        names(
            status
                .unstaged()
                .map(|e| e.path.to_string_lossy().into_owned())
                .collect()
        ),
        ["committed.txt"]
    );
    assert_eq!(
        names(
            status
                .untracked()
                .map(|e| e.path.to_string_lossy().into_owned())
                .collect()
        ),
        ["untracked.txt"]
    );
}
