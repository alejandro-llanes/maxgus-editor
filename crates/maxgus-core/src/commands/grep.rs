//! Project-wide search, and editing the results back into the files.
//!
//! `M-s g` searches; the results are a buffer you read. `C-c C-p` makes that
//! buffer *writable*, and `C-c C-c` writes every line you changed back to the
//! file it came from — which is how a rename across forty files is done here.

use crate::command;
use crate::command::{Args, Registry};
use crate::editor::Editor;
use crate::grep::{GrepView, Row, WrittenFile};
use crate::minibuffer::MinibufferKind;
use crate::task::Task;
use crate::{CoreError, Result};
use maxgus_grep::Replacement;
use maxgus_text::BufferId;
use std::path::Path;

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
pub fn show(
    editor: &mut Editor,
    pattern: &str,
    root: &Path,
    found: maxgus_grep::Found,
) -> Result<()> {
    let view = GrepView::new(pattern, root, found);
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
    // Results that were being edited when these arrived are read again.
    editor.activate_mode_keymap();
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

/// `RET` goes to the line, in the window the results were in unless another
/// is showing the file, and `M-,` comes back. `o` shows it in another window
/// and leaves the cursor in the results, for looking through them a line at
/// a time — which it did only for a file that had to be read: one already
/// open was put in the results' own window.
fn open(editor: &mut Editor, other_window: bool) -> Result<()> {
    let row = row(editor)?;
    let Some(hit) = view(editor)?.hit(&row) else {
        return Err(CoreError::Message("Not a result".into()));
    };
    let (path, line) = (hit.path.clone(), hit.line);
    let results = editor.windows.current_id();
    let Some(id) = editor.buffers.find_by_path(&path) else {
        if other_window {
            editor.pending_return = Some((path.clone(), results));
        } else {
            editor.push_jump();
        }
        editor.pending_line = Some((path.clone(), line));
        editor.spawn(Task::ReadFile {
            path,
            reverting: None,
            other_window,
        });
        return Ok(());
    };
    // A window already showing the file is used for it, whichever key it
    // was: `RET` after `o` goes to the line `o` put beside the results.
    let showing = editor
        .windows
        .showing(id)
        .into_iter()
        .find(|window| *window != results);
    match (showing, other_window) {
        (Some(window), _) => {
            editor.select_window(window);
        }
        (None, true) => editor.select_other_editing_window()?,
        (None, false) => {}
    }
    if !other_window {
        editor.push_jump();
    }
    editor.switch_to_buffer(id)?;
    editor.go_to_line(line);
    if other_window {
        editor.select_window(results);
    }
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
    // Where it ran the first time. From the results buffer, which has no
    // file of its own, the project is only a guess.
    let root = match &editor.grep {
        Some(view) => view.root.clone(),
        None => editor.project_root(),
    };
    editor.message(format!("Searching for `{}`…", search.pattern));
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
///
/// A file open in a buffer with unsaved changes is not written: its lines go
/// into the buffer, beside the changes, for whoever made them to save. The
/// disk under it was written and the buffer read again, which threw those
/// changes away. A buffer with nothing unsaved has its file written like any
/// other, and takes the lines as an edit `C-/` can take back.
///
/// Nothing is changed anywhere until every file has been checked, buffers
/// included.
fn apply(editor: &mut Editor, _: &Args) -> Result<()> {
    if !view(editor)?.editable {
        return Err(CoreError::Message(
            "The results are not being edited: C-c C-p first".into(),
        ));
    }
    let edited = editor.current_buffer().text();
    let replacements = view(editor)?.replacements(&edited)?;
    if replacements.is_empty() {
        return Err(CoreError::Message("Nothing was changed".into()));
    }
    let mut unsaved = Vec::new();
    let mut to_disk = Vec::new();
    for replacement in replacements {
        let Some(id) = editor.buffers.find_by_path(&replacement.path) else {
            to_disk.push(replacement);
            continue;
        };
        let buffer = editor.buffers.get(id).ok_or(CoreError::NoSuchBuffer)?;
        if buffer.is_read_only() {
            return Err(CoreError::Message(format!(
                "{} is read-only, so its lines cannot be changed",
                buffer.name()
            )));
        }
        if !holds(editor, id, std::slice::from_ref(&replacement)) {
            return Err(CoreError::Message(format!(
                "{} has changed since it was searched: C-c C-k, then g to search again",
                buffer.name()
            )));
        }
        match buffer.is_modified() {
            true => unsaved.push(replacement),
            false => to_disk.push(replacement),
        }
    }
    let total = unsaved.len() + to_disk.len();
    editor.spawn(Task::ApplyGrep {
        replacements: to_disk,
        unsaved,
    });
    editor.message(format!("Writing {}…", crate::count(total, "line")));
    Ok(())
}

/// Whether a buffer still has every one of `lines` as it was found.
fn holds(editor: &Editor, id: BufferId, lines: &[Replacement]) -> bool {
    editor.buffers.get(id).is_some_and(|buffer| {
        lines
            .iter()
            .all(|line| line.line < buffer.len_lines() && buffer.line_text(line.line) == line.was)
    })
}

/// Makes `lines` in a buffer, as one step `C-/` takes back.
fn edit_buffer(editor: &mut Editor, id: BufferId, lines: &[Replacement]) -> Result<()> {
    let mut lines = lines.to_vec();
    // From the bottom up, so each line is found before an edit above it
    // could move it.
    lines.sort_by_key(|line| std::cmp::Reverse(line.line));
    editor
        .with_buffer(id, |buffer| {
            buffer.transact(false, |buffer| {
                for line in &lines {
                    let start = buffer.line_start(line.line);
                    let end = start + buffer.line_text(line.line).chars().count();
                    buffer.replace(maxgus_text::Range::new(start, end), &line.now)?;
                }
                Ok::<(), maxgus_text::TextError>(())
            })
        })
        .ok_or(CoreError::NoSuchBuffer)??;
    editor.sync_language_server(id);
    Ok(())
}

/// The files are written, or as many of them as could be, and the buffers
/// are brought along.
pub fn written(
    editor: &mut Editor,
    written: Vec<WrittenFile>,
    failure: Option<String>,
    unsaved: Vec<Replacement>,
) {
    if !written.is_empty() {
        editor.refresh_tree_soon();
    }
    let (mut lines, mut files) = (0, 0);
    let mut made: Vec<Replacement> = Vec::new();
    for file in written {
        lines += file.lines.len();
        files += 1;
        if let Some(id) = editor.buffers.find_by_path(&file.path)
            && editor.buffers.get(id).is_some_and(|b| !b.is_modified())
        {
            match holds(editor, id, &file.lines) && edit_buffer(editor, id, &file.lines).is_ok() {
                // What it holds is what was just written, so it is as saved
                // as it was before.
                true => {
                    if let Some(buffer) = editor.buffers.get_mut(id) {
                        buffer.mark_saved();
                        buffer.set_disk_time(file.disk_time);
                    }
                    editor.notify_saved(id);
                }
                // Opened, or read again, while the file was being written.
                false => editor.spawn(Task::ReadFile {
                    path: file.path.clone(),
                    reverting: Some(id),
                    other_window: false,
                }),
            }
        }
        made.extend(file.lines);
    }
    let done = |lines: usize, files: usize| {
        format!(
            "Wrote {} in {}",
            crate::count(lines, "line"),
            crate::count(files, "file")
        )
    };
    if let Some(failure) = failure {
        // Naming the file as the results do: from the root of the disk, the
        // name was past the end of the echo area in any deep directory.
        let failure = match editor.grep.as_mut() {
            Some(view) => {
                view.applied(&made);
                let root = format!("{}{}", view.root.display(), std::path::MAIN_SEPARATOR);
                failure.replace(&root, "")
            }
            None => failure,
        };
        // Still being edited, so what was not written can be tried again.
        editor.error(match lines {
            0 => format!("Nothing was written: {failure}"),
            _ => format!("{}, then stopped: {failure}", done(lines, files)),
        });
        return;
    }
    let mut kept: Vec<String> = Vec::new();
    let mut moved: Vec<String> = Vec::new();
    let mut groups: Vec<(BufferId, Vec<Replacement>)> = Vec::new();
    for line in unsaved {
        let Some(id) = editor.buffers.find_by_path(&line.path) else {
            moved.push(line.path.display().to_string());
            continue;
        };
        match groups.iter_mut().find(|(group, _)| *group == id) {
            Some((_, group)) => group.push(line),
            None => groups.push((id, vec![line])),
        }
    }
    for (id, group) in groups {
        let name = editor
            .buffers
            .get(id)
            .map(|b| b.name().to_string())
            .unwrap_or_default();
        // Checked when the lines were sent; typed into since, perhaps.
        if !holds(editor, id, &group) || edit_buffer(editor, id, &group).is_err() {
            moved.push(name);
            continue;
        }
        lines += group.len();
        files += 1;
        kept.push(name);
        made.extend(group);
    }
    if let Some(view) = editor.grep.as_mut() {
        view.applied(&made);
    }
    if !moved.is_empty() {
        moved.dedup();
        editor.error(format!(
            "{}, but not into {}, which changed while the rest were written",
            done(lines, files),
            moved.join(", ")
        ));
        return;
    }
    stop_editing(editor);
    match kept.is_empty() {
        true => editor.message(done(lines, files)),
        false => editor.message(format!(
            "{}; {} had unsaved changes, and is left to be saved",
            done(lines, files),
            kept.join(", ")
        )),
    }
}

/// Puts the results back to being read, as the view now has them.
fn stop_editing(editor: &mut Editor) {
    let Some(text) = editor.grep.as_ref().map(GrepView::text) else {
        return;
    };
    if let Some(id) = editor.buffers.find_by_name(GREP_BUFFER_NAME) {
        editor.replace_buffer_contents(id, &text).ok();
        if let Some(buffer) = editor.buffers.get_mut(id) {
            buffer.set_read_only(true);
        }
    }
    if let Some(view) = editor.grep.as_mut() {
        view.editable = false;
    }
    editor.activate_mode_keymap();
}

/// `C-c C-k`: back to reading, with the results as the search left them.
fn abandon(editor: &mut Editor, _: &Args) -> Result<()> {
    if !view(editor)?.editable {
        return Err(CoreError::Message(
            "The results are not being edited".into(),
        ));
    }
    stop_editing(editor);
    editor.message("Edits abandoned".to_string());
    Ok(())
}
