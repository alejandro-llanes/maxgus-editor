//! The git status view, and everything reachable from it.
//!
//! Magit's arrangement, for magit's reason: a commit is assembled by looking
//! at the change rather than by remembering it, so the whole state of the
//! repository is one buffer that folds, and every key acts on whatever row
//! point is on. `s` stages a file when point is on a file, a hunk when point
//! is on a hunk, and everything when point is on the section heading.
//!
//! Staging a hunk is the piece that carries risk: it works by writing a patch
//! containing that hunk alone and handing it to `git apply --cached`. The
//! patch is built in `maxgus-git`, where it is tested against real git.

use crate::{
    MinibufferKind, Result, command,
    command::{Args, Registry},
    editor::Editor,
    git::{Row, Section},
    task::{GitAction, Task},
};
use std::path::PathBuf;

/// The buffer the status view is drawn into. Magit's own name, because a
/// person who knows magit should recognise it.
pub const STATUS_BUFFER_NAME: &str = "magit: status";
/// The buffer a commit message is written in.
pub const COMMIT_BUFFER_NAME: &str = "COMMIT_EDITMSG";
/// The buffers the other views are drawn into. Magit's own names, so that
/// switching between them with `C-x b` reads as one family.
pub const LOG_BUFFER_NAME: &str = "magit: log";
pub const DIFF_BUFFER_NAME: &str = "magit: diff";
pub const REVISION_BUFFER_NAME: &str = "magit: revision";
pub const REFS_BUFFER_NAME: &str = "magit: refs";
pub const PROCESS_BUFFER_NAME: &str = "magit: process";

/// Every buffer the status keymap applies in.
pub const MAGIT_BUFFERS: &[&str] = &[
    STATUS_BUFFER_NAME,
    LOG_BUFFER_NAME,
    DIFF_BUFFER_NAME,
    REVISION_BUFFER_NAME,
    REFS_BUFFER_NAME,
    PROCESS_BUFFER_NAME,
];

pub const GIT_MODE: &str = "magit-mode";
pub const COMMIT_MODE: &str = "git-commit-mode";

pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!("magit-status", "Show the state of the repository.", status),
        // What a person types. In Emacs `magit` is an alias for
        // `magit-status`, and somebody reaching for it should not have to
        // discover that this one spells it differently.
        command!("magit", "Show the state of the repository.", status),
        command!(
            "magit-refresh",
            "Read the repository again.",
            refresh,
            non_interactive
        ),
        command!(
            "magit-quit",
            "Close this magit view, killing it. With a prefix, bury it instead.",
            quit,
            non_interactive
        ),
        command!(
            "magit-toggle",
            "Fold or unfold whatever is here.",
            toggle,
            non_interactive
        ),
        command!(
            "magit-toggle-all",
            "Fold everything, or unfold it.",
            toggle_all,
            non_interactive
        ),
        command!(
            "magit-next-section",
            "Move to the next section.",
            next_section,
            non_interactive
        ),
        command!(
            "magit-previous-section",
            "Move to the previous section.",
            previous_section,
            non_interactive
        ),
        command!(
            "magit-next-sibling",
            "Move to the next section at this level.",
            next_sibling,
            non_interactive
        ),
        command!(
            "magit-previous-sibling",
            "Move to the previous section at this level.",
            previous_sibling,
            non_interactive
        ),
        command!(
            "magit-parent-section",
            "Move to the section this is inside.",
            parent_section,
            non_interactive
        ),
        command!(
            "magit-visit",
            "Open the file this line is about.",
            visit,
            non_interactive
        ),
        command!(
            "magit-stage",
            "Stage whatever is here.",
            stage,
            non_interactive
        ),
        command!(
            "magit-stage-all",
            "Stage every change.",
            stage_all,
            non_interactive
        ),
        command!(
            "magit-unstage",
            "Unstage whatever is here.",
            unstage,
            non_interactive
        ),
        command!(
            "magit-unstage-all",
            "Unstage everything.",
            unstage_all,
            non_interactive
        ),
        command!(
            "magit-discard",
            "Throw away whatever is here.",
            discard,
            non_interactive
        ),
        command!("magit-commit", "Write a commit message.", commit),
        command!(
            "magit-commit-amend",
            "Add this to the last commit.",
            commit_amend
        ),
        command!(
            "magit-commit-extend",
            "Add this to the last commit, keeping its message.",
            commit_extend
        ),
        command!(
            "magit-commit-finish",
            "Make the commit.",
            commit_finish,
            non_interactive
        ),
        command!(
            "magit-commit-cancel",
            "Abandon the commit message.",
            commit_cancel,
            non_interactive
        ),
        command!("magit-push", "Push to the upstream.", push),
        command!(
            "magit-push-force",
            "Push over the upstream, if nobody else has.",
            push_force
        ),
        command!("magit-pull", "Pull from the upstream.", pull),
        command!("magit-fetch", "Fetch every remote.", fetch),
        command!("magit-checkout", "Check out a branch.", checkout),
        command!(
            "magit-branch-create",
            "Create a branch and check it out.",
            branch_create
        ),
        command!("magit-merge", "Merge a branch into this one.", merge),
        command!("magit-stash", "Stash the working tree.", stash),
        command!(
            "magit-stash-pop",
            "Restore the stash here and drop it.",
            stash_pop,
            non_interactive
        ),
        command!(
            "magit-stash-apply",
            "Restore the stash here, keeping it.",
            stash_apply,
            non_interactive
        ),
        command!(
            "magit-stash-drop",
            "Throw the stash here away.",
            stash_drop,
            non_interactive
        ),
        command!(
            "magit-help",
            "Describe the status view's keymap.",
            help,
            non_interactive
        ),
        // ---- the menus ----
        command!(
            "magit-dispatch",
            "Show what git can do here.",
            menu_dispatch
        ),
        command!("magit-commit-menu", "Committing.", menu_commit),
        command!("magit-diff-menu", "Diffing.", menu_diff),
        command!("magit-log-menu", "Logging.", menu_log),
        command!("magit-branch-menu", "Branching.", menu_branch),
        command!("magit-merge-menu", "Merging.", menu_merge),
        command!("magit-rebase-menu", "Rebasing.", menu_rebase),
        command!("magit-reset-menu", "Resetting.", menu_reset),
        command!("magit-stash-menu", "Stashing.", menu_stash),
        command!("magit-tag-menu", "Tagging.", menu_tag),
        command!("magit-push-menu", "Pushing.", menu_push),
        command!("magit-pull-menu", "Pulling.", menu_pull),
        command!("magit-fetch-menu", "Fetching.", menu_fetch),
        command!("magit-remote-menu", "Remotes.", menu_remote),
        command!(
            "magit-cherry-pick-menu",
            "Cherry-picking.",
            menu_cherry_pick
        ),
        command!("magit-revert-menu", "Reverting.", menu_revert),
        // ---- the other views ----
        command!(
            "magit-show-commit",
            "Show the commit here in full.",
            show_commit,
            non_interactive
        ),
        command!("magit-show-refs", "List the branches and tags.", show_refs),
        command!(
            "magit-process-buffer",
            "Show what git has been asked to do.",
            process_buffer
        ),
        command!("magit-log-current", "Log the current branch.", log_current),
        command!("magit-log-head", "Log from HEAD.", log_head),
        command!("magit-log-other", "Log another branch.", log_other),
        command!("magit-log-file", "Log the file here.", log_file),
        command!(
            "magit-diff-unstaged",
            "Diff what is not staged.",
            diff_unstaged
        ),
        command!("magit-diff-staged", "Diff what is staged.", diff_staged),
        command!(
            "magit-diff-worktree",
            "Diff the working tree against HEAD.",
            diff_worktree
        ),
        command!("magit-diff-range", "Diff a range of commits.", diff_range),
        // ---- committing ----
        command!(
            "magit-commit-reword",
            "Change the last commit's message.",
            commit_reword
        ),
        command!(
            "magit-commit-fixup",
            "Make a fixup commit for the commit here.",
            commit_fixup
        ),
        // ---- branches ----
        command!(
            "magit-branch-new",
            "Create a branch without checking it out.",
            branch_new
        ),
        command!("magit-branch-delete", "Delete a branch.", branch_delete),
        command!("magit-branch-rename", "Rename a branch.", branch_rename),
        // ---- merging and rebasing ----
        command!(
            "magit-merge-abort",
            "Abandon the merge in progress.",
            merge_abort
        ),
        command!(
            "magit-rebase-upstream",
            "Rebase onto the upstream.",
            rebase_upstream
        ),
        command!(
            "magit-rebase-elsewhere",
            "Rebase onto another branch.",
            rebase_elsewhere
        ),
        command!(
            "magit-rebase-continue",
            "Carry on with the rebase.",
            rebase_continue
        ),
        command!(
            "magit-rebase-skip",
            "Skip this commit and carry on.",
            rebase_skip
        ),
        command!(
            "magit-rebase-abort",
            "Abandon the rebase in progress.",
            rebase_abort
        ),
        // ---- resetting ----
        command!(
            "magit-reset-mixed",
            "Reset the index, keeping the tree.",
            reset_mixed
        ),
        command!(
            "magit-reset-soft",
            "Reset HEAD, keeping the index.",
            reset_soft
        ),
        command!(
            "magit-reset-hard",
            "Reset everything, throwing changes away.",
            reset_hard
        ),
        // ---- tags and remotes ----
        command!("magit-tag-create", "Create a tag here.", tag_create),
        command!("magit-tag-delete", "Delete a tag.", tag_delete),
        command!("magit-remote-add", "Add a remote.", remote_add),
        command!("magit-remote-remove", "Remove a remote.", remote_remove),
        // ---- transferring ----
        command!("magit-push-tags", "Push the tags.", push_tags),
        command!(
            "magit-push-elsewhere",
            "Push somewhere else.",
            push_elsewhere
        ),
        command!("magit-fetch-all", "Fetch every remote.", fetch_all),
        // ---- the sequencer ----
        command!(
            "magit-cherry-pick",
            "Cherry-pick the commit here.",
            cherry_pick
        ),
        command!("magit-revert", "Revert the commit here.", revert),
        command!(
            "magit-sequencer-continue",
            "Carry on with what is in progress.",
            sequencer_continue
        ),
        command!(
            "magit-sequencer-abort",
            "Abandon what is in progress.",
            sequencer_abort
        ),
        // ---- odds and ends ----
        command!("magit-run", "Run a git command.", run_git),
        command!("magit-gitignore", "Ignore the file here.", gitignore),
    ]);
}

// ---- the menus ----------------------------------------------------------

macro_rules! menu {
    ($name:ident, $menu:literal) => {
        fn $name(editor: &mut Editor, _: &Args) -> Result<()> {
            crate::commands::transient::open(editor, $menu)
        }
    };
}

menu!(menu_dispatch, "dispatch");
menu!(menu_commit, "commit");
menu!(menu_diff, "diff");
menu!(menu_log, "log");
menu!(menu_branch, "branch");
menu!(menu_merge, "merge");
menu!(menu_rebase, "rebase");
menu!(menu_reset, "reset");
menu!(menu_stash, "stash");
menu!(menu_tag, "tag");
menu!(menu_push, "push");
menu!(menu_pull, "pull");
menu!(menu_fetch, "fetch");
menu!(menu_remote, "remote");
menu!(menu_cherry_pick, "cherry-pick");
menu!(menu_revert, "revert");

// ---- the other views ----------------------------------------------------

/// The commit the row point is on refers to, wherever that row is.
fn revision_here(editor: &Editor) -> Result<String> {
    if let Some(target) = editor.git_list_target() {
        return Ok(target);
    }
    match editor.git_row_at_cursor() {
        Some(Row::Commit { section, commit }) => editor
            .git
            .commits(*section)
            .get(*commit)
            .map(|commit| commit.hash.clone())
            .ok_or_else(|| crate::CoreError::Message("No commit here".into())),
        _ => Err(crate::CoreError::Message("No commit here".into())),
    }
}

fn show_commit(editor: &mut Editor, _: &Args) -> Result<()> {
    let revision = revision_here(editor)?;
    editor.git_pending_view = Some(REVISION_BUFFER_NAME);
    act(editor, GitAction::Show { revision })
}

fn show_refs(editor: &mut Editor, _: &Args) -> Result<()> {
    let head = editor.git.status.branch.clone();
    let view = crate::git::ListView::from_refs(&editor.git_references, head.as_deref());
    editor.open_git_list(REFS_BUFFER_NAME, view)
}

fn process_buffer(editor: &mut Editor, _: &Args) -> Result<()> {
    let view = editor.git_process_view();
    editor.open_git_list(PROCESS_BUFFER_NAME, view)
}

/// Asks for a log, with whatever the menu had switched on.
fn log_with(editor: &mut Editor, extra: Vec<String>, title: String) -> Result<()> {
    let mut arguments = editor.transient_arguments.clone();
    // `--patch` belongs to a diff, not to the list of commits this shows.
    arguments.retain(|flag| flag != "--patch");
    arguments.push("-n".into());
    arguments.push("256".into());
    arguments.extend(extra);
    editor.git_pending_view = Some(LOG_BUFFER_NAME);
    act(editor, GitAction::Log { arguments, title })
}

fn log_current(editor: &mut Editor, _: &Args) -> Result<()> {
    let branch = editor
        .git
        .status
        .branch
        .clone()
        .unwrap_or_else(|| "HEAD".into());
    log_with(editor, vec![branch.clone()], format!("Log {branch}"))
}

fn log_head(editor: &mut Editor, _: &Args) -> Result<()> {
    log_with(editor, vec!["HEAD".into()], "Log HEAD".into())
}

fn log_other(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        let branches = editor.git_branches.clone();
        editor.prompt_for(
            "magit-log-other",
            MinibufferKind::Choice,
            "Log branch: ",
            "",
            branches,
        );
        return Ok(());
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(crate::CoreError::Message("No branch named".into()));
    }
    log_with(editor, vec![name.clone()], format!("Log {name}"))
}

fn log_file(editor: &mut Editor, _: &Args) -> Result<()> {
    let path = file_here(editor)?;
    log_with(
        editor,
        vec!["--".into(), path.clone()],
        format!("Log {path}"),
    )
}

/// Asks for a diff, with whatever the menu had switched on.
fn diff_with(editor: &mut Editor, extra: Vec<String>, title: String) -> Result<()> {
    let mut arguments = editor.transient_arguments.clone();
    arguments.extend(extra);
    editor.git_pending_view = Some(DIFF_BUFFER_NAME);
    act(editor, GitAction::Diff { arguments, title })
}

fn diff_unstaged(editor: &mut Editor, _: &Args) -> Result<()> {
    diff_with(editor, Vec::new(), "Unstaged changes".into())
}

fn diff_staged(editor: &mut Editor, _: &Args) -> Result<()> {
    diff_with(editor, vec!["--cached".into()], "Staged changes".into())
}

fn diff_worktree(editor: &mut Editor, _: &Args) -> Result<()> {
    diff_with(
        editor,
        vec!["HEAD".into()],
        "Working tree against HEAD".into(),
    )
}

fn diff_range(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(range) = args.input.clone() else {
        editor.prompt_for(
            "magit-diff-range",
            MinibufferKind::Text,
            "Diff range (like main..HEAD): ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    let range = range.trim().to_string();
    if range.is_empty() {
        return Err(crate::CoreError::Message("No range given".into()));
    }
    diff_with(editor, vec![range.clone()], format!("Diff {range}"))
}

// ---- the rest of the operations -----------------------------------------

/// The path the row point is on, wherever that row is.
fn file_here(editor: &Editor) -> Result<String> {
    match editor.git_row_at_cursor() {
        Some(Row::File { section, file }) => editor
            .git
            .paths(*section)
            .get(*file)
            .cloned()
            .ok_or_else(|| crate::CoreError::Message("No file here".into())),
        _ => Err(crate::CoreError::Message("No file here".into())),
    }
}

/// Runs git with the given arguments, describing it for the echo area.
fn run(editor: &mut Editor, arguments: &[&str], describe: &str) -> Result<()> {
    act(
        editor,
        GitAction::Run {
            arguments: arguments.iter().map(|a| a.to_string()).collect(),
            describe: describe.to_string(),
        },
    )
}

fn commit_reword(editor: &mut Editor, _: &Args) -> Result<()> {
    let subject = editor.git.head_subject.clone();
    open_commit_buffer(editor, true, subject)
}

fn commit_fixup(editor: &mut Editor, _: &Args) -> Result<()> {
    let revision = revision_here(editor)?;
    run(editor, &["commit", "--fixup", &revision], "Fixup")
}

fn branch_new(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        editor.prompt_for(
            "magit-branch-new",
            MinibufferKind::Text,
            "New branch: ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(crate::CoreError::Message("No branch named".into()));
    }
    run(editor, &["branch", &name], "Create branch")
}

fn branch_delete(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        let branches = editor.git_branches.clone();
        editor.prompt_for(
            "magit-branch-delete",
            MinibufferKind::Choice,
            "Delete branch: ",
            "",
            branches,
        );
        return Ok(());
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(crate::CoreError::Message("No branch named".into()));
    }
    // `-d` rather than `-D`: refusing to delete unmerged work is the whole
    // safety of the command, and forcing it is what `!` is for.
    run(editor, &["branch", "-d", &name], "Delete branch")
}

fn branch_rename(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        let current = editor.git.status.branch.clone().unwrap_or_default();
        editor.prompt_for(
            "magit-branch-rename",
            MinibufferKind::Text,
            "Rename this branch to: ",
            &current,
            Vec::new(),
        );
        return Ok(());
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(crate::CoreError::Message("No name given".into()));
    }
    run(editor, &["branch", "-m", &name], "Rename branch")
}

fn merge_abort(editor: &mut Editor, _: &Args) -> Result<()> {
    run(editor, &["merge", "--abort"], "Abort the merge")
}

fn rebase_upstream(editor: &mut Editor, _: &Args) -> Result<()> {
    run(
        editor,
        &["rebase", "@{upstream}"],
        "Rebase onto the upstream",
    )
}

fn rebase_elsewhere(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        let branches = editor.git_branches.clone();
        editor.prompt_for(
            "magit-rebase-elsewhere",
            MinibufferKind::Choice,
            "Rebase onto: ",
            "",
            branches,
        );
        return Ok(());
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(crate::CoreError::Message("No branch named".into()));
    }
    run(editor, &["rebase", &name], "Rebase")
}

fn rebase_continue(editor: &mut Editor, _: &Args) -> Result<()> {
    run(editor, &["rebase", "--continue"], "Continue the rebase")
}

fn rebase_skip(editor: &mut Editor, _: &Args) -> Result<()> {
    run(editor, &["rebase", "--skip"], "Skip this commit")
}

fn rebase_abort(editor: &mut Editor, _: &Args) -> Result<()> {
    run(editor, &["rebase", "--abort"], "Abort the rebase")
}

fn reset_mixed(editor: &mut Editor, args: &Args) -> Result<()> {
    reset(editor, args, "--mixed", "magit-reset-mixed", false)
}

fn reset_soft(editor: &mut Editor, args: &Args) -> Result<()> {
    reset(editor, args, "--soft", "magit-reset-soft", false)
}

fn reset_hard(editor: &mut Editor, args: &Args) -> Result<()> {
    reset(editor, args, "--hard", "magit-reset-hard", true)
}

/// A reset asks where to, and a hard one asks twice.
fn reset(
    editor: &mut Editor,
    args: &Args,
    mode: &'static str,
    command: &'static str,
    dangerous: bool,
) -> Result<()> {
    let Some(target) = args.input.clone() else {
        let question = if dangerous {
            "Hard reset to (this throws away every change): "
        } else {
            "Reset to: "
        };
        editor.prompt_for(command, MinibufferKind::Text, question, "HEAD", Vec::new());
        return Ok(());
    };
    let target = target.trim().to_string();
    if target.is_empty() {
        return Err(crate::CoreError::Message("Nothing to reset to".into()));
    }
    run(editor, &["reset", mode, &target], "Reset")
}

fn tag_create(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        editor.prompt_for(
            "magit-tag-create",
            MinibufferKind::Text,
            "Tag name: ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(crate::CoreError::Message("No tag named".into()));
    }
    run(editor, &["tag", &name], "Create tag")
}

fn tag_delete(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        let tags = editor.git_tags();
        editor.prompt_for(
            "magit-tag-delete",
            MinibufferKind::Choice,
            "Delete tag: ",
            "",
            tags,
        );
        return Ok(());
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(crate::CoreError::Message("No tag named".into()));
    }
    run(editor, &["tag", "-d", &name], "Delete tag")
}

fn remote_add(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(text) = args.input.clone() else {
        editor.prompt_for(
            "magit-remote-add",
            MinibufferKind::Text,
            "Remote (name url): ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    let mut words = text.split_whitespace();
    match (words.next(), words.next()) {
        (Some(name), Some(url)) => run(editor, &["remote", "add", name, url], "Add remote"),
        _ => Err(crate::CoreError::Message("Give a name and a url".into())),
    }
}

fn remote_remove(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        editor.prompt_for(
            "magit-remote-remove",
            MinibufferKind::Text,
            "Remove remote: ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(crate::CoreError::Message("No remote named".into()));
    }
    run(editor, &["remote", "remove", &name], "Remove remote")
}

fn push_tags(editor: &mut Editor, _: &Args) -> Result<()> {
    run(editor, &["push", "--tags"], "Push tags")
}

fn push_elsewhere(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        editor.prompt_for(
            "magit-push-elsewhere",
            MinibufferKind::Text,
            "Push to remote: ",
            "origin",
            Vec::new(),
        );
        return Ok(());
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(crate::CoreError::Message("No remote named".into()));
    }
    let branch = editor
        .git
        .status
        .branch
        .clone()
        .unwrap_or_else(|| "HEAD".into());
    let mut arguments = vec!["push".to_string(), name, branch];
    arguments.extend(editor.transient_arguments.clone());
    act(
        editor,
        GitAction::Run {
            arguments,
            describe: "Push".into(),
        },
    )
}

fn fetch_all(editor: &mut Editor, _: &Args) -> Result<()> {
    run(editor, &["fetch", "--all", "--prune"], "Fetch everything")
}

fn cherry_pick(editor: &mut Editor, _: &Args) -> Result<()> {
    let revision = revision_here(editor)?;
    run(editor, &["cherry-pick", &revision], "Cherry-pick")
}

fn revert(editor: &mut Editor, _: &Args) -> Result<()> {
    let revision = revision_here(editor)?;
    run(editor, &["revert", "--no-edit", &revision], "Revert")
}

fn sequencer_continue(editor: &mut Editor, _: &Args) -> Result<()> {
    run(editor, &["cherry-pick", "--continue"], "Continue")
}

fn sequencer_abort(editor: &mut Editor, _: &Args) -> Result<()> {
    run(editor, &["cherry-pick", "--abort"], "Abort")
}

fn run_git(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(text) = args.input.clone() else {
        editor.prompt_for(
            "magit-run",
            MinibufferKind::Text,
            "Run: git ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    let arguments: Vec<String> = text.split_whitespace().map(str::to_string).collect();
    if arguments.is_empty() {
        return Err(crate::CoreError::Message("Nothing to run".into()));
    }
    act(
        editor,
        GitAction::Run {
            arguments,
            describe: format!("git {text}"),
        },
    )
}

/// Adds the file here to `.gitignore`.
///
/// Through the shell rather than by rewriting the file: appending a line is
/// what `>>` is for, and reading the whole file to add one line to it risks
/// losing whatever else was written while the editor was not looking.
fn gitignore(editor: &mut Editor, _: &Args) -> Result<()> {
    let path = file_here(editor)?;
    let directory = root(editor)?;
    editor.spawn(Task::Shell {
        // Quoted, because a path may contain anything a shell would read.
        command: format!("printf '%s\\n' {} >> .gitignore", crate::shell_quote(&path)),
        directory,
        insert_at: None,
    });
    editor.message(format!("Ignoring {path}"));
    refresh(editor, &Args::default())
}

/// The row point is on.
fn row(editor: &Editor) -> Result<Row> {
    editor
        .git_row_at_cursor()
        .cloned()
        .ok_or_else(|| crate::CoreError::Message("Nothing here".into()))
}

fn root(editor: &Editor) -> Result<PathBuf> {
    editor
        .git_root
        .clone()
        .ok_or_else(|| crate::CoreError::Message("Not in a git repository".into()))
}

/// Queues a git action and asks for a refresh when it answers.
fn act(editor: &mut Editor, action: GitAction) -> Result<()> {
    let root = root(editor)?;
    editor.spawn(Task::Git { root, action });
    Ok(())
}

// ---- opening and closing ------------------------------------------------

fn status(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = match editor.buffers.find_by_name(STATUS_BUFFER_NAME) {
        Some(id) => id,
        None => {
            let id = editor.buffers.create_with_text(STATUS_BUFFER_NAME, "");
            editor
                .buffers
                .get_mut(id)
                .expect("just created")
                .set_read_only(true);
            id
        }
    };
    // In the window that is there, as magit does: the status view is what you
    // are looking at while you use it, not a strip beside something else.
    editor.switch_to_buffer(id)?;
    editor.render_git_buffer();
    // The first refresh starts from wherever the editor is; git resolves the
    // repository from there and says where it really is.
    let from = editor
        .git_root
        .clone()
        .unwrap_or_else(|| editor.default_directory());
    editor.spawn(Task::Git {
        root: from,
        action: GitAction::Refresh,
    });
    Ok(())
}

fn refresh(editor: &mut Editor, _: &Args) -> Result<()> {
    act(editor, GitAction::Refresh)
}

/// `q`: closes the magit view, killing it.
///
/// Magit's views are working views — a status, a log, a commit being read.
/// Buried, they collect in `C-x b` and end up being killed by hand, so `q`
/// kills. `C-u q` buries instead, for a view worth keeping.
///
/// What comes up next is the most recently visited buffer left, which is the
/// one the view was opened from: a commit goes back to the log it was picked
/// from, the log to the status, the status to the file being edited.
fn quit(editor: &mut Editor, args: &Args) -> Result<()> {
    if args.prefix.is_present() {
        editor.bury_buffer();
        return Ok(());
    }
    let id = editor.current_buffer_id();
    let name = editor.current_buffer().name().to_string();
    match editor.kill_buffer(id) {
        Ok(_) => Ok(()),
        // The only buffer there is: burying it is the most that can be done.
        Err(crate::CoreError::LastBuffer) => {
            editor.bury_buffer();
            Ok(())
        }
        Err(error) => {
            editor.error(format!("Cannot close {name}: {error}"));
            Ok(())
        }
    }
}

// ---- moving and folding -------------------------------------------------

/// `TAB`: folds whatever point is on, at whatever level that is.
fn toggle(editor: &mut Editor, _: &Args) -> Result<()> {
    // A diff buffer folds by file; a list has nothing to fold.
    let name = editor.current_buffer().name().to_string();
    if editor.git_diffs.contains_key(&name) {
        return toggle_diff_file(editor, &name);
    }
    if editor.git_lists.contains_key(&name) {
        return Ok(());
    }
    let here = row(editor)?;
    match here {
        Row::Section(section) => editor.git.toggle_section(section),
        Row::File { section, file } => {
            let Some(path) = editor.git.paths(section).get(file).cloned() else {
                return Ok(());
            };
            editor.git.toggle_file(section, &path);
        }
        Row::Hunk {
            section,
            file,
            hunk,
        }
        | Row::Line {
            section,
            file,
            hunk,
            ..
        } => {
            let Some(path) = editor.git.paths(section).get(file).cloned() else {
                return Ok(());
            };
            editor.git.toggle_hunk(section, &path, hunk);
        }
        _ => return Ok(()),
    }
    editor.render_git_buffer();
    Ok(())
}

fn toggle_all(editor: &mut Editor, _: &Args) -> Result<()> {
    let folded = crate::git::SECTIONS
        .iter()
        .all(|s| editor.git.is_collapsed(*s));
    if folded {
        editor.git.expand_all();
    } else {
        editor.git.collapse_all();
    }
    editor.render_git_buffer();
    Ok(())
}

fn next_section(editor: &mut Editor, _: &Args) -> Result<()> {
    move_section(editor, 1, None)
}

fn previous_section(editor: &mut Editor, _: &Args) -> Result<()> {
    move_section(editor, -1, None)
}

/// `M-n`: the next section at the same depth, stepping over what is inside
/// the one point is on rather than into it.
fn next_sibling(editor: &mut Editor, _: &Args) -> Result<()> {
    let level = here_level(editor);
    move_section(editor, 1, level)
}

fn previous_sibling(editor: &mut Editor, _: &Args) -> Result<()> {
    let level = here_level(editor);
    move_section(editor, -1, level)
}

/// `^`: out one level, to whatever contains this.
fn parent_section(editor: &mut Editor, _: &Args) -> Result<()> {
    let Some(level) = here_level(editor) else {
        return Err(crate::CoreError::Message("Nothing above this".into()));
    };
    if level == 0 {
        return Err(crate::CoreError::Message("Already at the top".into()));
    }
    let here = editor.git_cursor_line();
    let target = editor
        .git
        .rows()
        .iter()
        .enumerate()
        .take(here)
        .rev()
        .find(|(_, row)| row.level().is_some_and(|found| found < level))
        .map(|(line, _)| line);
    match target {
        Some(line) => {
            editor.move_git_cursor_to_line(line);
            Ok(())
        }
        None => Err(crate::CoreError::Message("Nothing above this".into())),
    }
}

fn here_level(editor: &Editor) -> Option<usize> {
    editor.git_row_at_cursor().and_then(Row::level)
}

/// Moves to the next or previous section, optionally only at `level`.
fn move_section(editor: &mut Editor, delta: isize, level: Option<usize>) -> Result<()> {
    let stops: Vec<usize> = editor
        .git
        .rows()
        .iter()
        .enumerate()
        .filter(|(_, row)| match (row.level(), level) {
            (Some(found), Some(wanted)) => found == wanted,
            (Some(_), None) => true,
            _ => false,
        })
        .map(|(line, _)| line)
        .collect();
    if stops.is_empty() {
        return Err(crate::CoreError::Message("Nothing to move between".into()));
    }
    let here = editor.git_cursor_line();
    let target = match delta > 0 {
        true => stops.iter().find(|line| **line > here).copied(),
        false => stops.iter().rev().find(|line| **line < here).copied(),
    };
    // Stopping at the ends rather than wrapping: a status view is read top to
    // bottom, and coming back round is disorienting in a long one.
    let target = target.unwrap_or_else(|| {
        if delta > 0 {
            *stops.last().expect("checked")
        } else {
            stops[0]
        }
    });
    editor.move_git_cursor_to_line(target);
    Ok(())
}

/// Folds the file point is on in a diff or revision buffer.
fn toggle_diff_file(editor: &mut Editor, name: &str) -> Result<()> {
    let line = editor
        .current_buffer()
        .line_of(editor.windows.current().point);
    let Some(view) = editor.git_diffs.get(name) else {
        return Ok(());
    };
    // Which file, whichever of its rows point happens to be on. Folding takes
    // the rows inside it away, so point comes back to the file's own heading —
    // where magit leaves it, and the row that is certain to still be there.
    let file = match view.row(line) {
        Some(crate::git::DiffRow::File(index))
        | Some(crate::git::DiffRow::Hunk(index, _))
        | Some(crate::git::DiffRow::Line(index, _, _)) => Some(*index),
        _ => None,
    };
    let Some(file) = file else { return Ok(()) };
    let Some(path) = view.files.get(file).map(|f| f.path.clone()) else {
        return Ok(());
    };
    if let Some(view) = editor.git_diffs.get_mut(name) {
        view.toggle_file(&path);
    }
    let view = editor.git_diffs.get(name).cloned().expect("just looked at");
    let name: &'static str = crate::commands::git::MAGIT_BUFFERS
        .iter()
        .find(|candidate| **candidate == name)
        .copied()
        .unwrap_or(DIFF_BUFFER_NAME);
    editor.open_git_diff_showing(name, view, Some(crate::git::DiffRow::File(file)))
}

/// `RET`: opens the file the row is about, at the line the hunk is about.
fn visit(editor: &mut Editor, args: &Args) -> Result<()> {
    // In a list, `RET` acts on what the line stands for: a commit is shown, a
    // branch is checked out.
    let name = editor.current_buffer().name().to_string();
    if name == LOG_BUFFER_NAME {
        return show_commit(editor, args);
    }
    if name == REFS_BUFFER_NAME {
        let Some(target) = editor.git_list_target() else {
            return Err(crate::CoreError::Message("Nothing here".into()));
        };
        return act(editor, GitAction::Checkout(target));
    }
    if editor.git_diffs.contains_key(&name) {
        return visit_diff_line(editor, &name);
    }
    if editor.git_lists.contains_key(&name) {
        return Ok(());
    }
    let here = row(editor)?;
    // A commit is shown in full rather than opened as a file.
    if matches!(here, Row::Commit { .. } | Row::Stash(_)) {
        return show_commit(editor, args);
    }
    let (section, file, line) = match here {
        Row::File { section, file } => (section, file, None),
        Row::Hunk {
            section,
            file,
            hunk,
        } => (section, file, hunk_line(editor, section, file, hunk, 0)),
        Row::Line {
            section,
            file,
            hunk,
            line,
        } => (section, file, hunk_line(editor, section, file, hunk, line)),
        _ => return Err(crate::CoreError::Message("Nothing to open here".into())),
    };
    let Some(path) = editor.git.paths(section).get(file).cloned() else {
        return Err(crate::CoreError::Message("Nothing to open here".into()));
    };
    open_at(editor, &path, line)
}

/// Opens `path` relative to the repository, at `line` if one is given.
fn open_at(editor: &mut Editor, path: &str, line: Option<usize>) -> Result<()> {
    let full = root(editor)?.join(path);
    // A line, not a protocol position: a hunk header counts lines, and going
    // through the language server's coordinates to express that would tie
    // magit to a language server it has no need of.
    editor.pending_line = line.map(|line| (full.clone(), line.saturating_sub(1)));
    match editor.buffers.find_by_path(&full) {
        Some(id) => {
            editor.switch_to_buffer(id)?;
            // The file is already open, so the jump happens now rather than
            // waiting for a read that will not come.
            if let Some((_, line)) = editor.pending_line.take() {
                editor.go_to_line(line);
            }
        }
        None => editor.spawn(Task::ReadFile {
            path: full,
            reverting: None,
            other_window: false,
        }),
    }
    Ok(())
}

/// `RET` in a diff or revision buffer: the file, at the line under point.
fn visit_diff_line(editor: &mut Editor, name: &str) -> Result<()> {
    let at = editor
        .current_buffer()
        .line_of(editor.windows.current().point);
    let Some(view) = editor.git_diffs.get(name) else {
        return Ok(());
    };
    let (file, line) = match view.row(at) {
        Some(crate::git::DiffRow::File(index)) => (*index, None),
        Some(crate::git::DiffRow::Hunk(index, hunk)) => (*index, Some((*hunk, 0))),
        Some(crate::git::DiffRow::Line(index, hunk, line)) => (*index, Some((*hunk, *line))),
        _ => return Err(crate::CoreError::Message("Nothing to open here".into())),
    };
    let Some(diff) = view.files.get(file) else {
        return Err(crate::CoreError::Message("Nothing to open here".into()));
    };
    let path = diff.path.clone();
    let target = line.and_then(|(hunk, line)| {
        let hunk = diff.hunks.get(hunk)?;
        let offset = hunk
            .lines
            .iter()
            .take(line)
            .filter(|l| l.kind != maxgus_git::LineKind::Removed)
            .count();
        Some(hunk.new_start + offset)
    });
    open_at(editor, &path, target)
}

/// Which line of the file a hunk's row stands for.
fn hunk_line(
    editor: &Editor,
    section: Section,
    file: usize,
    hunk: usize,
    line: usize,
) -> Option<usize> {
    let hunk = editor.git.files(section).get(file)?.hunks.get(hunk)?;
    // Count the lines that exist in the new file up to this one, so pointing
    // at a removed line lands where it was rather than somewhere later.
    let offset = hunk
        .lines
        .iter()
        .take(line)
        .filter(|l| l.kind != maxgus_git::LineKind::Removed)
        .count();
    Some(hunk.new_start + offset)
}

// ---- staging ------------------------------------------------------------

/// What `s`, `u` and `k` act on: a whole section, a file, or one hunk.
enum Target {
    Section(Section),
    Paths(Section, Vec<PathBuf>),
    Hunk {
        section: Section,
        patch: String,
        path: String,
    },
}

fn target(editor: &Editor) -> Result<Target> {
    let here = row(editor)?;
    let section = here
        .section()
        .ok_or_else(|| crate::CoreError::Message("Nothing to do here".into()))?;
    match here {
        Row::Section(section) => Ok(Target::Section(section)),
        Row::File { file, .. } => {
            let path = editor
                .git
                .paths(section)
                .get(file)
                .cloned()
                .ok_or_else(|| crate::CoreError::Message("No file here".into()))?;
            Ok(Target::Paths(section, vec![PathBuf::from(path)]))
        }
        Row::Hunk { file, hunk, .. } | Row::Line { file, hunk, .. } => {
            let diff = editor
                .git
                .files(section)
                .get(file)
                .ok_or_else(|| crate::CoreError::Message("No change here".into()))?;
            let piece = diff
                .hunks
                .get(hunk)
                .ok_or_else(|| crate::CoreError::Message("No hunk here".into()))?;
            Ok(Target::Hunk {
                section,
                patch: maxgus_git::diff::hunk_patch(diff, piece),
                path: diff.path.clone(),
            })
        }
        _ => Err(crate::CoreError::Message("Nothing to stage here".into())),
    }
}

fn stage(editor: &mut Editor, _: &Args) -> Result<()> {
    match target(editor)? {
        Target::Section(Section::Staged) => Err(crate::CoreError::Message("Already staged".into())),
        Target::Section(section) => {
            let paths = paths_of(editor, section);
            act(editor, GitAction::Stage(paths))
        }
        Target::Paths(Section::Staged, _) => {
            Err(crate::CoreError::Message("Already staged".into()))
        }
        Target::Paths(_, paths) => act(editor, GitAction::Stage(paths)),
        Target::Hunk {
            section: Section::Staged,
            ..
        } => Err(crate::CoreError::Message("Already staged".into())),
        Target::Hunk { patch, path, .. } => act(
            editor,
            GitAction::ApplyPatch {
                patch,
                arguments: vec!["--cached".into()],
                describe: format!("Stage a hunk of {path}"),
            },
        ),
    }
}

fn unstage(editor: &mut Editor, _: &Args) -> Result<()> {
    match target(editor)? {
        Target::Section(Section::Staged) => {
            let paths = paths_of(editor, Section::Staged);
            act(editor, GitAction::Unstage(paths))
        }
        Target::Paths(Section::Staged, paths) => act(editor, GitAction::Unstage(paths)),
        Target::Hunk {
            section: Section::Staged,
            patch,
            path,
        } => act(
            editor,
            // The same patch, reversed: it came from `git diff --cached`, so
            // undoing it is exactly taking it back out of the index.
            GitAction::ApplyPatch {
                patch,
                arguments: vec!["--cached".into(), "--reverse".into()],
                describe: format!("Unstage a hunk of {path}"),
            },
        ),
        _ => Err(crate::CoreError::Message("That is not staged".into())),
    }
}

fn stage_all(editor: &mut Editor, _: &Args) -> Result<()> {
    act(editor, GitAction::StageAll)
}

fn unstage_all(editor: &mut Editor, _: &Args) -> Result<()> {
    act(editor, GitAction::UnstageAll)
}

/// `k`: throws the change away. The one thing here that cannot be undone, so
/// it asks first and says exactly what it is about to lose.
fn discard(editor: &mut Editor, args: &Args) -> Result<()> {
    let what = describe_target(editor)?;
    let Some(answer) = args.input.clone() else {
        editor.prompt_for(
            "magit-discard",
            MinibufferKind::YesNo,
            format!("Discard {what}? (yes or no) "),
            "",
            Vec::new(),
        );
        return Ok(());
    };
    if !answer.trim().eq_ignore_ascii_case("yes") {
        editor.message("Nothing discarded".to_string());
        return Ok(());
    }
    match target(editor)? {
        Target::Section(Section::Untracked) | Target::Paths(Section::Untracked, _) => {
            let paths = match target(editor)? {
                Target::Paths(_, paths) => paths,
                _ => paths_of(editor, Section::Untracked),
            };
            act(editor, GitAction::DeleteUntracked(paths))
        }
        Target::Section(section) => {
            let paths = paths_of(editor, section);
            act(editor, GitAction::Discard(paths))
        }
        Target::Paths(_, paths) => act(editor, GitAction::Discard(paths)),
        Target::Hunk {
            section,
            patch,
            path,
        } => {
            let arguments = match section {
                // A staged hunk is thrown away from both the index and the
                // tree, or it would come back the moment anything refreshed.
                Section::Staged => vec!["--index".into(), "--reverse".into()],
                _ => vec!["--reverse".into()],
            };
            act(
                editor,
                GitAction::ApplyPatch {
                    patch,
                    arguments,
                    describe: format!("Discard a hunk of {path}"),
                },
            )
        }
    }
}

/// What `discard` is about to lose, in words, for the question it asks.
fn describe_target(editor: &Editor) -> Result<String> {
    Ok(match target(editor)? {
        Target::Section(section) => {
            format!("every change in {}", section.title().to_lowercase())
        }
        Target::Paths(_, paths) => match paths.len() {
            1 => format!("changes to {}", paths[0].display()),
            n => format!("changes to {n} files"),
        },
        Target::Hunk { path, .. } => format!("a hunk of {path}"),
    })
}

fn paths_of(editor: &Editor, section: Section) -> Vec<PathBuf> {
    editor
        .git
        .paths(section)
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

// ---- committing ---------------------------------------------------------

/// Opens a buffer to write the message in, as magit does.
///
/// A commit message is prose, sometimes long, and belongs in a buffer with
/// the editor's own keys rather than in a one-line prompt.
fn commit(editor: &mut Editor, _: &Args) -> Result<()> {
    open_commit_buffer(editor, false, String::new())
}

fn commit_amend(editor: &mut Editor, _: &Args) -> Result<()> {
    let subject = editor.git.head_subject.clone();
    open_commit_buffer(editor, true, subject)
}

/// Adds what is staged to the last commit without touching its message.
fn commit_extend(editor: &mut Editor, _: &Args) -> Result<()> {
    let message = editor.git.head_subject.clone();
    let arguments = menu_arguments(editor);
    act(
        editor,
        GitAction::Commit {
            message,
            amend: true,
            arguments,
        },
    )
}

fn open_commit_buffer(editor: &mut Editor, amend: bool, initial: String) -> Result<()> {
    root(editor)?;
    if editor.git.status.staged().count() == 0 && !amend {
        return Err(crate::CoreError::Message("Nothing is staged".into()));
    }
    let id = match editor.buffers.find_by_name(COMMIT_BUFFER_NAME) {
        Some(id) => id,
        None => editor.buffers.create(COMMIT_BUFFER_NAME),
    };
    editor.replace_buffer_contents(id, &initial).ok();
    editor.committing_amend = amend;
    // Kept until the message is finished: the menu is long gone by then.
    editor.committing_arguments = menu_arguments(editor);
    editor.switch_to_buffer(id)?;
    editor.message("C-c C-c to commit, C-c C-k to abandon".to_string());
    Ok(())
}

fn commit_finish(editor: &mut Editor, _: &Args) -> Result<()> {
    let message = editor.current_buffer().text();
    // Comment lines are stripped the way git strips them, so a template can
    // explain itself without ending up in the history.
    let message: String = message
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    if message.trim().is_empty() {
        return Err(crate::CoreError::Message("An empty commit message".into()));
    }
    let amend = editor.committing_amend;
    let arguments = editor.committing_arguments.clone();
    act(
        editor,
        GitAction::Commit {
            message,
            amend,
            arguments,
        },
    )?;
    close_commit_buffer(editor);
    editor.message("Committing…".to_string());
    Ok(())
}

fn commit_cancel(editor: &mut Editor, _: &Args) -> Result<()> {
    close_commit_buffer(editor);
    editor.message("Commit abandoned".to_string());
    Ok(())
}

/// Closes the message buffer once the message is written or abandoned.
///
/// Killed rather than buried, which is what git does with the file it opened
/// an editor on: kept, it sits in `C-x b` holding a message that has already
/// been used, and the next commit opens on top of it.
fn close_commit_buffer(editor: &mut Editor) {
    let Some(id) = editor.buffers.find_by_name(COMMIT_BUFFER_NAME) else {
        return;
    };
    if editor.kill_buffer(id).is_err() {
        editor.bury_buffer();
    }
}

// ---- remotes and branches -----------------------------------------------

/// The switches the menu had on when it ran this.
fn menu_arguments(editor: &Editor) -> Vec<String> {
    editor.transient_arguments.clone()
}

fn push(editor: &mut Editor, _: &Args) -> Result<()> {
    let arguments = menu_arguments(editor);
    act(editor, GitAction::Push { arguments })
}

fn push_force(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(answer) = args.input.clone() else {
        editor.prompt_for(
            "magit-push-force",
            MinibufferKind::YesNo,
            "Force push, overwriting the upstream? (yes or no) ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    if !answer.trim().eq_ignore_ascii_case("yes") {
        editor.message("Not pushed".to_string());
        return Ok(());
    }
    let mut arguments = menu_arguments(editor);
    if !arguments.iter().any(|flag| flag == "--force-with-lease") {
        arguments.push("--force-with-lease".into());
    }
    act(editor, GitAction::Push { arguments })
}

fn pull(editor: &mut Editor, _: &Args) -> Result<()> {
    let arguments = menu_arguments(editor);
    act(editor, GitAction::Pull { arguments })
}

fn fetch(editor: &mut Editor, _: &Args) -> Result<()> {
    let arguments = menu_arguments(editor);
    act(editor, GitAction::Fetch { arguments })
}

fn checkout(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        let branches = editor.git_branches.clone();
        editor.prompt_for(
            "magit-checkout",
            MinibufferKind::Choice,
            "Check out: ",
            "",
            branches,
        );
        return Ok(());
    };
    if name.trim().is_empty() {
        return Err(crate::CoreError::Message("No branch named".into()));
    }
    act(editor, GitAction::Checkout(name.trim().to_string()))
}

fn branch_create(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        editor.prompt_for(
            "magit-branch-create",
            MinibufferKind::Text,
            "New branch: ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    if name.trim().is_empty() {
        return Err(crate::CoreError::Message("No branch named".into()));
    }
    act(editor, GitAction::CreateBranch(name.trim().to_string()))
}

fn merge(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(name) = args.input.clone() else {
        let branches = editor.git_branches.clone();
        editor.prompt_for(
            "magit-merge",
            MinibufferKind::Choice,
            "Merge: ",
            "",
            branches,
        );
        return Ok(());
    };
    if name.trim().is_empty() {
        return Err(crate::CoreError::Message("No branch named".into()));
    }
    act(editor, GitAction::Merge(name.trim().to_string()))
}

// ---- stashes ------------------------------------------------------------

fn stash(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(message) = args.input.clone() else {
        editor.prompt_for(
            "magit-stash",
            MinibufferKind::Text,
            "Stash message: ",
            "",
            Vec::new(),
        );
        return Ok(());
    };
    let message = (!message.trim().is_empty()).then(|| message.trim().to_string());
    let arguments = menu_arguments(editor);
    act(editor, GitAction::Stash { message, arguments })
}

/// The stash point is on, by the name git knows it by.
fn stash_here(editor: &Editor) -> Result<String> {
    match row(editor)? {
        Row::Stash(index) => editor
            .git
            .stashes
            .get(index)
            .map(|stash| stash.name.clone())
            .ok_or_else(|| crate::CoreError::Message("No stash here".into())),
        _ => Err(crate::CoreError::Message("No stash here".into())),
    }
}

fn stash_pop(editor: &mut Editor, _: &Args) -> Result<()> {
    let name = stash_here(editor)?;
    act(editor, GitAction::StashPop(name))
}

fn stash_apply(editor: &mut Editor, _: &Args) -> Result<()> {
    let name = stash_here(editor)?;
    act(editor, GitAction::StashApply(name))
}

fn stash_drop(editor: &mut Editor, args: &Args) -> Result<()> {
    let name = stash_here(editor)?;
    let Some(answer) = args.input.clone() else {
        editor.prompt_for(
            "magit-stash-drop",
            MinibufferKind::YesNo,
            format!("Drop {name}? (yes or no) "),
            "",
            Vec::new(),
        );
        return Ok(());
    };
    if !answer.trim().eq_ignore_ascii_case("yes") {
        editor.message("Kept".to_string());
        return Ok(());
    }
    act(editor, GitAction::StashDrop(name))
}

fn help(editor: &mut Editor, _: &Args) -> Result<()> {
    let mut text = String::from("Git status keys\n\n");
    for (keys, command) in crate::keymap::MAGIT_BINDINGS {
        text.push_str(&format!("{keys:<10} {command}\n"));
    }
    let id = match editor.buffers.find_by_name("*Help*") {
        Some(id) => {
            editor.replace_buffer_contents(id, &text)?;
            id
        }
        None => editor.buffers.create_with_text("*Help*", &text),
    };
    editor
        .buffers
        .get_mut(id)
        .expect("just created")
        .set_read_only(true);
    editor.switch_to_buffer(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transient::{Action, TRANSIENTS};

    #[test]
    fn every_key_the_status_view_binds_reaches_a_command() {
        let mut registry = Registry::new();
        register(&mut registry);
        crate::commands::motion::register(&mut registry);
        crate::commands::edit::register(&mut registry);
        crate::commands::window::register(&mut registry);
        for (keys, command) in crate::keymap::MAGIT_BINDINGS {
            assert!(
                registry.contains(command),
                "`{keys}` runs `{command}`, which is not registered"
            );
        }
    }

    #[test]
    fn every_command_a_menu_offers_is_a_command_that_exists() {
        // The menus are a table, so an entry naming a command that was
        // renamed is a line in a list nothing else would notice.
        let registry = crate::standard_registry();
        for transient in TRANSIENTS {
            for item in transient.groups.iter().flat_map(|group| group.items) {
                if let Action::Command(name) = item.action {
                    assert!(
                        registry.contains(name),
                        "`{}` in the {} menu runs `{name}`, which is not registered",
                        item.key,
                        transient.name
                    );
                }
            }
        }
    }

    #[test]
    fn every_menu_can_be_opened_by_a_key_or_from_another_menu() {
        // A menu nothing opens is a menu nobody will find. The top one is
        // opened by `?`; the rest are reached from it or bound directly.
        let bound: Vec<&str> = crate::keymap::MAGIT_BINDINGS
            .iter()
            .map(|(_, command)| *command)
            .collect();
        for transient in TRANSIENTS {
            let command = format!("magit-{}-menu", transient.name);
            let reachable = transient.name == "dispatch"
                || bound.contains(&command.as_str())
                || TRANSIENTS.iter().any(|other| {
                    other.groups.iter().flat_map(|group| group.items).any(|item| {
                        matches!(item.action, Action::Prefix(name) if name == transient.name)
                    })
                });
            assert!(reachable, "the {} menu cannot be opened", transient.name);
        }
    }
}
