//! Project-wide search, and editing the results back into the files.
//!
//! `M-s g` searches; the results are a buffer you read. `C-c C-p` makes that
//! buffer *writable*, and `C-c C-c` writes every line you changed back to the
//! file it came from — which is how a rename across forty files is done here.

use crate::command;
use crate::command::{Args, Registry};
use crate::editor::Editor;
use crate::grep::{GrepView, Row};
use crate::minibuffer::MinibufferKind;
use crate::task::Task;
use crate::{CoreError, Result};

pub const GREP_BUFFER_NAME: &str = "*grep*";
pub const GREP_MODE: &str = "grep-mode";
/// The same buffer once it is being written into.
///
/// Its own mode because the reading map binds `n`, `p`, `o`, `g` and `q` to
/// commands, and a buffer being typed into needs those keys to be letters.
pub const GREP_EDIT_MODE: &str = "grep-edit-mode";

pub fn register(registry: &mut Registry) {
    registry.register_all(&[
        command!(
            "project-grep",
            "Search every file in the project for a pattern.",
            project_grep
        ),
        command!(
            "project-grep-literal",
            "Search the project for text, taking the pattern as written.",
            project_grep_literal
        ),
        command!(
            "grep-visit",
            "Open the line under point.",
            visit,
            non_interactive
        ),
        command!(
            "grep-visit-other-window",
            "Open the line under point without leaving the results.",
            visit_other_window,
            non_interactive
        ),
        command!(
            "grep-next",
            "Move to the next result.",
            next,
            non_interactive
        ),
        command!(
            "grep-previous",
            "Move to the previous result.",
            previous,
            non_interactive
        ),
        command!("grep-refresh", "Run the search again.", refresh),
        command!("grep-quit", "Close the results.", quit, non_interactive),
        command!(
            "grep-edit",
            "Make the results editable, so the lines can be rewritten.",
            edit
        ),
        command!(
            "grep-apply",
            "Write the edited lines back to their files.",
            apply
        ),
        command!(
            "grep-abandon",
            "Give up the edits and go back to reading.",
            abandon
        ),
    ]);
}

/// The view the results buffer is showing.
fn view(editor: &Editor) -> Result<&GrepView> {
    editor
        .grep
        .as_ref()
        .ok_or_else(|| CoreError::Message("No search results".into()))
}

fn project_grep(editor: &mut Editor, args: &Args) -> Result<()> {
    start(
        editor,
        args,
        true,
        "project-grep",
        "Search project (regexp): ",
    )
}

fn project_grep_literal(editor: &mut Editor, args: &Args) -> Result<()> {
    start(
        editor,
        args,
        false,
        "project-grep-literal",
        "Search project for: ",
    )
}

fn start(
    editor: &mut Editor,
    args: &Args,
    regexp: bool,
    command: &str,
    prompt: &str,
) -> Result<()> {
    let suggestion = editor.word_at_point();
    let Some(pattern) = args.input.clone() else {
        // The word at point is offered as the *default* rather than typed
        // into the prompt: a filled prompt is one a different search has to
        // be cleared out of before it can be typed.
        let prompt = match &suggestion {
            Some(word) => format!("{} (default {word}): ", prompt.trim_end_matches(": ")),
            None => prompt.to_string(),
        };
        editor.grep_default = suggestion;
        editor.prompt_for(command, MinibufferKind::Search, prompt, "", Vec::new());
        return Ok(());
    };
    // An empty answer takes the default, as every prompt here does.
    let pattern = match pattern.trim().is_empty() {
        true => editor
            .grep_default
            .clone()
            .ok_or_else(|| CoreError::Message("Nothing to search for".into()))?,
        false => pattern,
    };
    let mut search = maxgus_grep::Search::new(&pattern);
    search.regexp = regexp;
    search.case_fold = editor.settings.case_fold_search;
    let root = editor.project_root();
    editor.grep_search = Some(search.clone());
    editor.spawn(Task::Grep { root, search });
    editor.message(format!("Searching for `{pattern}`…"));
    Ok(())
}

/// Puts a finished search on screen.
pub fn show(editor: &mut Editor, pattern: &str, found: maxgus_grep::Found) -> Result<()> {
    let view = GrepView::new(pattern, found);
    if view.is_empty() {
        editor.grep = None;
        return Err(CoreError::Message(format!("No matches for `{pattern}`")));
    }
    let hits = view.hits();
    let files = view.files.len();
    let text = view.text();
    let first = view.first_hit_line();
    editor.grep = Some(view);
    let id = match editor.buffers.find_by_name(GREP_BUFFER_NAME) {
        Some(id) => {
            editor.replace_buffer_contents(id, &text).ok();
            id
        }
        None => editor.buffers.create_with_text(GREP_BUFFER_NAME, &text),
    };
    if let Some(buffer) = editor.buffers.get_mut(id) {
        buffer.set_read_only(true);
    }
    editor.show_in_editing_window(id)?;
    editor.move_point_in(id, first);
    editor.message(format!(
        "{} in {}",
        crate::count(hits, "match"),
        crate::count(files, "file")
    ));
    Ok(())
}

/// The row point is on.
fn row(editor: &Editor) -> Result<Row> {
    let view = view(editor)?;
    let line = editor
        .current_buffer()
        .line_of(editor.windows.current().point);
    view.row(line)
        .cloned()
        .ok_or_else(|| CoreError::Message("Nothing here".into()))
}

fn open(editor: &mut Editor, other_window: bool) -> Result<()> {
    let row = row(editor)?;
    let view = view(editor)?;
    let Some(hit) = view.hit(&row) else {
        return Err(CoreError::Message("Not a result".into()));
    };
    let (path, line) = (hit.path.clone(), hit.line);
    if let Some(id) = editor.buffers.find_by_path(&path) {
        if !other_window {
            let target = editor.editing_window();
            if let Some(target) = target {
                editor.select_window(target);
            }
        }
        editor.switch_to_buffer(id)?;
        editor.go_to_line(line);
        return Ok(());
    }
    editor.pending_line = Some((path.clone(), line));
    editor.spawn(Task::ReadFile {
        path,
        reverting: None,
        other_window,
    });
    Ok(())
}

fn visit(editor: &mut Editor, _: &Args) -> Result<()> {
    open(editor, false)
}

fn visit_other_window(editor: &mut Editor, _: &Args) -> Result<()> {
    open(editor, true)
}

fn step(editor: &mut Editor, forward: bool) -> Result<()> {
    let line = editor
        .current_buffer()
        .line_of(editor.windows.current().point);
    let next = view(editor)?
        .step(line, forward)
        .ok_or_else(|| CoreError::Message("No further results".into()))?;
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
    let search = editor
        .grep_search
        .clone()
        .ok_or_else(|| CoreError::Message("No search to run again".into()))?;
    let root = editor.project_root();
    editor.spawn(Task::Grep { root, search });
    Ok(())
}

fn quit(editor: &mut Editor, _: &Args) -> Result<()> {
    let id = editor.current_buffer_id();
    editor.grep = None;
    editor.kill_buffer(id).ok();
    Ok(())
}

// ---- editing the results ------------------------------------------------

/// `C-c C-p`: the results become text that can be typed into.
fn edit(editor: &mut Editor, _: &Args) -> Result<()> {
    view(editor)?;
    let id = editor.current_buffer_id();
    if let Some(buffer) = editor.buffers.get_mut(id) {
        buffer.set_read_only(false);
    }
    if let Some(view) = editor.grep.as_mut() {
        view.editable = true;
    }
    // The navigation keys have to give way to the alphabet.
    editor.activate_mode_keymap();
    editor.message("Editing results: C-c C-c writes them back, C-c C-k gives up".to_string());
    Ok(())
}

/// `C-c C-c`: every changed line is written back to the file it came from.
fn apply(editor: &mut Editor, _: &Args) -> Result<()> {
    if !view(editor)?.editable {
        return Err(CoreError::Message(
            "The results are not being edited: C-c C-p first".into(),
        ));
    }
    let edited = editor.current_buffer().text();
    let replacements = view(editor)?.replacements(&edited);
    if replacements.is_empty() {
        return Err(CoreError::Message("Nothing was changed".into()));
    }
    let count = replacements.len();
    editor.spawn(Task::ApplyGrep { replacements });
    editor.message(format!("Writing {}…", crate::count(count, "line")));
    Ok(())
}

/// `C-c C-k`: back to reading, with the results as the search left them.
fn abandon(editor: &mut Editor, _: &Args) -> Result<()> {
    let text = view(editor)?.text();
    let id = editor.current_buffer_id();
    editor.replace_buffer_contents(id, &text).ok();
    if let Some(buffer) = editor.buffers.get_mut(id) {
        buffer.set_read_only(true);
    }
    if let Some(view) = editor.grep.as_mut() {
        view.editable = false;
    }
    editor.activate_mode_keymap();
    editor.message("Edits abandoned".to_string());
    Ok(())
}
