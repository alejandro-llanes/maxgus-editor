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
            "Delete everything flagged.",
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
    let path = PathBuf::from(input.trim_end_matches('/'));
    editor.spawn(Task::Dired { path });
    Ok(())
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
    editor.show_in_editing_window(id)?;
    editor.move_point_in(id, target);
    Ok(())
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

fn do_flagged(editor: &mut Editor, _: &Args) -> Result<()> {
    let flagged = view(editor)?.with_mark(Mark::Deleted);
    if flagged.is_empty() {
        return Err(CoreError::Message("Nothing is flagged".into()));
    }
    delete(editor, flagged)
}

fn do_delete(editor: &mut Editor, args: &Args) -> Result<()> {
    let paths = targets(editor)?;
    // Deleting is the one thing here that cannot be undone, so it asks —
    // and says exactly what it is about to lose.
    let Some(answer) = args.input.clone() else {
        let what = match paths.len() {
            1 => paths[0].display().to_string(),
            n => format!("{n} items"),
        };
        editor.prompt_for(
            "dired-do-delete",
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
    let to = PathBuf::from(input);
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
    editor.spawn(Task::DiredAct {
        action: FileAction::CreateDirectory(PathBuf::from(input)),
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
            format!("! on {} file(s): ", paths.len()),
            "",
            Vec::new(),
        );
        return Ok(());
    };
    let arguments: Vec<String> = paths
        .iter()
        .map(|path| crate::shell_quote(&path.to_string_lossy()))
        .collect();
    let directory = view(editor)?.path.clone();
    editor.spawn(Task::Shell {
        command: format!("{command} {}", arguments.join(" ")),
        directory,
        insert_at: None,
    });
    Ok(())
}

fn quit(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = editor.current_buffer_id();
    editor.dired = None;
    editor.kill_buffer(id).ok();
    Ok(())
}
