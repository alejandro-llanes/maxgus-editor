//! The undo-tree visualiser.
//!
//! `C-x u` opens a window on the history beside the buffer. Moving in it
//! moves the buffer: `p` undoes, `n` redoes, `b` takes the other branch, and
//! the text changes under you as you go, which is the whole point — the way
//! to find the version you want is to look at it.

use crate::command;
use crate::command::{Args, Registry};
use crate::editor::Editor;
use crate::undo_tree::{lay_out, line_of};
use crate::{CoreError, Result};
use maxgus_text::BufferId;

pub const VISUALIZER_BUFFER_NAME: &str = "*undo-tree*";
pub const VISUALIZER_MODE: &str = "undo-tree-mode";

pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "undo-tree-visualize",
            "Show the undo history as the tree it is.",
            visualize
        ),
        command!(
            "undo-tree-undo",
            "Step back through the history.",
            undo,
            non_interactive
        ),
        command!(
            "undo-tree-redo",
            "Step forward through the history.",
            redo,
            non_interactive
        ),
        command!(
            "undo-tree-switch-branch",
            "Take the other way forward from here.",
            switch_branch,
            non_interactive
        ),
        command!(
            "undo-tree-quit",
            "Close the visualiser, leaving the buffer where it is.",
            quit,
            non_interactive
        ),
    ]);
}

/// `C-x u`: opens the visualiser on the buffer being edited.
fn visualize(editor: &mut Editor, _: &Args) -> Result<()> {
    let subject = editor.current_buffer_id();
    if editor
        .buffers
        .get(subject)
        .is_some_and(|b| b.name() == VISUALIZER_BUFFER_NAME)
    {
        return Err(CoreError::Message("Already showing the history".into()));
    }
    editor.undo_tree_subject = Some(subject);
    let id = draw(editor)?;
    // Beside the buffer rather than over it: watching the text change as you
    // move is what makes the tree worth having.
    if editor.windows.len() < 2 {
        editor.split_window(crate::window::Direction::Vertical)?;
    }
    let target = editor
        .windows
        .ids()
        .into_iter()
        .find(|w| *w != editor.windows.current_id())
        .unwrap_or_else(|| editor.windows.current_id());
    editor.select_window(target);
    editor.switch_to_buffer(id)?;
    Ok(())
}

/// Redraws the visualiser from the subject's history.
fn draw(editor: &mut Editor) -> Result<BufferId> {
    let subject = editor
        .undo_tree_subject
        .ok_or_else(|| CoreError::Message("No history is being shown".into()))?;
    let (shape, name, position) = {
        let buffer = editor.buffers.get(subject).ok_or(CoreError::NoSuchBuffer)?;
        (
            buffer.undo_shape(),
            buffer.name().to_string(),
            buffer.undo_position(),
        )
    };
    let lines = lay_out(&shape, &name);
    let text: String = lines
        .iter()
        .map(|line| format!("{}\n", line.text))
        .collect();
    let id = match editor.buffers.find_by_name(VISUALIZER_BUFFER_NAME) {
        Some(id) => {
            editor.replace_buffer_contents(id, &text).ok();
            id
        }
        None => editor
            .buffers
            .create_with_text(VISUALIZER_BUFFER_NAME, &text),
    };
    if let Some(buffer) = editor.buffers.get_mut(id) {
        buffer.set_read_only(true);
    }
    if let Some(line) = line_of(&lines, position) {
        editor.move_point_in(id, line);
    }
    Ok(id)
}

/// The buffer whose history is being shown.
fn subject(editor: &Editor) -> Result<BufferId> {
    editor
        .undo_tree_subject
        .filter(|id| editor.buffers.get(*id).is_some())
        .ok_or_else(|| CoreError::Message("No history is being shown".into()))
}

/// Moves the subject, then redraws. The two always go together: a tree that
/// says the buffer is somewhere it is not is worse than no tree.
fn moved(editor: &mut Editor, what: impl Into<String>) -> Result<()> {
    draw(editor)?;
    editor.message(what);
    Ok(())
}

fn undo(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = subject(editor)?;
    let moved_back = editor
        .buffers
        .get_mut(id)
        .ok_or(CoreError::NoSuchBuffer)?
        .undo()?;
    if !moved_back {
        return Err(CoreError::Message("No further undo information".into()));
    }
    editor.forget_highlights(id);
    editor.request_highlighting(id);
    moved(editor, "Undid one change")
}

fn redo(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = subject(editor)?;
    let moved_on = editor
        .buffers
        .get_mut(id)
        .ok_or(CoreError::NoSuchBuffer)?
        .redo()?;
    if !moved_on {
        return Err(CoreError::Message("Nothing to redo".into()));
    }
    editor.forget_highlights(id);
    editor.request_highlighting(id);
    moved(editor, "Redid one change")
}

/// `b`: rotates through the ways forward from where the history is.
fn switch_branch(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = subject(editor)?;
    let buffer = editor.buffers.get_mut(id).ok_or(CoreError::NoSuchBuffer)?;
    let branches = buffer.undo_branches();
    if branches < 2 {
        return Err(CoreError::Message("Only one way forward from here".into()));
    }
    // The last is the one a redo takes, so moving the first to the end steps
    // through them in order and comes back round.
    buffer.set_undo_branch(0);
    moved(editor, format!("Branch 1 of {branches}"))
}

fn quit(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = editor.current_buffer_id();
    let subject = editor.undo_tree_subject;
    editor.undo_tree_subject = None;
    editor.kill_buffer(id).ok();
    // Back to the buffer whose history it was.
    if let Some(subject) = subject
        && editor.buffers.get(subject).is_some()
    {
        editor.switch_to_buffer(subject).ok();
    }
    Ok(())
}
