//! Dired: working on a directory rather than browsing it.
//!
//! `C-x d` opens one. `m` marks, `u` unmarks, `t` swaps the marks, `d` flags
//! for deletion and `x` carries the flags out. Everything else — `D` delete,
//! `C` copy, `R` rename, `!` a shell command — acts on what is marked, or on
//! the line point is on when nothing is, which is dired's own rule and the
//! reason marking is worth having.

use crate::command;
use crate::command::{Args, Registry};
use crate::dired::{DiredView, Mark, Row};
use crate::editor::Editor;
use crate::minibuffer::MinibufferKind;
use crate::task::{FileAction, Task};
use crate::{CoreError, Result};
use std::path::PathBuf;

pub const DIRED_BUFFER_NAME: &str = "*dired*";
pub const DIRED_MODE: &str = "dired-mode";

pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!("dired", "Open a directory as a buffer.", dired),
        command!(
            "dired-visit",
            "Open what the line names.",
            visit,
            non_interactive
        ),
        command!(
            "dired-up",
            "Go to the directory above.",
            up,
            non_interactive
        ),
        command!(
            "dired-next",
            "Move to the next line.",
            next,
            non_interactive
        ),
        command!(
            "dired-previous",
            "Move to the previous line.",
            previous,
            non_interactive
        ),
        command!("dired-refresh", "Read the directory again.", refresh),
        command!(
            "dired-mark",
            "Mark this line and move on.",
            mark,
            non_interactive
        ),
        command!(
            "dired-unmark",
            "Unmark this line and move on.",
            unmark,
            non_interactive
        ),
        command!(
            "dired-unmark-all",
            "Take off every mark.",
            unmark_all,
            non_interactive
        ),
        command!(
            "dired-toggle-marks",
            "Mark what is not marked, and the reverse.",
            toggle_marks,
            non_interactive
        ),
        command!(
            "dired-flag-deletion",
            "Flag this line for deletion.",
            flag,
            non_interactive
        ),
        command!(
            "dired-do-flagged-delete",
            "Delete everything flagged, after asking.",
            do_flagged,
            non_interactive
        ),
        command!("dired-do-delete", "Delete what is marked.", do_delete),
        command!("dired-do-copy", "Copy what is marked.", do_copy),
        command!(
            "dired-do-rename",
            "Rename or move what is marked.",
            do_rename
        ),
        command!(
            "dired-create-directory",
            "Make a directory here.",
            create_directory
        ),
        command!(
            "dired-do-shell-command",
            "Run a command over what is marked.",
            do_shell
        ),
        command!("dired-quit", "Close the directory.", quit, non_interactive),
    ]);
}

fn view(editor: &Editor) -> Result<&DiredView> {
    editor
        .dired
        .as_ref()
        .ok_or_else(|| CoreError::Message("This is not a directory listing".into()))
}

fn line(editor: &Editor) -> usize {
    editor
        .current_buffer()
        .line_of(editor.windows.current().point)
}

/// `C-x d`: opens a directory.
fn dired(editor: &mut Editor, args: &Args) -> Result<()> {
    let Some(input) = args.input.clone() else {
        let here = editor.default_directory();
        editor.prompt_for(
            "dired",
            MinibufferKind::File,
            "Dired: ".to_string(),
            &format!("{}/", here.display().to_string().trim_end_matches('/')),
            Vec::new(),
        );
        return Ok(());
    };
    if input.trim().is_empty() {
        return Err(CoreError::Message("No directory given".into()));
    }
    // `~`, a relative path and a `//` started over, as at every file prompt;
    // and the root, which trimming its slash used to turn into nothing.
    let path = crate::commands::file::expand(editor, &input);
    editor.spawn(Task::Dired { path });
    Ok(())
}

/// A path typed at one of dired's prompts, read against the directory being
/// listed rather than wherever the editor was started.
fn typed_path(editor: &Editor, input: &str) -> Result<PathBuf> {
    let here = view(editor)?.path.clone();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    Ok(crate::commands::file::expand_against(
        &here,
        home.as_deref(),
        input,
    ))
}

/// Puts a listing on screen, keeping point on whatever it was on.
pub fn show(editor: &mut Editor, path: PathBuf, entries: Vec<crate::dired::Entry>) -> Result<()> {
    let was_on = editor
        .dired
        .as_ref()
        .filter(|view| view.path == path)
        .and_then(|view| view.entry(line(editor)))
        .map(|entry| entry.name.clone());
    let view = match editor.dired.take() {
        Some(previous) if previous.path == path => previous.refreshed(entries),
        _ => DiredView::new(path, entries),
    };
    let text = view.text();
    let target = was_on
        .and_then(|name| view.line_of_name(&name))
        .unwrap_or_else(|| view.first_entry_line());
    editor.dired = Some(view);
    let id = match editor.buffers.find_by_name(DIRED_BUFFER_NAME) {
        Some(id) => {
            editor.replace_buffer_contents(id, &text).ok();
            id
        }
        None => editor.buffers.create_with_text(DIRED_BUFFER_NAME, &text),
    };
    if let Some(buffer) = editor.buffers.get_mut(id) {
        buffer.set_read_only(true);
    }
    // A listing already on the screen is redrawn where it is; one that is
    // not is brought to the window being edited in.
    if editor.windows.showing(id).is_empty() {
        editor.show_in_editing_window(id)?;
    }
    editor.move_point_in(id, target);
    Ok(())
}

/// Lists `path` again after something changed it — but only when a listing
/// of it is up to be wrong. A file deleted with `C-c f D` from its own
/// buffer opened dired on its directory, which nobody had asked to see.
pub fn relist_if_showing(editor: &mut Editor, path: PathBuf) {
    let showing = editor
        .buffers
        .find_by_name(DIRED_BUFFER_NAME)
        .is_some_and(|id| !editor.windows.showing(id).is_empty());
    if showing && editor.dired.as_ref().is_some_and(|view| view.path == path) {
        editor.spawn(Task::Dired { path });
    }
}

fn visit(editor: &mut Editor, _: &Args) -> Result<()> {
    let at = line(editor);
    let view = view(editor)?;
    let path = view
        .target(at)
        .ok_or_else(|| CoreError::Message("Nothing here".into()))?;
    let is_dir = matches!(view.row(at), Some(Row::Parent))
        || view.entry(at).is_some_and(|entry| entry.is_dir);
    if is_dir {
        editor.spawn(Task::Dired { path });
        return Ok(());
    }
    if let Some(id) = editor.buffers.find_by_path(&path) {
        return editor.show_in_editing_window(id);
    }
    editor.spawn(Task::ReadFile {
        path,
        reverting: None,
        other_window: false,
    });
    Ok(())
}

fn up(editor: &mut Editor, _: &Args) -> Result<()> {
    let path = view(editor)?
        .path
        .parent()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| CoreError::Message("This is the root".into()))?;
    editor.spawn(Task::Dired { path });
    Ok(())
}

fn step(editor: &mut Editor, forward: bool) -> Result<()> {
    let at = line(editor);
    let next = view(editor)?
        .step(at, forward)
        .ok_or_else(|| CoreError::Message("No further".into()))?;
    let id = editor.current_buffer_id();
    editor.move_point_in(id, next);
    Ok(())
}

fn next(editor: &mut Editor, _: &Args) -> Result<()> {
    step(editor, true)
}

fn previous(editor: &mut Editor, _: &Args) -> Result<()> {
    step(editor, false)
}

fn refresh(editor: &mut Editor, _: &Args) -> Result<()> {
    let path = view(editor)?.path.clone();
    editor.spawn(Task::Dired { path });
    Ok(())
}

/// Marks and redraws, then moves on: marking a run of files is `m m m`.
fn set_mark_and_move(editor: &mut Editor, mark: Mark) -> Result<()> {
    let at = line(editor);
    let changed = editor
        .dired
        .as_mut()
        .ok_or_else(|| CoreError::Message("This is not a directory listing".into()))?
        .set_mark(at, mark);
    // A line with nothing to mark — `..` — still moves on, as it does in
    // dired: `m` held down over a directory should not stop at the top.
    if changed {
        redraw(editor)?;
    }
    let next = view(editor)?.step(at, true).unwrap_or(at);
    let id = editor.current_buffer_id();
    editor.move_point_in(id, next);
    Ok(())
}

fn mark(editor: &mut Editor, _: &Args) -> Result<()> {
    set_mark_and_move(editor, Mark::Marked)
}

fn unmark(editor: &mut Editor, _: &Args) -> Result<()> {
    set_mark_and_move(editor, Mark::None)
}

fn flag(editor: &mut Editor, _: &Args) -> Result<()> {
    set_mark_and_move(editor, Mark::Deleted)
}

fn unmark_all(editor: &mut Editor, _: &Args) -> Result<()> {
    editor
        .dired
        .as_mut()
        .ok_or_else(|| CoreError::Message("This is not a directory listing".into()))?
        .mark_all(Mark::None);
    redraw(editor)
}

fn toggle_marks(editor: &mut Editor, _: &Args) -> Result<()> {
    editor
        .dired
        .as_mut()
        .ok_or_else(|| CoreError::Message("This is not a directory listing".into()))?
        .toggle_marks();
    redraw(editor)
}

/// Rewrites the buffer from the view, keeping point on its line.
fn redraw(editor: &mut Editor) -> Result<()> {
    let text = view(editor)?.text();
    let at = line(editor);
    let id = editor.current_buffer_id();
    editor.replace_buffer_contents(id, &text).ok();
    editor.move_point_in(id, at);
    Ok(())
}

/// The files an operation is about.
fn targets(editor: &Editor) -> Result<Vec<PathBuf>> {
    let acting = view(editor)?.acting_on(line(editor));
    match acting.is_empty() {
        true => Err(CoreError::Message("Nothing to act on".into())),
        false => Ok(acting),
    }
}

/// `x`: deletes what is flagged — after asking, as `D` does.
///
/// It asked nothing. The flags are set a key at a time, often a while
/// before, and `x` is one key; the question is where the list of what is
/// about to go gets read.
fn do_flagged(editor: &mut Editor, args: &Args) -> Result<()> {
    let flagged = view(editor)?.with_mark(Mark::Deleted);
    if flagged.is_empty() {
        return Err(CoreError::Message("Nothing is flagged".into()));
    }
    confirm_and_delete(editor, args, "dired-do-flagged-delete", flagged)
}

fn do_delete(editor: &mut Editor, args: &Args) -> Result<()> {
    let paths = targets(editor)?;
    confirm_and_delete(editor, args, "dired-do-delete", paths)
}

/// Deleting is the one thing here that cannot be undone, so it asks — and
/// says exactly what it is about to lose, directories and all.
fn confirm_and_delete(
    editor: &mut Editor,
    args: &Args,
    command: &str,
    paths: Vec<PathBuf>,
) -> Result<()> {
    let Some(answer) = args.input.clone() else {
        let directories = view(editor)?.directories_among(&paths);
        let what = match (paths.len(), directories) {
            (1, 1) => format!("the directory {} and everything in it", paths[0].display()),
            (1, _) => paths[0].display().to_string(),
            (n, 0) => crate::count(n, "file"),
            (n, d) => format!(
                "{} — {} of them with everything in {}",
                crate::count(n, "item"),
                crate::count(d, "directory"),
                if d == 1 { "it" } else { "them" }
            ),
        };
        editor.prompt_for(
            command,
            MinibufferKind::Choice,
            format!("Delete {what}? (yes or no) "),
            "",
            vec!["yes".into(), "no".into()],
        );
        return Ok(());
    };
    if !answer.eq_ignore_ascii_case("yes") && !answer.eq_ignore_ascii_case("y") {
        editor.message("Nothing deleted".to_string());
        return Ok(());
    }
    delete(editor, paths)
}

fn delete(editor: &mut Editor, paths: Vec<PathBuf>) -> Result<()> {
    editor.spawn(Task::DiredAct {
        action: FileAction::Delete(paths),
    });
    Ok(())
}

fn do_copy(editor: &mut Editor, args: &Args) -> Result<()> {
    transfer(editor, args, true)
}

fn do_rename(editor: &mut Editor, args: &Args) -> Result<()> {
    transfer(editor, args, false)
}

fn transfer(editor: &mut Editor, args: &Args, copying: bool) -> Result<()> {
    let paths = targets(editor)?;
    let Some(input) = args.input.clone() else {
        let here = view(editor)?.path.clone();
        let verb = match copying {
            true => "Copy",
            false => "Rename",
        };
        let what = match paths.len() {
            1 => paths[0]
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            n => format!("{n} items"),
        };
        editor.prompt_for(
            match copying {
                true => "dired-do-copy",
                false => "dired-do-rename",
            },
            MinibufferKind::File,
            format!("{verb} {what} to: "),
            &format!("{}/", here.display()),
            Vec::new(),
        );
        return Ok(());
    };
    if input.trim().is_empty() {
        return Err(CoreError::Message("No destination given".into()));
    }
    let to = typed_path(editor, &input)?;
    let action = match copying {
        true => FileAction::Copy { from: paths, to },
        false => FileAction::Rename { from: paths, to },
    };
    editor.spawn(Task::DiredAct { action });
    Ok(())
}

fn create_directory(editor: &mut Editor, args: &Args) -> Result<()> {
    let here = view(editor)?.path.clone();
    let Some(input) = args.input.clone() else {
        editor.prompt_for(
            "dired-create-directory",
            MinibufferKind::File,
            "Create directory: ".to_string(),
            &format!("{}/", here.display()),
            Vec::new(),
        );
        return Ok(());
    };
    if input.trim().is_empty() {
        return Err(CoreError::Message("No directory given".into()));
    }
    let path = typed_path(editor, &input)?;
    editor.spawn(Task::DiredAct {
        action: FileAction::CreateDirectory(path),
    });
    Ok(())
}

/// `!`: runs a command with the marked files as its arguments.
fn do_shell(editor: &mut Editor, args: &Args) -> Result<()> {
    let paths = targets(editor)?;
    let Some(command) = args.input.clone() else {
        editor.prompt_for(
            "dired-do-shell-command",
            MinibufferKind::Shell,
            format!("! on {}: ", crate::count(paths.len(), "file")),
            "",
            Vec::new(),
        );
        return Ok(());
    };
    if command.trim().is_empty() {
        return Err(CoreError::Message("No command given".into()));
    }
    let arguments: Vec<String> = paths
        .iter()
        .map(|path| crate::shell_quote(&path.to_string_lossy()))
        .collect();
    let directory = view(editor)?.path.clone();
    editor.spawn(Task::Shell {
        command: shell_line(&command, &arguments),
        directory,
        insert_at: None,
    });
    Ok(())
}

/// The command line `!` runs, with dired's own two placeholders.
///
/// A `*` on its own is where every file goes, once; a `?` on its own runs
/// the command once per file with that file in its place. With neither, the
/// files go on the end — which is all this ever did, so `tar czf out.tgz *`
/// archived the files and then tried to add them to the archive again.
fn shell_line(command: &str, arguments: &[String]) -> String {
    let words: Vec<&str> = command.split_whitespace().collect();
    if words.contains(&"*") {
        let all = arguments.join(" ");
        return words
            .iter()
            .map(|word| if *word == "*" { all.as_str() } else { word })
            .collect::<Vec<_>>()
            .join(" ");
    }
    if words.contains(&"?") {
        return arguments
            .iter()
            .map(|file| {
                words
                    .iter()
                    .map(|word| if *word == "?" { file.as_str() } else { word })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("; ");
    }
    format!("{command} {}", arguments.join(" "))
}

fn quit(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = editor.current_buffer_id();
    editor.dired = None;
    editor.kill_buffer(id).ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_shell_placeholders_put_the_files_where_they_are_asked_for() {
        let files = vec!["'a.txt'".to_string(), "'b c.txt'".to_string()];
        assert_eq!(
            super::shell_line("tar czf out.tgz *", &files),
            "tar czf out.tgz 'a.txt' 'b c.txt'"
        );
        assert_eq!(
            super::shell_line("gzip -k ?", &files),
            "gzip -k 'a.txt'; gzip -k 'b c.txt'"
        );
        assert_eq!(
            super::shell_line("wc -l", &files),
            "wc -l 'a.txt' 'b c.txt'"
        );
    }
}
